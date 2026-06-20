use sql_orm::prelude::*;
use sql_orm::query::CompiledQuery;
use sql_orm::tiberius::MssqlConnection;
use tokio::sync::{Mutex, MutexGuard};

use std::sync::OnceLock;

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const USERS_TABLE: &str = "dbo.sql_orm_graph_runtime_users";
const POSTS_TABLE: &str = "dbo.sql_orm_graph_runtime_posts";
const OPTIONAL_POSTS_TABLE: &str = "dbo.sql_orm_graph_runtime_optional_posts";

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_graph_runtime_users", schema = "dbo")]
struct GraphUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
    #[orm(has_many(GraphPost, foreign_key = user_id))]
    posts: Collection<GraphPost>,
    #[orm(has_many(OptionalGraphPost, foreign_key = user_id))]
    optional_posts: Collection<OptionalGraphPost>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_graph_runtime_posts", schema = "dbo")]
struct GraphPost {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(foreign_key(entity = GraphUser, column = id))]
    user_id: i64,
    #[orm(length = 120)]
    title: String,
    #[orm(belongs_to(GraphUser, foreign_key = user_id))]
    user: Navigation<GraphUser>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_graph_runtime_optional_posts", schema = "dbo")]
struct OptionalGraphPost {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(nullable)]
    #[orm(foreign_key(entity = GraphUser, column = id))]
    user_id: Option<i64>,
    #[orm(length = 120)]
    title: String,
    #[orm(belongs_to(GraphUser, foreign_key = user_id))]
    user: Navigation<GraphUser>,
}

#[derive(DbContext)]
struct GraphRuntimeDb {
    pub users: DbSet<GraphUser>,
    pub posts: DbSet<GraphPost>,
    pub optional_posts: DbSet<OptionalGraphPost>,
}

#[derive(Debug, PartialEq)]
struct GraphPostRow {
    id: SqlValue,
    user_id: SqlValue,
    title: SqlValue,
}

impl FromRow for GraphPostRow {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            id: row.get_required("id")?,
            user_id: row.get_required("user_id")?,
            title: row.get_required("title")?,
        })
    }
}

#[derive(Debug, PartialEq)]
struct OptionalGraphPostRow {
    id: SqlValue,
    user_id: SqlValue,
    title: SqlValue,
}

impl FromRow for OptionalGraphPostRow {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            id: row.get_required("id")?,
            user_id: row.get_required("user_id")?,
            title: row.get_required("title")?,
        })
    }
}

#[tokio::test]
async fn public_save_changes_persists_graph_dependent_insert_and_fk_move() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping graph persistence runtime integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = graph_runtime_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_graph_tables(&connection_string).await?;

    let result = async {
        let db = GraphRuntimeDb::connect(&connection_string).await?;

        let mut user = db.users.add_tracked(GraphUser {
            id: 0,
            name: "Ana".to_string(),
            posts: Collection::empty(),
            optional_posts: Collection::empty(),
        });
        assert_eq!(db.save_changes().await?, 1);
        assert!(user.id > 0);

        let mut post = db.posts.add_tracked(GraphPost {
            id: 0,
            user_id: user.id,
            title: "Inserted through graph".to_string(),
            user: Navigation::empty(),
        });
        user.posts.push_tracked_related(&post)?;
        assert_eq!(post.state(), EntityState::Added);

        assert_eq!(db.save_changes().await?, 1);
        assert!(post.id > 0);
        assert_eq!(user.state(), EntityState::Unchanged);
        assert_eq!(post.state(), EntityState::Unchanged);
        assert_eq!(post.user_id, user.id);

        let inserted = raw_post_row(&connection_string, post.id)
            .await?
            .expect("dependent inserted through graph should persist");
        assert_eq!(inserted.user_id, SqlValue::I64(user.id));
        assert_eq!(
            inserted.title,
            SqlValue::String("Inserted through graph".to_string())
        );

        let new_user = db.users.add_tracked(GraphUser {
            id: 0,
            name: "Bruno".to_string(),
            posts: Collection::empty(),
            optional_posts: Collection::empty(),
        });
        assert_eq!(db.save_changes().await?, 1);
        assert!(new_user.id > 0);

        post.user.set_tracked_related(Some(&user))?;
        let _ = post.user.take_relationship_change_batch();
        post.user_id = new_user.id;
        post.user.set_tracked_related(Some(&new_user))?;

        assert_eq!(db.save_changes().await?, 1);
        assert_eq!(post.state(), EntityState::Unchanged);
        assert_eq!(post.user_id, new_user.id);

        let moved = raw_post_row(&connection_string, post.id)
            .await?
            .expect("dependent should remain after FK move");
        assert_eq!(moved.user_id, SqlValue::I64(new_user.id));

        Ok(())
    }
    .await;

    cleanup_graph_tables(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_save_changes_persists_optional_relationship_removal_as_null() -> Result<(), OrmError>
{
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping optional graph persistence runtime integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = graph_runtime_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_graph_tables(&connection_string).await?;

    let result = async {
        let db = GraphRuntimeDb::connect(&connection_string).await?;

        let mut user = db.users.add_tracked(GraphUser {
            id: 0,
            name: "Optional owner".to_string(),
            posts: Collection::empty(),
            optional_posts: Collection::empty(),
        });
        assert_eq!(db.save_changes().await?, 1);
        assert!(user.id > 0);

        let mut optional_post = db.optional_posts.add_tracked(OptionalGraphPost {
            id: 0,
            user_id: Some(user.id),
            title: "Optional through graph".to_string(),
            user: Navigation::empty(),
        });

        assert_eq!(db.save_changes().await?, 1);
        assert!(optional_post.id > 0);
        assert_eq!(optional_post.user_id, Some(user.id));
        user.optional_posts.push_tracked_related(&optional_post)?;
        let _ = user.optional_posts.take_relationship_change_batch();

        let removed = user
            .optional_posts
            .remove_related_at(0)
            .expect("optional relationship should be loaded in wrapper");
        assert_eq!(removed.title, optional_post.title);
        let optional_post_id = optional_post.id;
        optional_post.user_id = None;
        drop(optional_post);

        assert_eq!(db.save_changes().await?, 1);

        let removed_row = raw_optional_post_row(&connection_string, optional_post_id)
            .await?
            .expect("optional dependent should remain after relationship removal");
        assert_eq!(removed_row.user_id, SqlValue::Null);

        Ok(())
    }
    .await;

    cleanup_graph_tables(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_save_changes_rejects_required_relationship_removal_before_sql()
-> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping required graph persistence rejection integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = graph_runtime_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_graph_tables(&connection_string).await?;

    let result = async {
        let db = GraphRuntimeDb::connect(&connection_string).await?;

        let mut user = db.users.add_tracked(GraphUser {
            id: 0,
            name: "Required owner".to_string(),
            posts: Collection::empty(),
            optional_posts: Collection::empty(),
        });
        assert_eq!(db.save_changes().await?, 1);
        assert!(user.id > 0);

        let post = db.posts.add_tracked(GraphPost {
            id: 0,
            user_id: user.id,
            title: "Required through graph".to_string(),
            user: Navigation::empty(),
        });

        assert_eq!(db.save_changes().await?, 1);
        assert_eq!(post.user_id, user.id);
        user.posts.push_tracked_related(&post)?;
        let _ = user.posts.take_relationship_change_batch();

        let removed = user
            .posts
            .remove_related_at(0)
            .expect("required relationship should be loaded in wrapper");
        assert_eq!(removed.title, post.title);

        let error = db
            .save_changes()
            .await
            .expect_err("required relationship removal should fail before SQL");
        assert_eq!(error.kind(), sql_orm::core::OrmErrorKind::Compile);
        assert!(
            error.message().contains("required relationship"),
            "unexpected error message: {}",
            error.message()
        );
        assert_eq!(post.state(), EntityState::Unchanged);

        let persisted = raw_post_row(&connection_string, post.id)
            .await?
            .expect("required dependent should remain after rejected removal");
        assert_eq!(persisted.user_id, SqlValue::I64(user.id));

        Ok(())
    }
    .await;

    cleanup_graph_tables(&connection_string, keep_tables).await?;
    result
}

#[tokio::test]
async fn public_save_changes_rejects_conflicting_relationship_assignments_before_sql()
-> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping graph persistence conflict runtime integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let _fixture_guard = graph_runtime_fixture_lock().await;
    let keep_tables = keep_test_tables();
    reset_graph_tables(&connection_string).await?;

    let result = async {
        let db = GraphRuntimeDb::connect(&connection_string).await?;

        let mut first_user = db.users.add_tracked(GraphUser {
            id: 0,
            name: "First principal".to_string(),
            posts: Collection::empty(),
            optional_posts: Collection::empty(),
        });
        let mut second_user = db.users.add_tracked(GraphUser {
            id: 0,
            name: "Second principal".to_string(),
            posts: Collection::empty(),
            optional_posts: Collection::empty(),
        });
        assert_eq!(db.save_changes().await?, 2);
        assert!(first_user.id > 0);
        assert!(second_user.id > 0);
        assert_ne!(first_user.id, second_user.id);

        let post = db.posts.add_tracked(GraphPost {
            id: 0,
            user_id: 0,
            title: "Conflicting dependent".to_string(),
            user: Navigation::empty(),
        });
        first_user.posts.push_tracked_related(&post)?;
        second_user.posts.push_tracked_related(&post)?;

        let error = db
            .save_changes()
            .await
            .expect_err("conflicting relationship assignment should fail before SQL");
        assert_eq!(error.kind(), sql_orm::core::OrmErrorKind::Compile);
        assert!(
            error.message().contains("different values"),
            "unexpected error message: {}",
            error.message()
        );
        assert_eq!(post.state(), EntityState::Added);
        assert_eq!(post.id, 0);
        assert_eq!(db.posts.query().count().await?, 0);

        Ok(())
    }
    .await;

    cleanup_graph_tables(&connection_string, keep_tables).await?;
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

async fn graph_runtime_fixture_lock() -> MutexGuard<'static, ()> {
    static FIXTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    FIXTURE_LOCK.get_or_init(|| Mutex::new(())).lock().await
}

async fn reset_graph_tables(connection_string: &str) -> Result<(), OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;
    drop_graph_tables(&mut connection).await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {USERS_TABLE} (\
                    id BIGINT IDENTITY(1,1) PRIMARY KEY,\
                    name NVARCHAR(120) NOT NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {POSTS_TABLE} (\
                    id BIGINT IDENTITY(1,1) PRIMARY KEY,\
                    user_id BIGINT NOT NULL,\
                    title NVARCHAR(120) NOT NULL,\
                    CONSTRAINT fk_sql_orm_graph_runtime_posts_user \
                        FOREIGN KEY (user_id) REFERENCES {USERS_TABLE}(id)\
                )"
            ),
            vec![],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {OPTIONAL_POSTS_TABLE} (\
                    id BIGINT IDENTITY(1,1) PRIMARY KEY,\
                    user_id BIGINT NULL,\
                    title NVARCHAR(120) NOT NULL,\
                    CONSTRAINT fk_sql_orm_graph_runtime_optional_posts_user \
                        FOREIGN KEY (user_id) REFERENCES {USERS_TABLE}(id)\
                )"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn cleanup_graph_tables(connection_string: &str, keep_tables: bool) -> Result<(), OrmError> {
    if keep_tables {
        return Ok(());
    }

    let mut connection = MssqlConnection::connect(connection_string).await?;
    drop_graph_tables(&mut connection).await
}

async fn drop_graph_tables(connection: &mut MssqlConnection) -> Result<(), OrmError> {
    for table in [OPTIONAL_POSTS_TABLE, POSTS_TABLE, USERS_TABLE] {
        connection
            .execute(CompiledQuery::new(
                format!("IF OBJECT_ID('{table}', 'U') IS NOT NULL DROP TABLE {table}"),
                vec![],
            ))
            .await?;
    }

    Ok(())
}

async fn raw_post_row(connection_string: &str, id: i64) -> Result<Option<GraphPostRow>, OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;
    connection
        .fetch_one::<GraphPostRow>(CompiledQuery::new(
            format!("SELECT [id], [user_id], [title] FROM {POSTS_TABLE} WHERE [id] = @P1"),
            vec![SqlValue::I64(id)],
        ))
        .await
}

async fn raw_optional_post_row(
    connection_string: &str,
    id: i64,
) -> Result<Option<OptionalGraphPostRow>, OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;
    connection
        .fetch_one::<OptionalGraphPostRow>(CompiledQuery::new(
            format!("SELECT [id], [user_id], [title] FROM {OPTIONAL_POSTS_TABLE} WHERE [id] = @P1"),
            vec![SqlValue::I64(id)],
        ))
        .await
}
