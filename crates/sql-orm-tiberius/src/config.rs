use sql_orm_core::OrmError;
use std::fmt;
use std::time::Duration;
use tiberius::Config;

#[derive(Clone)]
pub struct MssqlConnectionConfig {
    connection_string: String,
    inner: Config,
    options: MssqlOperationalOptions,
}

impl fmt::Debug for MssqlConnectionConfig {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MssqlConnectionConfig")
            .field("connection_string", &"<redacted>")
            .field("addr", &self.addr())
            .field("options", &self.options)
            .finish()
    }
}

impl MssqlConnectionConfig {
    pub fn from_connection_string(connection_string: &str) -> Result<Self, OrmError> {
        Self::from_connection_string_with_options(
            connection_string,
            MssqlOperationalOptions::default(),
        )
    }

    pub fn from_connection_string_with_options(
        connection_string: &str,
        options: MssqlOperationalOptions,
    ) -> Result<Self, OrmError> {
        if connection_string.trim().is_empty() {
            return Err(OrmError::connection("invalid SQL Server connection string"));
        }

        let inner = Config::from_ado_string(connection_string)
            .map_err(|_| OrmError::connection("invalid SQL Server connection string"))?;
        validate_config(&inner)?;

        Ok(Self {
            connection_string: connection_string.to_string(),
            inner,
            options,
        })
    }

    pub fn with_options(mut self, options: MssqlOperationalOptions) -> Self {
        self.options = options;
        self
    }

    pub fn connection_string(&self) -> &str {
        &self.connection_string
    }

    pub fn addr(&self) -> String {
        self.inner.get_addr()
    }

    pub fn options(&self) -> &MssqlOperationalOptions {
        &self.options
    }

    pub(crate) fn tiberius_config(&self) -> &Config {
        &self.inner
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MssqlOperationalOptions {
    pub timeouts: MssqlTimeoutOptions,
    pub retry: MssqlRetryOptions,
    pub tracing: MssqlTracingOptions,
    pub slow_query: MssqlSlowQueryOptions,
    pub health: MssqlHealthCheckOptions,
    pub pool: MssqlPoolOptions,
}

impl MssqlOperationalOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_timeouts(mut self, timeouts: MssqlTimeoutOptions) -> Self {
        self.timeouts = timeouts;
        self
    }

    pub fn with_retry(mut self, retry: MssqlRetryOptions) -> Self {
        self.retry = retry;
        self
    }

    pub fn with_tracing(mut self, tracing: MssqlTracingOptions) -> Self {
        self.tracing = tracing;
        self
    }

    pub fn with_slow_query(mut self, slow_query: MssqlSlowQueryOptions) -> Self {
        self.slow_query = slow_query;
        self
    }

    pub fn with_health(mut self, health: MssqlHealthCheckOptions) -> Self {
        self.health = health;
        self
    }

    pub fn with_pool(mut self, pool: MssqlPoolOptions) -> Self {
        self.pool = pool;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct MssqlTimeoutOptions {
    pub connect_timeout: Option<Duration>,
    pub query_timeout: Option<Duration>,
    pub acquire_timeout: Option<Duration>,
}

impl MssqlTimeoutOptions {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_connect_timeout(mut self, timeout: Duration) -> Self {
        self.connect_timeout = Some(timeout);
        self
    }

    pub fn with_query_timeout(mut self, timeout: Duration) -> Self {
        self.query_timeout = Some(timeout);
        self
    }

    pub fn with_acquire_timeout(mut self, timeout: Duration) -> Self {
        self.acquire_timeout = Some(timeout);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MssqlRetryOptions {
    pub enabled: bool,
    pub max_retries: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
}

impl Default for MssqlRetryOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            max_retries: 0,
            base_delay: Duration::from_millis(100),
            max_delay: Duration::from_secs(2),
        }
    }
}

impl MssqlRetryOptions {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn enabled(max_retries: u32, base_delay: Duration, max_delay: Duration) -> Self {
        Self {
            enabled: true,
            max_retries,
            base_delay,
            max_delay,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MssqlTracingOptions {
    pub enabled: bool,
    pub parameter_logging: MssqlParameterLogMode,
    pub emit_start_event: bool,
    pub emit_finish_event: bool,
    pub emit_error_event: bool,
}

impl Default for MssqlTracingOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            parameter_logging: MssqlParameterLogMode::Redacted,
            emit_start_event: true,
            emit_finish_event: true,
            emit_error_event: true,
        }
    }
}

impl MssqlTracingOptions {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn enabled() -> Self {
        Self {
            enabled: true,
            ..Self::default()
        }
    }

    pub fn with_parameter_logging(mut self, parameter_logging: MssqlParameterLogMode) -> Self {
        self.parameter_logging = parameter_logging;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MssqlParameterLogMode {
    Disabled,
    Redacted,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MssqlSlowQueryOptions {
    pub enabled: bool,
    pub threshold: Duration,
    pub parameter_logging: MssqlParameterLogMode,
}

impl Default for MssqlSlowQueryOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            threshold: Duration::from_millis(500),
            parameter_logging: MssqlParameterLogMode::Redacted,
        }
    }
}

impl MssqlSlowQueryOptions {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn enabled(threshold: Duration) -> Self {
        Self {
            enabled: true,
            threshold,
            ..Self::default()
        }
    }

    pub fn with_parameter_logging(mut self, parameter_logging: MssqlParameterLogMode) -> Self {
        self.parameter_logging = parameter_logging;
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MssqlHealthCheckOptions {
    pub enabled: bool,
    pub query: MssqlHealthCheckQuery,
    pub timeout: Option<Duration>,
}

impl Default for MssqlHealthCheckOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            query: MssqlHealthCheckQuery::SelectOne,
            timeout: None,
        }
    }
}

impl MssqlHealthCheckOptions {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn enabled(query: MssqlHealthCheckQuery) -> Self {
        Self {
            enabled: true,
            query,
            timeout: None,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = Some(timeout);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MssqlHealthCheckQuery {
    SelectOne,
}

impl MssqlHealthCheckQuery {
    pub(crate) fn sql(self) -> &'static str {
        match self {
            Self::SelectOne => "SELECT 1 AS [health_check]",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MssqlPoolOptions {
    pub enabled: bool,
    pub backend: MssqlPoolBackend,
    pub max_size: u32,
    pub min_idle: Option<u32>,
    pub acquire_timeout: Option<Duration>,
    pub idle_timeout: Option<Duration>,
    pub max_lifetime: Option<Duration>,
}

impl Default for MssqlPoolOptions {
    fn default() -> Self {
        Self {
            enabled: false,
            backend: MssqlPoolBackend::Bb8,
            max_size: 10,
            min_idle: None,
            acquire_timeout: None,
            idle_timeout: None,
            max_lifetime: None,
        }
    }
}

impl MssqlPoolOptions {
    pub fn disabled() -> Self {
        Self::default()
    }

    pub fn bb8(max_size: u32) -> Self {
        Self {
            enabled: true,
            backend: MssqlPoolBackend::Bb8,
            max_size,
            ..Self::default()
        }
    }

    pub fn with_min_idle(mut self, min_idle: u32) -> Self {
        self.min_idle = Some(min_idle);
        self
    }

    pub fn with_acquire_timeout(mut self, timeout: Duration) -> Self {
        self.acquire_timeout = Some(timeout);
        self
    }

    pub fn with_idle_timeout(mut self, timeout: Duration) -> Self {
        self.idle_timeout = Some(timeout);
        self
    }

    pub fn with_max_lifetime(mut self, timeout: Duration) -> Self {
        self.max_lifetime = Some(timeout);
        self
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MssqlPoolBackend {
    Bb8,
}

fn validate_config(config: &Config) -> Result<(), OrmError> {
    let addr = config.get_addr();

    if addr.is_empty() || addr.starts_with(':') {
        return Err(OrmError::connection("invalid SQL Server connection string"));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        MssqlConnectionConfig, MssqlHealthCheckOptions, MssqlHealthCheckQuery,
        MssqlOperationalOptions, MssqlParameterLogMode, MssqlPoolBackend, MssqlPoolOptions,
        MssqlRetryOptions, MssqlSlowQueryOptions, MssqlTimeoutOptions, MssqlTracingOptions,
    };
    use std::time::Duration;

    #[test]
    fn parses_valid_ado_connection_string() {
        let config = MssqlConnectionConfig::from_connection_string(
            "server=tcp:localhost,1433;database=AppDb;user=sa;password=Password123;TrustServerCertificate=true;Application Name=sql-orm-tests",
        )
        .unwrap();

        assert_eq!(
            config.connection_string(),
            "server=tcp:localhost,1433;database=AppDb;user=sa;password=Password123;TrustServerCertificate=true;Application Name=sql-orm-tests"
        );
        assert_eq!(config.addr(), "localhost:1433");
        assert_eq!(config.options(), &MssqlOperationalOptions::default());
    }

    #[test]
    fn preserves_explicit_operational_options() {
        let options = MssqlOperationalOptions::new()
            .with_timeouts(
                MssqlTimeoutOptions::new()
                    .with_connect_timeout(Duration::from_secs(5))
                    .with_query_timeout(Duration::from_secs(30))
                    .with_acquire_timeout(Duration::from_secs(2)),
            )
            .with_retry(MssqlRetryOptions::enabled(
                3,
                Duration::from_millis(50),
                Duration::from_secs(1),
            ))
            .with_tracing(
                MssqlTracingOptions::enabled()
                    .with_parameter_logging(MssqlParameterLogMode::Disabled),
            )
            .with_slow_query(
                MssqlSlowQueryOptions::enabled(Duration::from_millis(250))
                    .with_parameter_logging(MssqlParameterLogMode::Disabled),
            )
            .with_health(
                MssqlHealthCheckOptions::enabled(MssqlHealthCheckQuery::SelectOne)
                    .with_timeout(Duration::from_secs(3)),
            )
            .with_pool(
                MssqlPoolOptions::bb8(16)
                    .with_min_idle(4)
                    .with_acquire_timeout(Duration::from_secs(2))
                    .with_idle_timeout(Duration::from_secs(30))
                    .with_max_lifetime(Duration::from_secs(300)),
            );

        let config = MssqlConnectionConfig::from_connection_string_with_options(
            "server=tcp:localhost,1433;database=AppDb;user=sa;password=Password123;TrustServerCertificate=true",
            options.clone(),
        )
        .unwrap();

        assert_eq!(config.options(), &options);
        assert_eq!(config.options().pool.backend, MssqlPoolBackend::Bb8);
        assert!(config.options().retry.enabled);
        assert!(config.options().tracing.enabled);
        assert!(config.options().slow_query.enabled);
        assert!(config.options().health.enabled);
        assert!(config.options().pool.enabled);
    }

    #[test]
    fn can_replace_options_on_existing_config() {
        let config = MssqlConnectionConfig::from_connection_string(
            "server=tcp:localhost,1433;database=AppDb;user=sa;password=Password123;TrustServerCertificate=true",
        )
        .unwrap()
        .with_options(MssqlOperationalOptions::new().with_tracing(MssqlTracingOptions::enabled()));

        assert!(config.options().tracing.enabled);
    }

    #[test]
    fn debug_redacts_connection_string() {
        let config = MssqlConnectionConfig::from_connection_string(
            "server=tcp:localhost,1433;database=SecretDb;user=sa;password=Password123;TrustServerCertificate=true;Application Name=secret-app",
        )
        .unwrap();

        let debug = format!("{config:?}");

        assert!(debug.contains("MssqlConnectionConfig"));
        assert!(debug.contains("connection_string: \"<redacted>\""));
        assert!(debug.contains("addr: \"localhost:1433\""));
        assert!(!debug.contains("SecretDb"));
        assert!(!debug.contains("Password123"));
        assert!(!debug.contains("secret-app"));
        assert!(!debug.contains("user=sa"));
    }

    #[test]
    fn rejects_invalid_connection_string() {
        let error = MssqlConnectionConfig::from_connection_string("server=").unwrap_err();

        assert_eq!(error.message(), "invalid SQL Server connection string");
    }

    #[test]
    fn health_check_query_uses_stable_sql() {
        assert_eq!(
            MssqlHealthCheckQuery::SelectOne.sql(),
            "SELECT 1 AS [health_check]"
        );
    }
}
