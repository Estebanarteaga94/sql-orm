use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::{Router, routing::get};
use sql_orm::prelude::{
    DbContext, MssqlConnectionConfig, MssqlHealthCheckOptions, MssqlHealthCheckQuery,
    MssqlOperationalOptions, MssqlParameterLogMode, MssqlPoolOptions, MssqlRetryOptions,
    MssqlSlowQueryOptions, MssqlTimeoutOptions, MssqlTracingOptions, OrmError,
};
#[cfg(feature = "pool-bb8")]
use sql_orm::{MssqlPool, MssqlPoolBuilder};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

pub mod db;
pub mod domain;
pub mod http;
pub mod queries;

pub use db::TodoAppDbContext;
pub use domain::{AuditEvent, TodoAudit, TodoItem, TodoList, User as TodoUser};
pub use http::{
    OpenItemsCountResponse, PageParams, PreviewParams, TodoAppApi, TodoItemPreviewResponse,
    TodoListResponse, count_open_items_handler, get_todo_list_handler, list_user_lists_handler,
    preview_open_items_handler,
};
pub use queries::{
    list_items_page_query, open_items_count_query, open_items_preview_query, user_lists_page_query,
};

const DEFAULT_APP_ADDR: &str = "127.0.0.1:3000";
const DEFAULT_RUST_LOG: &str = "info,todo_app=debug,sql_orm=debug";

#[derive(Debug, Clone, Default)]
pub struct PendingTodoAppDbContext;

impl DbContext for PendingTodoAppDbContext {
    fn from_shared_connection(_connection: sql_orm::SharedConnection) -> Self {
        Self
    }

    fn shared_connection(&self) -> sql_orm::SharedConnection {
        panic!("pending todo_app db context does not expose a shared connection yet")
    }

    fn tracking_registry(&self) -> sql_orm::TrackingRegistryHandle {
        std::sync::Arc::new(sql_orm::TrackingRegistry::default())
    }

    fn health_check(&self) -> impl std::future::Future<Output = Result<(), OrmError>> + Send {
        async { Err(OrmError::new("todo_app pool wiring is still pending")) }
    }
}

impl TodoAppApi for PendingTodoAppDbContext {
    fn find_list(&self, _list_id: i64) -> impl http::FutureResult<Option<TodoList>> {
        async { Err(OrmError::new("todo_app pool wiring is still pending")) }
    }

    fn list_user_lists(
        &self,
        _user_id: i64,
        _page: sql_orm::PageRequest,
    ) -> impl http::FutureResult<Vec<TodoList>> {
        async { Err(OrmError::new("todo_app pool wiring is still pending")) }
    }

    fn preview_open_items(
        &self,
        _list_id: i64,
        _limit: u64,
    ) -> impl http::FutureResult<Vec<TodoItem>> {
        async { Err(OrmError::new("todo_app pool wiring is still pending")) }
    }

    fn count_open_items(&self, _list_id: i64) -> impl http::FutureResult<i64> {
        async { Err(OrmError::new("todo_app pool wiring is still pending")) }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TodoAppSettings {
    pub bind_addr: SocketAddr,
    pub database_url: String,
    pub rust_log: String,
    pub operational_options: MssqlOperationalOptions,
}

impl TodoAppSettings {
    pub fn from_env() -> Result<Self, Box<dyn std::error::Error>> {
        let env = std::env::vars().collect::<HashMap<_, _>>();
        Self::from_map(&env)
    }

    pub fn from_map(env: &HashMap<String, String>) -> Result<Self, Box<dyn std::error::Error>> {
        let database_url = env
            .get("DATABASE_URL")
            .cloned()
            .ok_or("DATABASE_URL no está configurada para el ejemplo todo-app")?;
        let bind_addr = env
            .get("APP_ADDR")
            .map(String::as_str)
            .unwrap_or(DEFAULT_APP_ADDR)
            .parse()?;
        let rust_log = env
            .get("RUST_LOG")
            .cloned()
            .unwrap_or_else(|| DEFAULT_RUST_LOG.to_string());

        Ok(Self {
            bind_addr,
            database_url,
            rust_log,
            operational_options: default_operational_options(),
        })
    }

    pub fn connection_config(&self) -> Result<MssqlConnectionConfig, OrmError> {
        MssqlConnectionConfig::from_connection_string_with_options(
            &self.database_url,
            self.operational_options.clone(),
        )
    }
}

pub fn default_operational_options() -> MssqlOperationalOptions {
    MssqlOperationalOptions::new()
        .with_timeouts(
            MssqlTimeoutOptions::new()
                .with_connect_timeout(Duration::from_secs(5))
                .with_query_timeout(Duration::from_secs(10))
                .with_acquire_timeout(Duration::from_secs(2)),
        )
        .with_retry(MssqlRetryOptions::enabled(
            2,
            Duration::from_millis(100),
            Duration::from_millis(500),
        ))
        .with_tracing(
            MssqlTracingOptions::enabled().with_parameter_logging(MssqlParameterLogMode::Redacted),
        )
        .with_slow_query(
            MssqlSlowQueryOptions::enabled(Duration::from_millis(250))
                .with_parameter_logging(MssqlParameterLogMode::Redacted),
        )
        .with_health(
            MssqlHealthCheckOptions::enabled(MssqlHealthCheckQuery::SelectOne)
                .with_timeout(Duration::from_secs(2)),
        )
        .with_pool(
            MssqlPoolOptions::bb8(16)
                .with_min_idle(4)
                .with_acquire_timeout(Duration::from_secs(2))
                .with_idle_timeout(Duration::from_secs(300))
                .with_max_lifetime(Duration::from_secs(1800)),
        )
}

#[cfg(feature = "pool-bb8")]
pub fn pool_builder_from_settings(settings: &TodoAppSettings) -> MssqlPoolBuilder {
    MssqlPool::builder().with_pool_options(settings.operational_options.pool)
}

#[cfg(feature = "pool-bb8")]
pub async fn connect_pool(settings: &TodoAppSettings) -> Result<MssqlPool, OrmError> {
    pool_builder_from_settings(settings)
        .connect_with_config(settings.connection_config()?)
        .await
}

#[cfg(feature = "pool-bb8")]
pub fn state_from_pool(
    pool: MssqlPool,
    settings: TodoAppSettings,
) -> TodoAppState<TodoAppDbContext> {
    TodoAppState::new(TodoAppDbContext::from_pool(pool), settings)
}

#[derive(Debug, Clone)]
pub struct TodoAppState<Db> {
    pub db: Db,
    pub settings: TodoAppSettings,
}

impl<Db> TodoAppState<Db> {
    pub fn new(db: Db, settings: TodoAppSettings) -> Self {
        Self { db, settings }
    }
}

pub fn build_app<Db>(state: TodoAppState<Db>) -> Router
where
    Db: TodoAppApi,
{
    Router::new()
        .route("/health", get(health_check_handler::<Db>))
        .route("/todo-lists/{list_id}", get(get_todo_list_handler::<Db>))
        .route(
            "/users/{user_id}/todo-lists",
            get(list_user_lists_handler::<Db>),
        )
        .route(
            "/todo-lists/{list_id}/items/preview",
            get(preview_open_items_handler::<Db>),
        )
        .route(
            "/todo-lists/{list_id}/open-items/count",
            get(count_open_items_handler::<Db>),
        )
        .with_state(state)
}

pub async fn health_check_handler<Db>(State(state): State<TodoAppState<Db>>) -> impl IntoResponse
where
    Db: DbContext + Clone + Send + Sync + 'static,
{
    match state.db.health_check().await {
        Ok(()) => (StatusCode::OK, "ok"),
        Err(_) => (StatusCode::SERVICE_UNAVAILABLE, "database unavailable"),
    }
}

pub fn init_tracing(rust_log: &str) {
    let filter = tracing_subscriber::EnvFilter::try_new(rust_log)
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new(DEFAULT_RUST_LOG));

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_target(false)
        .compact()
        .init();
}

#[cfg(test)]
mod tests {
    use super::{
        DEFAULT_APP_ADDR, DEFAULT_RUST_LOG, TodoAppSettings, TodoAppState, build_app,
        default_operational_options, health_check_handler,
    };
    use crate::http::{
        PageParams, PreviewParams, TodoAppApi, count_open_items_handler, get_todo_list_handler,
        list_user_lists_handler, preview_open_items_handler,
    };
    use crate::{TodoItem, TodoList, http};
    use axum::body::to_bytes;
    use axum::extract::Query;
    use axum::extract::State;
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use sql_orm::prelude::OrmError;
    use sql_orm::{DbContext, SharedConnection, TrackingRegistry, TrackingRegistryHandle};
    use std::collections::HashMap;
    use std::future;
    use std::net::SocketAddr;
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Debug, Clone)]
    struct FakeDbContext {
        health_check_result: Result<(), OrmError>,
        todo_lists: Vec<TodoList>,
        todo_items: Vec<TodoItem>,
    }

    impl FakeDbContext {
        fn healthy() -> Self {
            Self {
                health_check_result: Ok(()),
                todo_lists: sample_lists(),
                todo_items: sample_items(),
            }
        }

        fn unhealthy() -> Self {
            Self {
                health_check_result: Err(OrmError::new("database unavailable")),
                todo_lists: Vec::new(),
                todo_items: Vec::new(),
            }
        }
    }

    impl DbContext for FakeDbContext {
        fn from_shared_connection(_connection: SharedConnection) -> Self {
            Self::healthy()
        }

        fn shared_connection(&self) -> SharedConnection {
            panic!("fake db context does not expose a real shared connection")
        }

        fn tracking_registry(&self) -> TrackingRegistryHandle {
            Arc::new(TrackingRegistry::default())
        }

        fn health_check(&self) -> impl future::Future<Output = Result<(), OrmError>> + Send {
            let result = self.health_check_result.clone();
            async move { result }
        }
    }

    impl TodoAppApi for FakeDbContext {
        fn find_list(&self, list_id: i64) -> impl http::FutureResult<Option<TodoList>> {
            let list = self
                .todo_lists
                .iter()
                .find(|list| list.id == list_id)
                .cloned();
            async move { Ok(list) }
        }

        fn list_user_lists(
            &self,
            user_id: i64,
            page: sql_orm::PageRequest,
        ) -> impl http::FutureResult<Vec<TodoList>> {
            let pagination = page.to_pagination();
            let start = pagination.offset as usize;
            let end = start.saturating_add(pagination.limit as usize);
            let lists = self
                .todo_lists
                .iter()
                .filter(|list| list.owner_user_id == user_id && !list.is_archived)
                .cloned()
                .skip(start)
                .take(end.saturating_sub(start))
                .collect::<Vec<_>>();
            async move { Ok(lists) }
        }

        fn preview_open_items(
            &self,
            list_id: i64,
            limit: u64,
        ) -> impl http::FutureResult<Vec<TodoItem>> {
            let items = self
                .todo_items
                .iter()
                .filter(|item| item.list_id == list_id && !item.is_completed)
                .take(limit as usize)
                .cloned()
                .collect::<Vec<_>>();
            async move { Ok(items) }
        }

        fn count_open_items(&self, list_id: i64) -> impl http::FutureResult<i64> {
            let count = self
                .todo_items
                .iter()
                .filter(|item| item.list_id == list_id && !item.is_completed)
                .count() as i64;
            async move { Ok(count) }
        }
    }

    fn sample_lists() -> Vec<TodoList> {
        vec![
            TodoList {
                id: 10,
                owner_user_id: 7,
                title: "Inbox".to_string(),
                description: Some("Daily working list".to_string()),
                is_archived: false,
                created_at: "2026-04-23T00:00:00".to_string(),
                version: vec![1],
            },
            TodoList {
                id: 11,
                owner_user_id: 7,
                title: "Archived".to_string(),
                description: None,
                is_archived: true,
                created_at: "2026-04-23T00:00:00".to_string(),
                version: vec![2],
            },
        ]
    }

    fn sample_items() -> Vec<TodoItem> {
        vec![
            TodoItem {
                id: 100,
                list_id: 10,
                created_by_user_id: 7,
                completed_by_user_id: None,
                title: "Ship release".to_string(),
                position: 1,
                is_completed: false,
                completed_at: None,
                created_at: "2026-04-23T00:00:00".to_string(),
                version: vec![1],
            },
            TodoItem {
                id: 101,
                list_id: 10,
                created_by_user_id: 7,
                completed_by_user_id: Some(7),
                title: "Write docs".to_string(),
                position: 2,
                is_completed: true,
                completed_at: Some("2026-04-23T01:00:00".to_string()),
                created_at: "2026-04-23T00:00:00".to_string(),
                version: vec![2],
            },
            TodoItem {
                id: 102,
                list_id: 10,
                created_by_user_id: 7,
                completed_by_user_id: None,
                title: "Review PR".to_string(),
                position: 3,
                is_completed: false,
                completed_at: None,
                created_at: "2026-04-23T00:00:00".to_string(),
                version: vec![3],
            },
        ]
    }

    fn env_with_database_url() -> HashMap<String, String> {
        HashMap::from([(
            "DATABASE_URL".to_string(),
            "server=tcp:localhost,1433;database=tempdb;user=sa;password=Password123;TrustServerCertificate=true".to_string(),
        )])
    }

    #[test]
    fn settings_validate_required_database_url_and_defaults() {
        let env = HashMap::<String, String>::new();
        let error = TodoAppSettings::from_map(&env).unwrap_err();

        assert_eq!(
            error.to_string(),
            "DATABASE_URL no está configurada para el ejemplo todo-app"
        );

        let env = env_with_database_url();
        let settings = TodoAppSettings::from_map(&env).unwrap();

        assert_eq!(
            settings.bind_addr,
            DEFAULT_APP_ADDR.parse::<SocketAddr>().unwrap()
        );
        assert_eq!(settings.rust_log, DEFAULT_RUST_LOG);
        assert!(settings.operational_options.pool.enabled);
        assert_eq!(settings.operational_options.pool.max_size, 16);
        assert_eq!(settings.operational_options.pool.min_idle, Some(4));
    }

    #[test]
    fn settings_accept_overrides_and_build_connection_config() {
        let mut env = env_with_database_url();
        env.insert("APP_ADDR".to_string(), "0.0.0.0:4040".to_string());
        env.insert("RUST_LOG".to_string(), "debug,todo_app=trace".to_string());

        let settings = TodoAppSettings::from_map(&env).unwrap();

        assert_eq!(
            settings.bind_addr,
            "0.0.0.0:4040".parse::<SocketAddr>().unwrap()
        );
        assert_eq!(settings.rust_log, "debug,todo_app=trace");
        let config = settings.connection_config().unwrap();

        assert_eq!(config.connection_string(), settings.database_url);
        assert_eq!(config.options(), &settings.operational_options);
    }

    #[test]
    fn operational_options_and_router_can_be_built_without_services() {
        let options = default_operational_options();

        assert_eq!(
            options.timeouts.connect_timeout,
            Some(Duration::from_secs(5))
        );
        assert_eq!(
            options.timeouts.query_timeout,
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            options.timeouts.acquire_timeout,
            Some(Duration::from_secs(2))
        );
        assert!(options.retry.enabled);
        assert_eq!(options.retry.max_retries, 2);
        assert!(options.tracing.enabled);
        assert!(options.slow_query.enabled);
        assert!(options.health.enabled);
        assert!(options.pool.enabled);

        let settings = TodoAppSettings::from_map(&env_with_database_url()).unwrap();
        let state = TodoAppState::new(FakeDbContext::healthy(), settings.clone());
        let _app = build_app(state.clone());

        assert_eq!(state.settings, settings);
    }

    #[tokio::test]
    async fn public_handlers_cover_health_and_read_responses() {
        let settings = TodoAppSettings::from_map(&env_with_database_url()).unwrap();

        let healthy = TodoAppState::new(FakeDbContext::healthy(), settings.clone());
        let response = health_check_handler(State(healthy.clone()))
            .await
            .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(&body[..], b"ok");

        let unhealthy = TodoAppState::new(FakeDbContext::unhealthy(), settings);
        let response = health_check_handler(State(unhealthy)).await.into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(&body[..], b"database unavailable");

        let response =
            get_todo_list_handler::<FakeDbContext>(State(healthy.clone()), axum::extract::Path(10))
                .await
                .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            r#"{"id":10,"owner_user_id":7,"title":"Inbox","is_archived":false,"created_at":"2026-04-23T00:00:00"}"#
        );

        let response = get_todo_list_handler::<FakeDbContext>(
            State(healthy.clone()),
            axum::extract::Path(999),
        )
        .await
        .into_response();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);

        let response = list_user_lists_handler::<FakeDbContext>(
            State(healthy.clone()),
            axum::extract::Path(7),
            Query(PageParams {
                page: 1,
                page_size: 20,
            }),
        )
        .await
        .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            r#"[{"id":10,"owner_user_id":7,"title":"Inbox","is_archived":false,"created_at":"2026-04-23T00:00:00"}]"#
        );

        let response = preview_open_items_handler::<FakeDbContext>(
            State(healthy.clone()),
            axum::extract::Path(10),
            Query(PreviewParams { limit: 1 }),
        )
        .await
        .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            String::from_utf8(body.to_vec()).unwrap(),
            r#"[{"id":100,"list_id":10,"title":"Ship release","position":1,"is_completed":false}]"#
        );

        let response =
            count_open_items_handler::<FakeDbContext>(State(healthy), axum::extract::Path(10))
                .await
                .into_response();
        let status = response.status();
        let body = to_bytes(response.into_body(), usize::MAX).await.unwrap();
        assert_eq!(status, StatusCode::OK);
        assert_eq!(String::from_utf8(body.to_vec()).unwrap(), r#"{"count":2}"#);
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn settings_build_pool_builder_from_operational_profile() {
        let settings = TodoAppSettings::from_map(&env_with_database_url()).unwrap();
        let builder = super::pool_builder_from_settings(&settings);

        assert_eq!(builder.options(), settings.operational_options.pool);
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn example_exposes_dbcontext_from_pool_wiring() {
        let _from_pool = crate::TodoAppDbContext::from_pool;
        let _state_from_pool = super::state_from_pool;
        let _connect_pool = super::connect_pool;
    }

    #[cfg(feature = "pool-bb8")]
    #[tokio::test]
    #[ignore = "requires a live SQL Server fixture configured through DATABASE_URL"]
    async fn smoke_preview_query_runs_against_sql_server_fixture() {
        let settings = TodoAppSettings::from_env().expect("DATABASE_URL for smoke test");
        let pool = super::connect_pool(&settings)
            .await
            .expect("connect smoke test pool");
        let db = crate::TodoAppDbContext::from_pool(pool);
        let items = crate::open_items_preview_query(&db, 10, 2)
            .all()
            .await
            .expect("load preview items from SQL Server");

        assert_eq!(items.len(), 2);
        assert_eq!(
            items
                .iter()
                .map(|item| item.title.as_str())
                .collect::<Vec<_>>(),
            vec!["Ship release", "Review PR"]
        );
    }
}
