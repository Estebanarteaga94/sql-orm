//! Tiberius adapter boundary for execution concerns.

mod config;
mod connection;
mod error;
mod executor;
mod parameter;
#[cfg(feature = "pool-bb8")]
mod pool;
mod row;
mod telemetry;
mod transaction;

use sql_orm_core::CrateIdentity;

/// Placeholder execution adapter marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TiberiusAdapter;

pub use config::{
    MssqlConnectionConfig, MssqlHealthCheckOptions, MssqlHealthCheckQuery, MssqlOperationalOptions,
    MssqlParameterLogMode, MssqlPoolBackend, MssqlPoolOptions, MssqlRetryOptions,
    MssqlSlowQueryOptions, MssqlTimeoutOptions, MssqlTracingOptions,
};
pub use connection::{MssqlConnection, TokioConnectionStream};
pub use executor::{ExecuteResult, Executor};
#[cfg(feature = "pool-bb8")]
pub use pool::{MssqlPool, MssqlPoolBuilder, MssqlPooledConnection};
pub use row::MssqlRow;
pub use transaction::MssqlTransaction;

pub const CRATE_IDENTITY: CrateIdentity = CrateIdentity {
    name: "sql-orm-tiberius",
    responsibility: "connections, execution, rows and transactions over Tiberius",
};

#[cfg(test)]
mod tests {
    use super::{
        CRATE_IDENTITY, ExecuteResult, MssqlConnectionConfig, MssqlHealthCheckOptions,
        MssqlHealthCheckQuery, MssqlOperationalOptions, MssqlPoolOptions, MssqlRetryOptions,
        MssqlRow, MssqlSlowQueryOptions, MssqlTimeoutOptions, MssqlTracingOptions,
        MssqlTransaction, TiberiusAdapter, TokioConnectionStream,
    };
    use std::time::Duration;

    #[test]
    fn declares_execution_boundary() {
        let adapter = TiberiusAdapter;
        assert_eq!(adapter, TiberiusAdapter);
        assert!(CRATE_IDENTITY.responsibility.contains("transactions"));
    }

    #[test]
    fn reexports_connection_config() {
        let config = MssqlConnectionConfig::from_connection_string(
            "server=tcp:localhost,1433;database=master;user=sa;password=Password123;TrustServerCertificate=true",
        )
        .unwrap();

        assert_eq!(config.addr(), "localhost:1433");
    }

    #[test]
    fn reexports_operational_options_surface() {
        let options = MssqlOperationalOptions::new()
            .with_timeouts(MssqlTimeoutOptions::new().with_connect_timeout(Duration::from_secs(5)))
            .with_retry(MssqlRetryOptions::enabled(
                2,
                Duration::from_millis(50),
                Duration::from_secs(1),
            ))
            .with_tracing(MssqlTracingOptions::enabled())
            .with_slow_query(MssqlSlowQueryOptions::enabled(Duration::from_millis(250)))
            .with_health(MssqlHealthCheckOptions::enabled(
                MssqlHealthCheckQuery::SelectOne,
            ))
            .with_pool(MssqlPoolOptions::bb8(8));

        assert_eq!(options.pool.max_size, 8);
        assert!(options.tracing.enabled);
        assert!(options.slow_query.enabled);
        assert!(options.health.enabled);
    }

    #[test]
    fn reexports_execute_result() {
        let result = ExecuteResult::new(vec![1, 2]);

        assert_eq!(result.total(), 3);
    }

    #[test]
    fn reexports_mssql_row_wrapper() {
        let wrapper = core::mem::size_of::<MssqlRow<'static>>();

        assert!(wrapper > 0);
    }

    #[test]
    fn reexports_transaction_wrapper() {
        let wrapper =
            core::mem::size_of::<Option<MssqlTransaction<'static, TokioConnectionStream>>>();

        assert!(wrapper > 0);
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn reexports_pool_surface() {
        let builder = super::MssqlPool::builder();

        assert_eq!(builder.options().backend, super::MssqlPoolBackend::Bb8);
    }
}
