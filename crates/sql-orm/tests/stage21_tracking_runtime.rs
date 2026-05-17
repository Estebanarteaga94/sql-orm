use sql_orm::prelude::*;
use sql_orm::query::CompiledQuery;
use sql_orm::tiberius::MssqlConnection;
#[cfg(feature = "pool-bb8")]
use sql_orm::{
    MssqlConnectionConfig, MssqlOperationalOptions, MssqlPool, MssqlRetryOptions,
    MssqlTimeoutOptions, MssqlTracingOptions,
};
use std::sync::OnceLock;
#[cfg(feature = "pool-bb8")]
use std::time::Duration;
use tokio::sync::{Mutex, MutexGuard};

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const TEST_TABLE_NAME: &str = "dbo.sql_orm_tracking_runtime";

#[derive(TenantContext)]
struct RuntimeTenant {
    tenant_id: i64,
}

#[derive(AuditFields)]
#[allow(dead_code)]
struct RuntimeAudit {
    #[orm(created_by)]
    #[orm(length = 120)]
    #[orm(updatable = false)]
    created_by: String,

    #[orm(updated_by)]
    #[orm(nullable)]
    #[orm(length = 120)]
    #[orm(insertable = false)]
    updated_by: Option<String>,
}

#[derive(SoftDeleteFields)]
#[allow(dead_code)]
struct RuntimeSoftDelete {
    #[orm(deleted_at)]
    #[orm(sql_type = "datetime2")]
    deleted_at: Option<String>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(
    table = "sql_orm_tracking_runtime",
    schema = "dbo",
    tenant = RuntimeTenant,
    audit = RuntimeAudit,
    soft_delete = RuntimeSoftDelete
)]
struct TrackedPolicyUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
}

#[derive(DbContext)]
struct TrackingRuntimeDb {
    pub users: DbSet<TrackedPolicyUser>,
}

#[derive(Debug, PartialEq)]
struct TrackingRuntimeRow {
    tenant_id: SqlValue,
    name: SqlValue,
    created_by: SqlValue,
    updated_by: SqlValue,
    deleted_at: SqlValue,
}

impl FromRow for TrackingRuntimeRow {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            tenant_id: row.get_required("tenant_id")?,
            name: row.get_required("name")?,
            created_by: row.get_required("created_by")?,
            updated_by: row.get_required("updated_by")?,
            deleted_at: row.get_required("deleted_at")?,
        })
    }
}

#[tokio::test]
async fn public_save_changes_preserves_tenant_and_audit_runtime_values() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping tracking tenant/audit runtime integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = tracking_runtime_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = TrackingRuntimeDb::connect(&connection_string)
            .await?
            .with_tenant(RuntimeTenant { tenant_id: 7 })
            .with_audit_request_values(AuditRequestValues::new(vec![ColumnValue::new(
                "created_by",
                SqlValue::String("creator-7".to_string()),
            )]));

        let mut tracked = db.users.add_tracked(TrackedPolicyUser {
            id: 0,
            name: "tenant audited insert".to_string(),
        });

        assert_eq!(db.save_changes().await?, 1);
        assert!(tracked.id > 0);
        assert_eq!(tracked.state(), EntityState::Unchanged);

        let inserted_row = raw_runtime_row(&connection_string, tracked.id)
            .await?
            .expect("tracked insert should persist a row");
        assert_eq!(inserted_row.tenant_id, SqlValue::I64(7));
        assert_eq!(
            inserted_row.name,
            SqlValue::String("tenant audited insert".to_string())
        );
        assert_eq!(
            inserted_row.created_by,
            SqlValue::String("creator-7".to_string())
        );
        assert_eq!(inserted_row.updated_by, SqlValue::Null);

        let other_tenant = TrackingRuntimeDb::connect(&connection_string)
            .await?
            .with_tenant(RuntimeTenant { tenant_id: 8 });
        assert_eq!(other_tenant.users.find(tracked.id).await?, None);

        let db =
            db.clear_audit_request_values()
                .with_audit_request_values(AuditRequestValues::new(vec![ColumnValue::new(
                    "updated_by",
                    SqlValue::String("updater-7".to_string()),
                )]));
        tracked.name = "tenant audited update".to_string();

        assert_eq!(db.save_changes().await?, 1);
        assert_eq!(tracked.state(), EntityState::Unchanged);

        let updated_row = raw_runtime_row(&connection_string, tracked.id)
            .await?
            .expect("tracked update should keep the row");
        assert_eq!(updated_row.tenant_id, SqlValue::I64(7));
        assert_eq!(
            updated_row.name,
            SqlValue::String("tenant audited update".to_string())
        );
        assert_eq!(
            updated_row.created_by,
            SqlValue::String("creator-7".to_string())
        );
        assert_eq!(
            updated_row.updated_by,
            SqlValue::String("updater-7".to_string())
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

#[cfg(feature = "pool-bb8")]
#[tokio::test]
async fn public_pool_transaction_preserves_runtime_policies_and_tracking() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping pooled tracking policy transaction integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = tracking_runtime_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let options = MssqlOperationalOptions::new()
            .with_timeouts(MssqlTimeoutOptions::new().with_query_timeout(Duration::from_secs(5)))
            .with_tracing(MssqlTracingOptions::enabled())
            .with_retry(MssqlRetryOptions::enabled(
                2,
                Duration::from_millis(10),
                Duration::from_millis(20),
            ));
        let config = MssqlConnectionConfig::from_connection_string_with_options(
            &connection_string,
            options,
        )?;
        let pool = MssqlPool::builder()
            .max_size(1)
            .acquire_timeout(Duration::from_secs(2))
            .connect_with_config(config)
            .await?;
        let db = TrackingRuntimeDb::from_pool(pool)
            .with_tenant(RuntimeTenant { tenant_id: 42 })
            .with_audit_request_values(AuditRequestValues::new(vec![ColumnValue::new(
                "created_by",
                SqlValue::String("pooled-creator".to_string()),
            )]));

        let mut tracked = db.users.add_tracked(TrackedPolicyUser {
            id: 0,
            name: "pooled tracked insert".to_string(),
        });

        assert_eq!(
            db.transaction(|tx| async move { tx.save_changes().await })
                .await?,
            1
        );
        assert!(tracked.id > 0);
        assert_eq!(tracked.state(), EntityState::Unchanged);

        let inserted = raw_runtime_row(&connection_string, tracked.id)
            .await?
            .expect("pooled tracked insert should persist");
        assert_eq!(inserted.tenant_id, SqlValue::I64(42));
        assert_eq!(
            inserted.created_by,
            SqlValue::String("pooled-creator".to_string())
        );

        tracked.name = "pooled tracked update".to_string();
        let db =
            db.clear_audit_request_values()
                .with_audit_request_values(AuditRequestValues::new(vec![ColumnValue::new(
                    "updated_by",
                    SqlValue::String("pooled-updater".to_string()),
                )]));

        assert_eq!(
            db.transaction(|tx| async move { tx.save_changes().await })
                .await?,
            1
        );
        assert_eq!(tracked.state(), EntityState::Unchanged);

        let updated = raw_runtime_row(&connection_string, tracked.id)
            .await?
            .expect("pooled tracked update should persist");
        assert_eq!(
            updated.name,
            SqlValue::String("pooled tracked update".to_string())
        );
        assert_eq!(
            updated.updated_by,
            SqlValue::String("pooled-updater".to_string())
        );

        let db = db
            .clear_audit_request_values()
            .with_soft_delete_request_values(SoftDeleteRequestValues::new(vec![ColumnValue::new(
                "deleted_at",
                SqlValue::String("2026-05-17T00:00:00".to_string()),
            )]));
        db.users.remove_tracked(&mut tracked);

        assert_eq!(
            db.transaction(|tx| async move { tx.save_changes().await })
                .await?,
            1
        );
        assert_eq!(db.users.query().count().await?, 0);

        let deleted = raw_runtime_row(&connection_string, tracked.id)
            .await?
            .expect("soft-deleted row should remain physically present");
        assert_eq!(
            deleted.deleted_at,
            SqlValue::String("2026-05-17T00:00:00".to_string())
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

fn test_connection_string() -> Option<String> {
    std::env::var(TEST_CONNECTION_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty())
}

fn keep_test_tables() -> bool {
    std::env::var(KEEP_TABLES_ENV).is_ok_and(|value| {
        matches!(
            value.trim().to_ascii_lowercase().as_str(),
            "1" | "true" | "yes" | "on"
        )
    })
}

async fn tracking_runtime_fixture_lock() -> MutexGuard<'static, ()> {
    static FIXTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    FIXTURE_LOCK.get_or_init(|| Mutex::new(())).lock().await
}

async fn reset_test_table(connection_string: &str) -> Result<(), OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "IF OBJECT_ID('{TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {TEST_TABLE_NAME}"
            ),
            vec![],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {TEST_TABLE_NAME} (\
                    id BIGINT IDENTITY(1,1) PRIMARY KEY,\
                    name NVARCHAR(120) NOT NULL,\
                    tenant_id BIGINT NOT NULL,\
                    created_by NVARCHAR(120) NOT NULL,\
                    updated_by NVARCHAR(120) NULL,\
                    deleted_at DATETIME2 NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn cleanup_test_table(connection_string: &str, keep_tables: bool) -> Result<(), OrmError> {
    if keep_tables {
        return Ok(());
    }

    let mut connection = MssqlConnection::connect(connection_string).await?;
    connection
        .execute(CompiledQuery::new(
            format!(
                "IF OBJECT_ID('{TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {TEST_TABLE_NAME}"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn raw_runtime_row(
    connection_string: &str,
    id: i64,
) -> Result<Option<TrackingRuntimeRow>, OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;
    connection
        .fetch_one::<TrackingRuntimeRow>(CompiledQuery::new(
            format!(
                "SELECT [tenant_id], [name], [created_by], [updated_by], [deleted_at] FROM {TEST_TABLE_NAME} WHERE [id] = @P1"
            ),
            vec![SqlValue::I64(id)],
        ))
        .await
}
