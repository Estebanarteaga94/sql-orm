use crate::config::MssqlConnectionConfig;
use crate::error::{TiberiusErrorContext, map_tiberius_error};
use crate::executor::fetch_one_compiled;
use crate::telemetry::trace_connection;
use crate::transaction::{
    MssqlTransaction, begin_transaction_scope, commit_transaction_scope, rollback_transaction_scope,
};
use futures_io::{AsyncRead, AsyncWrite};
use sql_orm_core::OrmError;
use sql_orm_query::CompiledQuery;
use std::time::Duration;
use tiberius::Client;
use tokio::net::TcpStream;
use tokio::time::timeout;
use tokio_util::compat::{Compat, TokioAsyncWriteCompatExt};

pub type TokioConnectionStream = Compat<TcpStream>;

pub struct MssqlConnection<S: AsyncRead + AsyncWrite + Unpin + Send = TokioConnectionStream> {
    client: Client<S>,
    config: MssqlConnectionConfig,
}

impl MssqlConnection<TokioConnectionStream> {
    pub async fn connect(connection_string: &str) -> Result<Self, OrmError> {
        let config = MssqlConnectionConfig::from_connection_string(connection_string)?;
        Self::connect_with_config(config).await
    }

    pub async fn connect_with_config(config: MssqlConnectionConfig) -> Result<Self, OrmError> {
        let tracing_options = config.options().tracing;
        let connect_timeout = config.options().timeouts.connect_timeout;
        let addr = config.addr();
        let trace_addr = addr.clone();
        let tiberius_config = config.tiberius_config().clone();

        let client = trace_connection(tracing_options, &trace_addr, connect_timeout, async {
            run_with_timeout(connect_timeout, "SQL Server connection timed out", async {
                let tcp = TcpStream::connect(addr).await.map_err(|error| {
                    map_tiberius_error(&error.into(), TiberiusErrorContext::ConnectTcp)
                })?;
                tcp.set_nodelay(true).map_err(|error| {
                    map_tiberius_error(&error.into(), TiberiusErrorContext::ConfigureTcp)
                })?;

                Client::connect(tiberius_config, tcp.compat_write())
                    .await
                    .map_err(|error| {
                        map_tiberius_error(&error, TiberiusErrorContext::InitializeClient)
                    })
            })
            .await
        })
        .await?;

        Ok(Self { client, config })
    }
}

impl<S: AsyncRead + AsyncWrite + Unpin + Send> MssqlConnection<S> {
    pub fn new(client: Client<S>, config: MssqlConnectionConfig) -> Self {
        Self { client, config }
    }

    pub fn config(&self) -> &MssqlConnectionConfig {
        &self.config
    }

    pub fn client(&self) -> &Client<S> {
        &self.client
    }

    pub fn client_mut(&mut self) -> &mut Client<S> {
        &mut self.client
    }

    pub(crate) fn query_timeout(&self) -> Option<Duration> {
        self.config.options().timeouts.query_timeout
    }

    pub(crate) fn tracing_options(&self) -> crate::config::MssqlTracingOptions {
        self.config.options().tracing
    }

    pub(crate) fn slow_query_options(&self) -> crate::config::MssqlSlowQueryOptions {
        self.config.options().slow_query
    }

    pub(crate) fn retry_options(&self) -> crate::config::MssqlRetryOptions {
        self.config.options().retry
    }

    #[doc(hidden)]
    pub fn replace_retry_options(
        &mut self,
        retry: crate::config::MssqlRetryOptions,
    ) -> crate::config::MssqlRetryOptions {
        let previous = self.config.options().retry;
        let options = self.config.options().clone().with_retry(retry);
        self.config = self.config.clone().with_options(options);
        previous
    }

    pub(crate) fn health_options(&self) -> crate::config::MssqlHealthCheckOptions {
        self.config.options().health
    }

    pub(crate) fn server_addr(&self) -> String {
        self.config.addr()
    }

    pub async fn begin_transaction<'a>(&'a mut self) -> Result<MssqlTransaction<'a, S>, OrmError> {
        let query_timeout = self.query_timeout();
        let tracing_options = self.tracing_options();
        let slow_query_options = self.slow_query_options();
        let server_addr = self.server_addr();
        MssqlTransaction::begin(
            self.client_mut(),
            query_timeout,
            tracing_options,
            slow_query_options,
            server_addr,
        )
        .await
    }

    pub async fn begin_transaction_scope(&mut self) -> Result<(), OrmError> {
        let query_timeout = self.query_timeout();
        let tracing_options = self.tracing_options();
        let server_addr = self.server_addr();
        begin_transaction_scope(
            self.client_mut(),
            query_timeout,
            tracing_options,
            &server_addr,
        )
        .await
    }

    pub async fn commit_transaction(&mut self) -> Result<(), OrmError> {
        let query_timeout = self.query_timeout();
        let tracing_options = self.tracing_options();
        let server_addr = self.server_addr();
        commit_transaction_scope(
            self.client_mut(),
            query_timeout,
            tracing_options,
            &server_addr,
        )
        .await
    }

    pub async fn rollback_transaction(&mut self) -> Result<(), OrmError> {
        let query_timeout = self.query_timeout();
        let tracing_options = self.tracing_options();
        let server_addr = self.server_addr();
        rollback_transaction_scope(
            self.client_mut(),
            query_timeout,
            tracing_options,
            &server_addr,
        )
        .await
    }

    pub async fn health_check(&mut self) -> Result<(), OrmError> {
        let tracing_options = self.tracing_options();
        let slow_query_options = self.slow_query_options();
        let retry_options = self.retry_options();
        let server_addr = self.server_addr();
        let health_options = self.health_options();
        let health_timeout = resolve_health_timeout(health_options, self.query_timeout());
        let query = build_health_check_query(health_options);

        let row = run_with_timeout(health_timeout, "SQL Server health check timed out", async {
            fetch_one_compiled::<_, HealthCheckRow>(
                self.client_mut(),
                query,
                tracing_options,
                slow_query_options,
                retry_options,
                &server_addr,
                health_timeout,
            )
            .await
        })
        .await?;

        match row {
            Some(HealthCheckRow { value: 1 }) => Ok(()),
            Some(_) => Err(OrmError::connection(
                "SQL Server health check returned an unexpected value",
            )),
            None => Err(OrmError::connection(
                "SQL Server health check did not return a row",
            )),
        }
    }

    pub fn into_inner(self) -> Client<S> {
        self.client
    }
}

struct HealthCheckRow {
    value: i32,
}

impl sql_orm_core::FromRow for HealthCheckRow {
    fn from_row<R: sql_orm_core::Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            value: row.get_required_typed::<i32>("health_check")?,
        })
    }
}

fn resolve_health_timeout(
    health_options: crate::config::MssqlHealthCheckOptions,
    query_timeout: Option<Duration>,
) -> Option<Duration> {
    health_options.timeout.or(query_timeout)
}

fn build_health_check_query(
    health_options: crate::config::MssqlHealthCheckOptions,
) -> CompiledQuery {
    CompiledQuery::read_only(health_options.query.sql().to_string(), vec![])
}

pub(crate) async fn run_with_timeout<F, T>(
    duration: Option<Duration>,
    timeout_message: &'static str,
    future: F,
) -> Result<T, OrmError>
where
    F: core::future::Future<Output = Result<T, OrmError>>,
{
    match duration {
        Some(duration) => timeout(duration, future)
            .await
            .map_err(|_| timeout_error(timeout_message))?,
        None => future.await,
    }
}

fn timeout_error(message: &'static str) -> OrmError {
    if message.contains("connection") || message.contains("health check") {
        OrmError::connection(message)
    } else {
        OrmError::execution(message)
    }
}

#[cfg(test)]
mod tests {
    use super::{build_health_check_query, resolve_health_timeout, run_with_timeout};
    use crate::config::{MssqlHealthCheckOptions, MssqlHealthCheckQuery};
    use sql_orm_core::OrmErrorKind;
    use std::time::Duration;

    #[test]
    fn health_check_prefers_explicit_health_timeout_over_query_timeout() {
        let health = MssqlHealthCheckOptions::enabled(MssqlHealthCheckQuery::SelectOne)
            .with_timeout(Duration::from_secs(3));

        assert_eq!(
            resolve_health_timeout(health, Some(Duration::from_secs(30))),
            Some(Duration::from_secs(3))
        );
    }

    #[test]
    fn health_check_falls_back_to_query_timeout_when_no_dedicated_timeout_exists() {
        let health = MssqlHealthCheckOptions::enabled(MssqlHealthCheckQuery::SelectOne);

        assert_eq!(
            resolve_health_timeout(health, Some(Duration::from_secs(30))),
            Some(Duration::from_secs(30))
        );
        assert_eq!(resolve_health_timeout(health, None), None);
    }

    #[test]
    fn health_check_builds_expected_compiled_query() {
        let query = build_health_check_query(MssqlHealthCheckOptions::enabled(
            MssqlHealthCheckQuery::SelectOne,
        ));

        assert_eq!(query.sql, "SELECT 1 AS [health_check]");
        assert!(query.params.is_empty());
    }

    #[tokio::test]
    async fn run_with_timeout_returns_future_result_without_timeout() {
        let value = run_with_timeout(None, "timeout", async {
            Ok::<_, sql_orm_core::OrmError>(7)
        })
        .await
        .unwrap();

        assert_eq!(value, 7);
    }

    #[tokio::test]
    async fn run_with_timeout_fails_when_future_exceeds_deadline() {
        let error = run_with_timeout(
            Some(Duration::from_millis(5)),
            "SQL Server connection timed out",
            async {
                tokio::time::sleep(Duration::from_millis(25)).await;
                Ok::<_, sql_orm_core::OrmError>(())
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.message(), "SQL Server connection timed out");
        assert_eq!(error.kind(), OrmErrorKind::Connection);
    }

    #[tokio::test]
    async fn run_with_timeout_classifies_query_timeout_as_execution() {
        let error = run_with_timeout(
            Some(Duration::from_millis(5)),
            "SQL Server query timed out",
            async {
                tokio::time::sleep(Duration::from_millis(25)).await;
                Ok::<_, sql_orm_core::OrmError>(())
            },
        )
        .await
        .unwrap_err();

        assert_eq!(error.message(), "SQL Server query timed out");
        assert_eq!(error.kind(), OrmErrorKind::Execution);
    }
}
