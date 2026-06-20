use sql_orm::prelude::*;
use sql_orm::query::CompiledQuery;
use sql_orm::tiberius::MssqlConnection;
use tokio::sync::{Mutex, MutexGuard};

use std::sync::OnceLock;

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const USERS_TABLE: &str = "dbo.sql_orm_graph_runtime_users";
const POSTS_TABLE: &str = "dbo.sql_orm_graph_runtime_posts";

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

#[derive(DbContext)]
struct GraphRuntimeDb {
    pub users: DbSet<GraphUser>,
    pub posts: DbSet<GraphPost>,
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
        });
        let post = db.posts.add_tracked(GraphPost {
            id: 0,
            user_id: 0,
            title: "Inserted through graph".to_string(),
            user: Navigation::empty(),
        });
        user.posts.push_tracked_related(&post)?;

        assert_eq!(db.save_changes().await?, 2);
        assert!(user.id > 0);
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
        });
        assert_eq!(db.save_changes().await?, 1);
        assert!(new_user.id > 0);

        let mut tracked_post = db
            .posts
            .find_tracked(post.id)
            .await?
            .expect("tracked post should reload for FK move");
        tracked_post.user.set_tracked_related(Some(&new_user))?;

        assert_eq!(db.save_changes().await?, 1);
        assert_eq!(tracked_post.state(), EntityState::Unchanged);
        assert_eq!(tracked_post.user_id, new_user.id);

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
    for table in [POSTS_TABLE, USERS_TABLE] {
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
