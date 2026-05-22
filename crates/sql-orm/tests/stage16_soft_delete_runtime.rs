use std::sync::{Arc, OnceLock};

use sql_orm::prelude::*;
use sql_orm::query::CompiledQuery;
use sql_orm::tiberius::MssqlConnection;
use tokio::sync::{Mutex, MutexGuard};

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const TEST_TABLE_NAME: &str = "dbo.sql_orm_soft_delete_runtime";
const VERSIONED_TEST_TABLE_NAME: &str = "dbo.sql_orm_soft_delete_runtime_versioned";

struct DeletedAtPolicy;

impl EntityPolicy for DeletedAtPolicy {
    const POLICY_NAME: &'static str = "soft_delete";
    const COLUMN_NAMES: &'static [&'static str] = &["deleted_at"];

    fn columns() -> &'static [ColumnMetadata] {
        const COLUMNS: &[ColumnMetadata] = &[ColumnMetadata {
            rust_field: "deleted_at",
            column_name: "deleted_at",
            renamed_from: None,
            sql_type: SqlServerType::DateTime2,
            nullable: true,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        }];

        COLUMNS
    }
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(
    table = "sql_orm_soft_delete_runtime",
    schema = "dbo",
    soft_delete = DeletedAtPolicy
)]
struct SoftDeleteUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = SoftDeleteUser)]
struct NewSoftDeleteUser {
    name: String,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(
    table = "sql_orm_soft_delete_runtime_versioned",
    schema = "dbo",
    soft_delete = DeletedAtPolicy
)]
struct VersionedSoftDeleteUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
    #[orm(rowversion)]
    version: Vec<u8>,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = VersionedSoftDeleteUser)]
struct NewVersionedSoftDeleteUser {
    name: String,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = VersionedSoftDeleteUser)]
struct UpdateVersionedSoftDeleteUser {
    name: Option<String>,
    version: Option<Vec<u8>>,
}

#[derive(DbContext)]
struct SoftDeleteDb {
    pub users: DbSet<SoftDeleteUser>,
    pub versioned_users: DbSet<VersionedSoftDeleteUser>,
}

struct FixedDeletedAtProvider;

struct DeletedAtRow {
    deleted_at: SqlValue,
}

impl SoftDeleteProvider for FixedDeletedAtProvider {
    fn apply(
        &self,
        context: SoftDeleteContext<'_>,
        changes: &mut Vec<ColumnValue>,
    ) -> Result<(), OrmError> {
        assert_eq!(context.operation, SoftDeleteOperation::Delete);
        changes.push(ColumnValue::new(
            "deleted_at",
            SqlValue::String("2026-04-25T00:00:00".to_string()),
        ));
        Ok(())
    }
}

impl FromRow for DeletedAtRow {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            deleted_at: row.get_required("deleted_at")?,
        })
    }
}

#[tokio::test]
async fn public_dbcontext_soft_delete_provider_routes_delete_through_update() -> Result<(), OrmError>
{
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping soft_delete runtime integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = soft_delete_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = SoftDeleteDb::connect(&connection_string).await?;
        let db = db.with_soft_delete_provider(Arc::new(FixedDeletedAtProvider));

        let inserted = db
            .users
            .insert(NewSoftDeleteUser {
                name: "Soft Delete".to_string(),
            })
            .await?;

        let deleted = db.users.delete(inserted.id).await?;
        assert!(deleted);

        let found = db.users.find(inserted.id).await?;
        assert_eq!(found, None);
        assert_eq!(db.users.query().count().await?, 0);
        assert_eq!(db.users.query().with_deleted().count().await?, 1);
        assert_eq!(
            db.users.query().only_deleted().first().await?,
            Some(inserted.clone())
        );

        let mut connection = MssqlConnection::connect(&connection_string).await?;
        let row = connection
            .fetch_one::<DeletedAtRow>(CompiledQuery::new(
                format!("SELECT [deleted_at] FROM {TEST_TABLE_NAME} WHERE [id] = @P1"),
                vec![SqlValue::I64(inserted.id)],
            ))
            .await?
            .map(|row| row.deleted_at)
            .expect("row should still exist after soft delete");

        assert_ne!(row, SqlValue::Null);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_active_record_soft_delete_routes_delete_through_update() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping soft_delete Active Record integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = soft_delete_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = SoftDeleteDb::connect(&connection_string).await?;
        let db = db.with_soft_delete_provider(Arc::new(FixedDeletedAtProvider));
        let inserted = db
            .users
            .insert(NewSoftDeleteUser {
                name: "Active Record Soft Delete".to_string(),
            })
            .await?;

        let deleted = inserted.delete(&db).await?;
        assert!(deleted);

        assert_eq!(SoftDeleteUser::find(&db, inserted.id).await?, None);
        assert_eq!(SoftDeleteUser::query(&db).count().await?, 0);
        assert_eq!(SoftDeleteUser::query(&db).with_deleted().count().await?, 1);
        assert_ne!(
            raw_deleted_at(&connection_string, TEST_TABLE_NAME, inserted.id).await?,
            SqlValue::Null
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_save_changes_soft_delete_routes_deleted_tracking_through_update()
-> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping soft_delete tracking integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = soft_delete_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = SoftDeleteDb::connect(&connection_string).await?;
        let db = db.with_soft_delete_provider(Arc::new(FixedDeletedAtProvider));
        let inserted = db
            .users
            .insert(NewSoftDeleteUser {
                name: "Tracked Soft Delete".to_string(),
            })
            .await?;
        let mut tracked = db
            .users
            .find_tracked(inserted.id)
            .await?
            .expect("tracked entity should exist");

        db.users.remove_tracked(&mut tracked);
        let saved = db.save_changes().await?;

        assert_eq!(saved, 1);
        assert_eq!(db.users.find(inserted.id).await?, None);
        assert_eq!(db.users.query().with_deleted().count().await?, 1);
        assert_ne!(
            raw_deleted_at(&connection_string, TEST_TABLE_NAME, inserted.id).await?,
            SqlValue::Null
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_soft_delete_with_rowversion_reports_concurrency_conflict_without_deleting()
-> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping soft_delete rowversion integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = soft_delete_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = SoftDeleteDb::connect(&connection_string).await?;
        let db = db.with_soft_delete_provider(Arc::new(FixedDeletedAtProvider));
        let inserted = db
            .versioned_users
            .insert(NewVersionedSoftDeleteUser {
                name: "Versioned Soft Delete".to_string(),
            })
            .await?;
        let stale = inserted.clone();
        let updated = db
            .versioned_users
            .update(
                inserted.id,
                UpdateVersionedSoftDeleteUser {
                    name: Some("Updated Before Delete".to_string()),
                    version: Some(inserted.version.clone()),
                },
            )
            .await?
            .expect("versioned update should return a row");

        let stale_error = stale.delete(&db).await.unwrap_err();
        assert_eq!(stale_error, OrmError::ConcurrencyConflict);
        assert_eq!(
            raw_deleted_at(&connection_string, VERSIONED_TEST_TABLE_NAME, inserted.id).await?,
            SqlValue::Null
        );
        assert!(db.versioned_users.find(inserted.id).await?.is_some());

        let deleted_current = updated.delete(&db).await?;
        assert!(deleted_current);
        assert_eq!(db.versioned_users.find(inserted.id).await?, None);
        assert_ne!(
            raw_deleted_at(&connection_string, VERSIONED_TEST_TABLE_NAME, inserted.id).await?,
            SqlValue::Null
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
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn keep_test_tables() -> bool {
    matches!(
        std::env::var(KEEP_TABLES_ENV)
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

async fn soft_delete_fixture_lock() -> MutexGuard<'static, ()> {
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
                "IF OBJECT_ID('{VERSIONED_TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {VERSIONED_TEST_TABLE_NAME}"
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
                    deleted_at DATETIME2 NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {VERSIONED_TEST_TABLE_NAME} (\
                    id BIGINT IDENTITY(1,1) PRIMARY KEY,\
                    name NVARCHAR(120) NOT NULL,\
                    deleted_at DATETIME2 NULL,\
                    version ROWVERSION NOT NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn raw_deleted_at(
    connection_string: &str,
    table_name: &str,
    id: i64,
) -> Result<SqlValue, OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;
    connection
        .fetch_one::<DeletedAtRow>(CompiledQuery::new(
            format!("SELECT [deleted_at] FROM {table_name} WHERE [id] = @P1"),
            vec![SqlValue::I64(id)],
        ))
        .await?
        .map(|row| row.deleted_at)
        .ok_or_else(|| {
            OrmError::execution("expected soft-deleted row to remain physically present")
        })
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
    connection
        .execute(CompiledQuery::new(
            format!(
                "IF OBJECT_ID('{VERSIONED_TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {VERSIONED_TEST_TABLE_NAME}"
            ),
            vec![],
        ))
        .await?;
    Ok(())
}
