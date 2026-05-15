use crate::TodoAppState;
use crate::db::TodoAppDbContext;
use crate::domain::{TodoItem, TodoList};
use crate::queries::{open_items_count_query, open_items_preview_query, user_lists_page_query};
use axum::Json;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use sql_orm::prelude::{DbContext, OrmError, PageRequest};
use serde::{Deserialize, Serialize};

pub trait TodoAppApi: DbContext + Clone + Send + Sync + 'static {
    fn find_list(&self, list_id: i64) -> impl FutureResult<Option<TodoList>>;

    fn list_user_lists(&self, user_id: i64, page: PageRequest) -> impl FutureResult<Vec<TodoList>>;

    fn preview_open_items(&self, list_id: i64, limit: u64) -> impl FutureResult<Vec<TodoItem>>;

    fn count_open_items(&self, list_id: i64) -> impl FutureResult<i64>;
}

pub trait FutureResult<T>: core::future::Future<Output = Result<T, OrmError>> + Send {}

impl<F, T> FutureResult<T> for F where F: core::future::Future<Output = Result<T, OrmError>> + Send {}

impl TodoAppApi for TodoAppDbContext {
    fn find_list(&self, list_id: i64) -> impl FutureResult<Option<TodoList>> {
        async move { self.todo_lists.find(list_id).await }
    }

    fn list_user_lists(&self, user_id: i64, page: PageRequest) -> impl FutureResult<Vec<TodoList>> {
        async move { user_lists_page_query(self, user_id, page).all().await }
    }

    fn preview_open_items(&self, list_id: i64, limit: u64) -> impl FutureResult<Vec<TodoItem>> {
        async move { open_items_preview_query(self, list_id, limit).all().await }
    }

    fn count_open_items(&self, list_id: i64) -> impl FutureResult<i64> {
        async move { open_items_count_query(self, list_id).count().await }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PageParams {
    #[serde(default = "default_page")]
    pub page: u64,
    #[serde(default = "default_page_size")]
    pub page_size: u64,
}

impl PageParams {
    pub fn request(&self) -> PageRequest {
        PageRequest::new(self.page, self.page_size)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct PreviewParams {
    #[serde(default = "default_preview_limit")]
    pub limit: u64,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TodoListResponse {
    pub id: i64,
    pub owner_user_id: i64,
    pub title: String,
    pub is_archived: bool,
    pub created_at: String,
}

impl From<TodoList> for TodoListResponse {
    fn from(value: TodoList) -> Self {
        Self {
            id: value.id,
            owner_user_id: value.owner_user_id,
            title: value.title,
            is_archived: value.is_archived,
            created_at: value.created_at,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct TodoItemPreviewResponse {
    pub id: i64,
    pub list_id: i64,
    pub title: String,
    pub position: i32,
    pub is_completed: bool,
}

impl From<TodoItem> for TodoItemPreviewResponse {
    fn from(value: TodoItem) -> Self {
        Self {
            id: value.id,
            list_id: value.list_id,
            title: value.title,
            position: value.position,
            is_completed: value.is_completed,
        }
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct OpenItemsCountResponse {
    pub count: i64,
}

pub async fn get_todo_list_handler<Db>(
    State(state): State<TodoAppState<Db>>,
    Path(list_id): Path<i64>,
) -> impl IntoResponse
where
    Db: TodoAppApi,
{
    match state.db.find_list(list_id).await {
        Ok(Some(list)) => (StatusCode::OK, Json(TodoListResponse::from(list))).into_response(),
        Ok(None) => StatusCode::NOT_FOUND.into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn list_user_lists_handler<Db>(
    State(state): State<TodoAppState<Db>>,
    Path(user_id): Path<i64>,
    Query(params): Query<PageParams>,
) -> impl IntoResponse
where
    Db: TodoAppApi,
{
    match state.db.list_user_lists(user_id, params.request()).await {
        Ok(lists) => (
            StatusCode::OK,
            Json(
                lists
                    .into_iter()
                    .map(TodoListResponse::from)
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn preview_open_items_handler<Db>(
    State(state): State<TodoAppState<Db>>,
    Path(list_id): Path<i64>,
    Query(params): Query<PreviewParams>,
) -> impl IntoResponse
where
    Db: TodoAppApi,
{
    match state.db.preview_open_items(list_id, params.limit).await {
        Ok(items) => (
            StatusCode::OK,
            Json(
                items
                    .into_iter()
                    .map(TodoItemPreviewResponse::from)
                    .collect::<Vec<_>>(),
            ),
        )
            .into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

pub async fn count_open_items_handler<Db>(
    State(state): State<TodoAppState<Db>>,
    Path(list_id): Path<i64>,
) -> impl IntoResponse
where
    Db: TodoAppApi,
{
    match state.db.count_open_items(list_id).await {
        Ok(count) => (StatusCode::OK, Json(OpenItemsCountResponse { count })).into_response(),
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}

const fn default_page() -> u64 {
    1
}

const fn default_page_size() -> u64 {
    20
}

const fn default_preview_limit() -> u64 {
    5
}
