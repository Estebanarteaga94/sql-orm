use sql_orm::prelude::*;
use sql_orm::query::{CompiledQuery, Expr, Predicate};
use sql_orm::tiberius::MssqlConnection;

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const USERS_TABLE: &str = "dbo.sql_orm_nav_users";
const PROFILES_TABLE: &str = "dbo.sql_orm_nav_profiles";
const POSTS_TABLE: &str = "dbo.sql_orm_nav_posts";

#[derive(Entity, Debug, Clone)]
#[orm(table = "sql_orm_nav_users", schema = "dbo")]
struct RuntimeUser {
    #[orm(primary_key)]
    id: i64,
    #[orm(length = 120)]
    name: String,
    #[orm(has_one(RuntimeProfile, foreign_key = user_id))]
    profile: Navigation<RuntimeProfile>,
    #[orm(has_many(RuntimePost, foreign_key = user_id))]
    posts: Collection<RuntimePost>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "sql_orm_nav_profiles", schema = "dbo")]
struct RuntimeProfile {
    #[orm(primary_key)]
    id: i64,
    #[orm(foreign_key(entity = RuntimeUser, column = id))]
    user_id: i64,
    #[orm(length = 120)]
    bio: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "sql_orm_nav_posts", schema = "dbo")]
struct RuntimePost {
    #[orm(primary_key)]
    id: i64,
    #[orm(foreign_key(entity = RuntimeUser, column = id))]
    user_id: i64,
    #[orm(length = 120)]
    title: String,
    #[orm(belongs_to(RuntimeUser, foreign_key = user_id))]
    user: Navigation<RuntimeUser>,
}

#[derive(DbContext)]
struct RuntimeNavigationDb {
    pub users: DbSet<RuntimeUser>,
    pub profiles: DbSet<RuntimeProfile>,
    pub posts: DbSet<RuntimePost>,
}

#[tokio::test]
async fn navigation_runtime_loads_graph_shapes_against_real_sql_server() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping navigation runtime integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    reset_navigation_tables(&connection_string).await?;
    seed_navigation_rows(&connection_string).await?;
    announce_test_tables(keep_tables);

    let result = async {
        let db = RuntimeNavigationDb::connect(&connection_string).await?;

        let user_with_profile = db
            .users
            .query()
            .include_as::<RuntimeProfile>("profile", "profile")?
            .filter(Predicate::eq(
                Expr::from(RuntimeUser::id),
                Expr::value(SqlValue::I64(1)),
            ))
            .first()
            .await?
            .expect("user with has_one profile");

        let profile = user_with_profile
            .profile
            .as_ref()
            .expect("has_one profile should be loaded");
        assert_eq!(profile.id, 10);
        assert_eq!(profile.user_id, 1);
        assert_eq!(profile.bio, "Profile Ana");

        let user_without_profile = db
            .users
            .query()
            .include_as::<RuntimeProfile>("profile", "profile")?
            .filter(Predicate::eq(
                Expr::from(RuntimeUser::id),
                Expr::value(SqlValue::I64(2)),
            ))
            .first()
            .await?
            .expect("user without has_one profile should still be returned");
        assert!(
            user_without_profile.profile.as_ref().is_none(),
            "missing has_one rows must leave the navigation empty without dropping the root"
        );

        let post_with_user = db
            .posts
            .query()
            .include_as::<RuntimeUser>("user", "user")?
            .filter(Predicate::eq(
                Expr::from(RuntimePost::id),
                Expr::value(SqlValue::I64(100)),
            ))
            .first()
            .await?
            .expect("post with belongs_to user");

        let post_user = post_with_user
            .user
            .as_ref()
            .expect("belongs_to user should be loaded");
        assert_eq!(post_user.id, 1);
        assert_eq!(post_user.name, "Ana");

        let users_with_posts = db
            .users
            .query()
            .include_many_as::<RuntimePost>("posts", "posts")?
            .filter(Predicate::eq(
                Expr::from(RuntimeUser::id),
                Expr::value(SqlValue::I64(1)),
            ))
            .all()
            .await?;
        assert_eq!(users_with_posts.len(), 1);

        let mut included_titles = users_with_posts[0]
            .posts
            .as_slice()
            .iter()
            .map(|post| post.title.as_str())
            .collect::<Vec<_>>();
        included_titles.sort_unstable();
        assert_eq!(included_titles, vec!["First post", "Second post"]);

        let users_with_all_posts = db
            .users
            .query()
            .include_many_as::<RuntimePost>("posts", "posts")?
            .order_by(RuntimeUser::id.asc())
            .all()
            .await?;
        assert_eq!(
            users_with_all_posts
                .iter()
                .map(|user| user.id)
                .collect::<Vec<_>>(),
            vec![1, 2],
            "include_many must group joined rows without duplicating roots"
        );

        let mut first_user_titles = users_with_all_posts[0]
            .posts
            .as_slice()
            .iter()
            .map(|post| post.title.as_str())
            .collect::<Vec<_>>();
        first_user_titles.sort_unstable();
        assert_eq!(first_user_titles, vec!["First post", "Second post"]);
        assert_eq!(users_with_all_posts[1].posts.as_slice().len(), 1);
        assert_eq!(
            users_with_all_posts[1].posts.as_slice()[0].title,
            "Other post"
        );

        let mut loaded_user = db.users.find(1_i64).await?.expect("materialized user");
        assert!(loaded_user.posts.as_slice().is_empty());
        db.users
            .load_collection::<RuntimePost>(&mut loaded_user, "posts")
            .await?;

        let mut explicit_titles = loaded_user
            .posts
            .as_slice()
            .iter()
            .map(|post| post.title.as_str())
            .collect::<Vec<_>>();
        explicit_titles.sort_unstable();
        assert_eq!(explicit_titles, vec!["First post", "Second post"]);

        let mut tracked_user = db.users.find_tracked(1_i64).await?.expect("tracked user");
        assert_eq!(tracked_user.state(), EntityState::Unchanged);
        db.users
            .load_collection_tracked::<RuntimePost>(&mut tracked_user, "posts")
            .await?;
        assert_eq!(tracked_user.state(), EntityState::Unchanged);
        assert_eq!(tracked_user.current().posts.as_slice().len(), 2);

        let bounded_nested_post = db
            .posts
            .query()
            .include_as::<RuntimeUser>("user", "user")?
            .filter(Predicate::eq(
                Expr::from(RuntimePost::id),
                Expr::value(SqlValue::I64(101)),
            ))
            .first()
            .await?
            .expect("bounded nested step post");
        let mut bounded_nested_user = bounded_nested_post
            .user
            .as_ref()
            .expect("bounded nested step user")
            .clone();
        db.users
            .load_collection::<RuntimePost>(&mut bounded_nested_user, "posts")
            .await?;
        assert_eq!(bounded_nested_user.posts.as_slice().len(), 2);

        let mut bounded_nested_root = db
            .users
            .query()
            .include_as::<RuntimeProfile>("profile", "profile")?
            .filter(Predicate::eq(
                Expr::from(RuntimeUser::id),
                Expr::value(SqlValue::I64(1)),
            ))
            .first()
            .await?
            .expect("bounded nested root user");
        assert!(
            bounded_nested_root.profile.as_ref().is_some(),
            "root has_one include should populate the first graph edge"
        );
        db.users
            .load_collection::<RuntimePost>(&mut bounded_nested_root, "posts")
            .await?;
        let mut bounded_root_titles = bounded_nested_root
            .posts
            .as_slice()
            .iter()
            .map(|post| post.title.as_str())
            .collect::<Vec<_>>();
        bounded_root_titles.sort_unstable();
        assert_eq!(bounded_root_titles, vec!["First post", "Second post"]);
        assert!(
            bounded_nested_root.posts.as_slice()[0]
                .user
                .as_ref()
                .is_none(),
            "bounded graph loading must not introduce hidden nested includes"
        );

        Ok(())
    }
    .await;

    cleanup_navigation_tables(&connection_string, keep_tables).await?;

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

fn announce_test_tables(keep_tables: bool) {
    if keep_tables {
        eprintln!("keeping navigation runtime integration tables because {KEEP_TABLES_ENV}=1");
    } else {
        eprintln!("created navigation runtime integration tables");
    }
}

async fn reset_navigation_tables(connection_string: &str) -> Result<(), OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;

    drop_navigation_tables(&mut connection).await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {USERS_TABLE} (\
                    id BIGINT NOT NULL PRIMARY KEY,\
                    name NVARCHAR(120) NOT NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {PROFILES_TABLE} (\
                    id BIGINT NOT NULL PRIMARY KEY,\
                    user_id BIGINT NOT NULL,\
                    bio NVARCHAR(120) NOT NULL,\
                    CONSTRAINT fk_sql_orm_nav_profiles_user \
                        FOREIGN KEY (user_id) REFERENCES {USERS_TABLE}(id)\
                )"
            ),
            vec![],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {POSTS_TABLE} (\
                    id BIGINT NOT NULL PRIMARY KEY,\
                    user_id BIGINT NOT NULL,\
                    title NVARCHAR(120) NOT NULL,\
                    CONSTRAINT fk_sql_orm_nav_posts_user \
                        FOREIGN KEY (user_id) REFERENCES {USERS_TABLE}(id)\
                )"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn seed_navigation_rows(connection_string: &str) -> Result<(), OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;

    connection
        .execute(CompiledQuery::new(
            format!("INSERT INTO {USERS_TABLE} (id, name) VALUES (@P1, @P2), (@P3, @P4)"),
            vec![
                SqlValue::I64(1),
                SqlValue::String("Ana".to_string()),
                SqlValue::I64(2),
                SqlValue::String("Bruno".to_string()),
            ],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!("INSERT INTO {PROFILES_TABLE} (id, user_id, bio) VALUES (@P1, @P2, @P3)"),
            vec![
                SqlValue::I64(10),
                SqlValue::I64(1),
                SqlValue::String("Profile Ana".to_string()),
            ],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "INSERT INTO {POSTS_TABLE} (id, user_id, title) \
                 VALUES (@P1, @P2, @P3), (@P4, @P5, @P6), (@P7, @P8, @P9)"
            ),
            vec![
                SqlValue::I64(100),
                SqlValue::I64(1),
                SqlValue::String("First post".to_string()),
                SqlValue::I64(101),
                SqlValue::I64(1),
                SqlValue::String("Second post".to_string()),
                SqlValue::I64(102),
                SqlValue::I64(2),
                SqlValue::String("Other post".to_string()),
            ],
        ))
        .await?;

    Ok(())
}

async fn cleanup_navigation_tables(
    connection_string: &str,
    keep_tables: bool,
) -> Result<(), OrmError> {
    if keep_tables {
        return Ok(());
    }

    let mut connection = MssqlConnection::connect(connection_string).await?;
    drop_navigation_tables(&mut connection).await
}

async fn drop_navigation_tables(connection: &mut MssqlConnection) -> Result<(), OrmError> {
    for table in [POSTS_TABLE, PROFILES_TABLE, USERS_TABLE] {
        connection
            .execute(CompiledQuery::new(
                format!("IF OBJECT_ID('{table}', 'U') IS NOT NULL DROP TABLE {table}"),
                vec![],
            ))
            .await?;
    }

    Ok(())
}
