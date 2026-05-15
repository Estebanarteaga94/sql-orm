use sql_orm::prelude::*;
use sql_orm::query::CompiledQuery;
use sql_orm::tiberius::MssqlConnection;

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const TEST_TABLE_NAME: &str = "dbo.sql_orm_active_record";
const VERSIONED_TEST_TABLE_NAME: &str = "dbo.sql_orm_active_record_versioned";

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_active_record", schema = "dbo")]
struct ActiveRecordUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
    active: bool,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = ActiveRecordUser)]
struct NewActiveRecordUser {
    name: String,
    active: bool,
}

#[derive(DbContext)]
struct ActiveRecordDb {
    pub users: DbSet<ActiveRecordUser>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_active_record_versioned", schema = "dbo")]
struct VersionedActiveRecordUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
    #[orm(rowversion)]
    version: Vec<u8>,
}

#[derive(DbContext)]
struct VersionedActiveRecordDb {
    pub users: DbSet<VersionedActiveRecordUser>,
}

#[tokio::test]
async fn public_active_record_query_roundtrips_against_real_sql_server() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping Active Record query integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = ActiveRecordDb::connect(&connection_string).await?;

        let inserted_ana = db
            .users
            .insert(NewActiveRecordUser {
                name: "Ana".to_string(),
                active: true,
            })
            .await?;
        let inserted_luis = db
            .users
            .insert(NewActiveRecordUser {
                name: "Luis".to_string(),
                active: false,
            })
            .await?;

        let all = ActiveRecordUser::query(&db)
            .order_by(ActiveRecordUser::id.asc())
            .all()
            .await?;
        assert_eq!(all, vec![inserted_ana.clone(), inserted_luis.clone()]);

        let active_only = ActiveRecordUser::query(&db)
            .filter(ActiveRecordUser::active.eq(true))
            .all()
            .await?;
        assert_eq!(active_only, vec![inserted_ana]);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_active_record_find_roundtrips_and_returns_none_when_missing() -> Result<(), OrmError>
{
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping Active Record find integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = ActiveRecordDb::connect(&connection_string).await?;

        let inserted = db
            .users
            .insert(NewActiveRecordUser {
                name: "Maria".to_string(),
                active: true,
            })
            .await?;

        let found = ActiveRecordUser::find(&db, inserted.id).await?;
        assert_eq!(found, Some(inserted));

        let missing = ActiveRecordUser::find(&db, i64::MAX).await?;
        assert_eq!(missing, None);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_active_record_delete_roundtrips_and_returns_false_when_missing()
-> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping Active Record delete integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = ActiveRecordDb::connect(&connection_string).await?;

        let inserted = db
            .users
            .insert(NewActiveRecordUser {
                name: "Delete Me".to_string(),
                active: true,
            })
            .await?;

        let deleted = inserted.delete(&db).await?;
        assert!(deleted);

        let missing_after_delete = ActiveRecordUser::find(&db, inserted.id).await?;
        assert_eq!(missing_after_delete, None);

        let deleted_again = inserted.delete(&db).await?;
        assert!(!deleted_again);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_active_record_save_inserts_and_updates_against_real_sql_server()
-> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping Active Record save integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = ActiveRecordDb::connect(&connection_string).await?;

        let mut user = ActiveRecordUser {
            id: 0,
            name: "Inserted".to_string(),
            active: true,
        };

        user.save(&db).await?;

        assert!(user.id > 0);
        assert_eq!(user.name, "Inserted");
        assert!(user.active);
        assert_eq!(db.users.query().count().await?, 1);

        user.name = "Updated".to_string();
        user.active = false;
        user.save(&db).await?;

        let persisted = ActiveRecordUser::find(&db, user.id).await?;
        assert_eq!(
            persisted,
            Some(ActiveRecordUser {
                id: user.id,
                name: "Updated".to_string(),
                active: false,
            })
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_active_record_respects_rowversion_on_save_and_delete() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping Active Record rowversion integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    reset_versioned_test_table(&connection_string).await?;

    let result = async {
        let db = VersionedActiveRecordDb::connect(&connection_string).await?;

        let mut user = VersionedActiveRecordUser {
            id: 0,
            name: "Inserted".to_string(),
            version: Vec::new(),
        };

        user.save(&db).await?;
        let inserted_version = user.version.clone();
        assert!(!inserted_version.is_empty());

        user.name = "Updated".to_string();
        user.save(&db).await?;
        assert_ne!(user.version, inserted_version);

        let mut stale = user.clone();
        stale.version = inserted_version;
        stale.name = "Stale".to_string();

        let save_error = stale.save(&db).await.unwrap_err();
        assert_eq!(save_error, OrmError::ConcurrencyConflict);

        let delete_error = stale.delete(&db).await.unwrap_err();
        assert_eq!(delete_error, OrmError::ConcurrencyConflict);

        let deleted_current = user.delete(&db).await?;
        assert!(deleted_current);

        Ok(())
    }
    .await;

    cleanup_versioned_test_table(&connection_string, keep_tables).await?;
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
                    active BIT NOT NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn reset_versioned_test_table(connection_string: &str) -> Result<(), OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;

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
                "CREATE TABLE {VERSIONED_TEST_TABLE_NAME} (\
                    id BIGINT IDENTITY(1,1) PRIMARY KEY,\
                    name NVARCHAR(120) NOT NULL,\
                    version ROWVERSION NOT NULL\
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

async fn cleanup_versioned_test_table(
    connection_string: &str,
    keep_tables: bool,
) -> Result<(), OrmError> {
    if keep_tables {
        return Ok(());
    }

    let mut connection = MssqlConnection::connect(connection_string).await?;
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
