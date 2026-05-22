use crate::config::{MssqlRetryOptions, MssqlSlowQueryOptions, MssqlTracingOptions};
use crate::connection::{MssqlConnection, run_with_timeout};
use crate::error::{TiberiusErrorContext, is_transient_tiberius_error, map_tiberius_error};
use crate::parameter::PreparedQuery;
use crate::row::MssqlRow;
use crate::telemetry::{QueryTrace, classify_sql, trace_query};
use crate::transaction::MssqlTransaction;
use async_trait::async_trait;
use futures_io::{AsyncRead, AsyncWrite};
use sql_orm_core::{FromRow, OrmError};
use sql_orm_query::{CompiledQuery, QueryExecution};
use std::time::Duration;
use tiberius::Client;
use tiberius::QueryStream;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecuteResult {
    rows_affected: Vec<u64>,
}

#[derive(Clone, Copy)]
pub(crate) struct QueryExecutionOptions<'a> {
    pub(crate) tracing: MssqlTracingOptions,
    pub(crate) slow_query: MssqlSlowQueryOptions,
    pub(crate) retry: MssqlRetryOptions,
    pub(crate) server_addr: &'a str,
    pub(crate) timeout: Option<Duration>,
}

impl ExecuteResult {
    pub fn new(rows_affected: Vec<u64>) -> Self {
        Self { rows_affected }
    }

    pub fn rows_affected(&self) -> &[u64] {
        &self.rows_affected
    }

    pub fn total(&self) -> u64 {
        self.rows_affected.iter().sum()
    }
}

#[async_trait]
pub trait Executor {
    async fn execute(&mut self, query: CompiledQuery) -> Result<ExecuteResult, OrmError>;
    async fn fetch_one<T>(&mut self, query: CompiledQuery) -> Result<Option<T>, OrmError>
    where
        T: FromRow + Send;
    async fn fetch_all<T>(&mut self, query: CompiledQuery) -> Result<Vec<T>, OrmError>
    where
        T: FromRow + Send;
}

#[async_trait]
impl<S> Executor for MssqlConnection<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    async fn execute(&mut self, query: CompiledQuery) -> Result<ExecuteResult, OrmError> {
        MssqlConnection::execute(self, query).await
    }

    async fn fetch_one<T>(&mut self, query: CompiledQuery) -> Result<Option<T>, OrmError>
    where
        T: FromRow + Send,
    {
        MssqlConnection::fetch_one(self, query).await
    }

    async fn fetch_all<T>(&mut self, query: CompiledQuery) -> Result<Vec<T>, OrmError>
    where
        T: FromRow + Send,
    {
        MssqlConnection::fetch_all(self, query).await
    }
}

#[async_trait]
impl<S> Executor for MssqlTransaction<'_, S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    async fn execute(&mut self, query: CompiledQuery) -> Result<ExecuteResult, OrmError> {
        MssqlTransaction::execute(self, query).await
    }

    async fn fetch_one<T>(&mut self, query: CompiledQuery) -> Result<Option<T>, OrmError>
    where
        T: FromRow + Send,
    {
        MssqlTransaction::fetch_one(self, query).await
    }

    async fn fetch_all<T>(&mut self, query: CompiledQuery) -> Result<Vec<T>, OrmError>
    where
        T: FromRow + Send,
    {
        MssqlTransaction::fetch_all(self, query).await
    }
}

impl<S> MssqlConnection<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    pub async fn execute(&mut self, query: CompiledQuery) -> Result<ExecuteResult, OrmError> {
        let tracing_options = self.tracing_options();
        let slow_query_options = self.slow_query_options();
        let server_addr = self.server_addr();
        let query_timeout = self.query_timeout();
        run_with_timeout(self.query_timeout(), "SQL Server query timed out", async {
            execute_compiled(
                self.client_mut(),
                query,
                tracing_options,
                slow_query_options,
                &server_addr,
                query_timeout,
            )
            .await
        })
        .await
    }

    pub async fn query_raw<'a>(
        &'a mut self,
        query: CompiledQuery,
    ) -> Result<QueryStream<'a>, OrmError> {
        let tracing_options = self.tracing_options();
        let slow_query_options = self.slow_query_options();
        let server_addr = self.server_addr();
        let query_timeout = self.query_timeout();
        run_with_timeout(query_timeout, "SQL Server query timed out", async {
            query_raw_compiled(
                self.client_mut(),
                query,
                tracing_options,
                slow_query_options,
                &server_addr,
                query_timeout,
            )
            .await
        })
        .await
    }

    pub async fn fetch_one<T>(&mut self, query: CompiledQuery) -> Result<Option<T>, OrmError>
    where
        T: FromRow + Send,
    {
        let tracing_options = self.tracing_options();
        let slow_query_options = self.slow_query_options();
        let retry_options = self.retry_options();
        let server_addr = self.server_addr();
        let query_timeout = self.query_timeout();
        run_with_timeout(self.query_timeout(), "SQL Server query timed out", async {
            fetch_one_compiled(
                self.client_mut(),
                query,
                QueryExecutionOptions {
                    tracing: tracing_options,
                    slow_query: slow_query_options,
                    retry: retry_options,
                    server_addr: &server_addr,
                    timeout: query_timeout,
                },
            )
            .await
        })
        .await
    }

    pub async fn fetch_all<T>(&mut self, query: CompiledQuery) -> Result<Vec<T>, OrmError>
    where
        T: FromRow + Send,
    {
        let tracing_options = self.tracing_options();
        let slow_query_options = self.slow_query_options();
        let retry_options = self.retry_options();
        let server_addr = self.server_addr();
        let query_timeout = self.query_timeout();
        run_with_timeout(self.query_timeout(), "SQL Server query timed out", async {
            fetch_all_compiled(
                self.client_mut(),
                query,
                QueryExecutionOptions {
                    tracing: tracing_options,
                    slow_query: slow_query_options,
                    retry: retry_options,
                    server_addr: &server_addr,
                    timeout: query_timeout,
                },
            )
            .await
        })
        .await
    }

    pub async fn fetch_one_with<T, F>(
        &mut self,
        query: CompiledQuery,
        map: F,
    ) -> Result<Option<T>, OrmError>
    where
        F: FnMut(MssqlRow<'_>) -> Result<T, OrmError> + Send,
    {
        let tracing_options = self.tracing_options();
        let slow_query_options = self.slow_query_options();
        let retry_options = self.retry_options();
        let server_addr = self.server_addr();
        let query_timeout = self.query_timeout();
        run_with_timeout(self.query_timeout(), "SQL Server query timed out", async {
            fetch_one_compiled_with(
                self.client_mut(),
                query,
                QueryExecutionOptions {
                    tracing: tracing_options,
                    slow_query: slow_query_options,
                    retry: retry_options,
                    server_addr: &server_addr,
                    timeout: query_timeout,
                },
                map,
            )
            .await
        })
        .await
    }

    pub async fn fetch_all_with<T, F>(
        &mut self,
        query: CompiledQuery,
        map: F,
    ) -> Result<Vec<T>, OrmError>
    where
        F: FnMut(MssqlRow<'_>) -> Result<T, OrmError> + Send,
    {
        let tracing_options = self.tracing_options();
        let slow_query_options = self.slow_query_options();
        let retry_options = self.retry_options();
        let server_addr = self.server_addr();
        let query_timeout = self.query_timeout();
        run_with_timeout(self.query_timeout(), "SQL Server query timed out", async {
            fetch_all_compiled_with(
                self.client_mut(),
                query,
                QueryExecutionOptions {
                    tracing: tracing_options,
                    slow_query: slow_query_options,
                    retry: retry_options,
                    server_addr: &server_addr,
                    timeout: query_timeout,
                },
                map,
            )
            .await
        })
        .await
    }
}

pub(crate) async fn execute_compiled<S>(
    client: &mut Client<S>,
    query: CompiledQuery,
    tracing_options: MssqlTracingOptions,
    slow_query_options: MssqlSlowQueryOptions,
    server_addr: &str,
    query_timeout: Option<std::time::Duration>,
) -> Result<ExecuteResult, OrmError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let prepared = PreparedQuery::from_compiled(query);
    let trace = QueryTrace::new(server_addr, query_timeout, tracing_options, &prepared);
    let result = trace_query(tracing_options, slow_query_options, trace, async {
        prepared.validate_parameter_count()?;
        prepared.execute(client).await
    })
    .await?;

    Ok(ExecuteResult::new(result.rows_affected().to_vec()))
}

pub(crate) async fn query_raw_compiled<'a, S>(
    client: &'a mut Client<S>,
    query: CompiledQuery,
    tracing_options: MssqlTracingOptions,
    slow_query_options: MssqlSlowQueryOptions,
    server_addr: &str,
    query_timeout: Option<std::time::Duration>,
) -> Result<QueryStream<'a>, OrmError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
{
    let prepared = PreparedQuery::from_compiled(query);
    let trace = QueryTrace::new(server_addr, query_timeout, tracing_options, &prepared);
    trace_query(tracing_options, slow_query_options, trace, async {
        prepared.validate_parameter_count()?;
        prepared.query(client).await
    })
    .await
}

pub(crate) async fn fetch_one_compiled<S, T>(
    client: &mut Client<S>,
    query: CompiledQuery,
    options: QueryExecutionOptions<'_>,
) -> Result<Option<T>, OrmError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    T: FromRow + Send,
{
    let retryable_query = is_retryable_read_query(&query, options.retry);
    let mut attempt = 0;

    let row = loop {
        let prepared = PreparedQuery::from_compiled(query.clone());
        prepared.validate_parameter_count()?;
        let trace = QueryTrace::new(
            options.server_addr,
            options.timeout,
            options.tracing,
            &prepared,
        );

        match trace_query(options.tracing, options.slow_query, trace, async {
            prepared.query_driver(client).await?.into_row().await
        })
        .await
        {
            Ok(row) => break row,
            Err(error)
                if retryable_query
                    && attempt < options.retry.max_retries
                    && is_transient_tiberius_error(&error) =>
            {
                attempt += 1;
                let delay = retry_delay(options.retry, attempt);

                tracing::warn!(
                    target: "orm.query.retry",
                    server_addr = %options.server_addr,
                    operation = %classify_sql(&query.sql),
                    attempt,
                    max_retries = options.retry.max_retries,
                    delay_ms = delay.as_millis(),
                    error_code = ?error.code(),
                    error = %error,
                );

                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                return Err(map_tiberius_error(
                    &error,
                    TiberiusErrorContext::ExecuteQuery,
                ));
            }
        }
    };

    row.as_ref()
        .map(|row| T::from_row(&MssqlRow::new(row)))
        .transpose()
}

async fn fetch_one_compiled_with<S, T, F>(
    client: &mut Client<S>,
    query: CompiledQuery,
    options: QueryExecutionOptions<'_>,
    mut map: F,
) -> Result<Option<T>, OrmError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    F: FnMut(MssqlRow<'_>) -> Result<T, OrmError> + Send,
{
    let retryable_query = is_retryable_read_query(&query, options.retry);
    let mut attempt = 0;

    let row = loop {
        let prepared = PreparedQuery::from_compiled(query.clone());
        prepared.validate_parameter_count()?;
        let trace = QueryTrace::new(
            options.server_addr,
            options.timeout,
            options.tracing,
            &prepared,
        );

        match trace_query(options.tracing, options.slow_query, trace, async {
            prepared.query_driver(client).await?.into_row().await
        })
        .await
        {
            Ok(row) => break row,
            Err(error)
                if retryable_query
                    && attempt < options.retry.max_retries
                    && is_transient_tiberius_error(&error) =>
            {
                attempt += 1;
                let delay = retry_delay(options.retry, attempt);

                tracing::warn!(
                    target: "orm.query.retry",
                    server_addr = %options.server_addr,
                    operation = %classify_sql(&query.sql),
                    attempt,
                    max_retries = options.retry.max_retries,
                    delay_ms = delay.as_millis(),
                    error_code = ?error.code(),
                    error = %error,
                );

                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                return Err(map_tiberius_error(
                    &error,
                    TiberiusErrorContext::ExecuteQuery,
                ));
            }
        }
    };

    row.as_ref().map(|row| map(MssqlRow::new(row))).transpose()
}

pub(crate) async fn fetch_all_compiled<S, T>(
    client: &mut Client<S>,
    query: CompiledQuery,
    options: QueryExecutionOptions<'_>,
) -> Result<Vec<T>, OrmError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    T: FromRow + Send,
{
    let retryable_query = is_retryable_read_query(&query, options.retry);
    let mut attempt = 0;

    let rows = loop {
        let prepared = PreparedQuery::from_compiled(query.clone());
        prepared.validate_parameter_count()?;
        let trace = QueryTrace::new(
            options.server_addr,
            options.timeout,
            options.tracing,
            &prepared,
        );

        match trace_query(options.tracing, options.slow_query, trace, async {
            prepared
                .query_driver(client)
                .await?
                .into_first_result()
                .await
        })
        .await
        {
            Ok(rows) => break rows,
            Err(error)
                if retryable_query
                    && attempt < options.retry.max_retries
                    && is_transient_tiberius_error(&error) =>
            {
                attempt += 1;
                let delay = retry_delay(options.retry, attempt);

                tracing::warn!(
                    target: "orm.query.retry",
                    server_addr = %options.server_addr,
                    operation = %classify_sql(&query.sql),
                    attempt,
                    max_retries = options.retry.max_retries,
                    delay_ms = delay.as_millis(),
                    error_code = ?error.code(),
                    error = %error,
                );

                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                return Err(map_tiberius_error(
                    &error,
                    TiberiusErrorContext::ExecuteQuery,
                ));
            }
        }
    };

    rows.iter()
        .map(|row| T::from_row(&MssqlRow::new(row)))
        .collect()
}

async fn fetch_all_compiled_with<S, T, F>(
    client: &mut Client<S>,
    query: CompiledQuery,
    options: QueryExecutionOptions<'_>,
    mut map: F,
) -> Result<Vec<T>, OrmError>
where
    S: AsyncRead + AsyncWrite + Unpin + Send,
    F: FnMut(MssqlRow<'_>) -> Result<T, OrmError> + Send,
{
    let retryable_query = is_retryable_read_query(&query, options.retry);
    let mut attempt = 0;

    let rows = loop {
        let prepared = PreparedQuery::from_compiled(query.clone());
        prepared.validate_parameter_count()?;
        let trace = QueryTrace::new(
            options.server_addr,
            options.timeout,
            options.tracing,
            &prepared,
        );

        match trace_query(options.tracing, options.slow_query, trace, async {
            prepared
                .query_driver(client)
                .await?
                .into_first_result()
                .await
        })
        .await
        {
            Ok(rows) => break rows,
            Err(error)
                if retryable_query
                    && attempt < options.retry.max_retries
                    && is_transient_tiberius_error(&error) =>
            {
                attempt += 1;
                let delay = retry_delay(options.retry, attempt);

                tracing::warn!(
                    target: "orm.query.retry",
                    server_addr = %options.server_addr,
                    operation = %classify_sql(&query.sql),
                    attempt,
                    max_retries = options.retry.max_retries,
                    delay_ms = delay.as_millis(),
                    error_code = ?error.code(),
                    error = %error,
                );

                tokio::time::sleep(delay).await;
            }
            Err(error) => {
                return Err(map_tiberius_error(
                    &error,
                    TiberiusErrorContext::ExecuteQuery,
                ));
            }
        }
    };

    rows.iter().map(|row| map(MssqlRow::new(row))).collect()
}

fn is_retryable_read_query(query: &CompiledQuery, retry_options: MssqlRetryOptions) -> bool {
    retry_options.enabled
        && retry_options.max_retries > 0
        && query.execution == QueryExecution::ReadOnly
}

fn retry_delay(retry_options: MssqlRetryOptions, attempt: u32) -> Duration {
    let multiplier = 1u32
        .checked_shl(attempt.saturating_sub(1))
        .unwrap_or(u32::MAX);
    let base_millis = retry_options.base_delay.as_millis();
    let max_millis = retry_options.max_delay.as_millis();
    let scaled = base_millis.saturating_mul(u128::from(multiplier));

    Duration::from_millis(scaled.min(max_millis) as u64)
}

#[cfg(test)]
mod tests {
    use super::{
        ExecuteResult, fetch_all_compiled, fetch_one_compiled, is_retryable_read_query,
        query_raw_compiled, retry_delay,
    };
    use crate::config::{MssqlSlowQueryOptions, MssqlTracingOptions};
    use sql_orm_core::{FromRow, OrmError, Row};
    use sql_orm_query::{CompiledQuery, QueryExecution};
    use std::time::Duration;

    struct TestRowModel;

    impl FromRow for TestRowModel {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self)
        }
    }

    #[test]
    fn execute_result_exposes_rows_affected_and_total() {
        let result = ExecuteResult::new(vec![1, 2, 3]);

        assert_eq!(result.rows_affected(), &[1, 2, 3]);
        assert_eq!(result.total(), 6);
    }

    #[test]
    fn reuses_shared_execution_helpers_from_transaction_boundary() {
        let query_raw = query_raw_compiled::<tokio_util::compat::Compat<tokio::net::TcpStream>>;
        let fetch_one =
            fetch_one_compiled::<tokio_util::compat::Compat<tokio::net::TcpStream>, TestRowModel>;
        let fetch_all =
            fetch_all_compiled::<tokio_util::compat::Compat<tokio::net::TcpStream>, TestRowModel>;

        let _ = (query_raw, fetch_one, fetch_all);
    }

    #[test]
    fn compiled_query_helpers_accept_tracing_context_shape() {
        let tracing = MssqlTracingOptions::enabled();
        let slow_query = MssqlSlowQueryOptions::enabled(std::time::Duration::from_millis(250));

        assert!(tracing.enabled);
        assert!(slow_query.enabled);
    }

    #[test]
    fn retry_policy_only_targets_explicit_read_only_queries() {
        let retry = crate::config::MssqlRetryOptions::enabled(
            2,
            Duration::from_millis(50),
            Duration::from_secs(1),
        );

        assert!(is_retryable_read_query(
            &CompiledQuery::read_only("EXEC dbo.read_only_proc", vec![]),
            retry
        ));
        assert!(!is_retryable_read_query(
            &CompiledQuery::write(
                "SELECT * INTO [dbo].[users_copy] FROM [dbo].[users]",
                vec![]
            ),
            retry
        ));
        assert!(!is_retryable_read_query(
            &CompiledQuery::with_execution("SELECT 1", vec![], QueryExecution::RawNoRetry),
            retry
        ));
    }

    #[test]
    fn retry_delay_caps_at_max_delay() {
        let retry = crate::config::MssqlRetryOptions::enabled(
            4,
            Duration::from_millis(100),
            Duration::from_millis(250),
        );

        assert_eq!(retry_delay(retry, 1), Duration::from_millis(100));
        assert_eq!(retry_delay(retry, 2), Duration::from_millis(200));
        assert_eq!(retry_delay(retry, 3), Duration::from_millis(250));
    }
}
