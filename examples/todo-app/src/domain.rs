use sql_orm::prelude::*;

#[derive(AuditFields)]
pub struct TodoAudit {
    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    #[orm(updatable = false)]
    pub created_at: String,

    #[orm(column = "created_by_user_id")]
    pub created_by: Option<i64>,

    #[orm(nullable)]
    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    pub updated_at: Option<String>,

    #[orm(nullable)]
    #[orm(length = 120)]
    pub updated_by: Option<String>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(length = 180)]
    #[orm(unique)]
    pub email: String,

    #[orm(length = 120)]
    pub display_name: String,

    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    pub created_at: String,

    #[orm(rowversion)]
    pub version: Vec<u8>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists", schema = "todo")]
#[orm(index(name = "ix_todo_lists_owner_title", columns(owner_user_id, title)))]
pub struct TodoList {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    #[orm(on_delete = "cascade")]
    pub owner_user_id: i64,

    #[orm(length = 160)]
    pub title: String,

    #[orm(nullable)]
    #[orm(length = 500)]
    pub description: Option<String>,

    #[orm(unsafe_default_sql = "0")]
    pub is_archived: bool,

    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    pub created_at: String,

    #[orm(rowversion)]
    pub version: Vec<u8>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_items", schema = "todo")]
#[orm(index(name = "ix_todo_items_list_position", columns(list_id, position)))]
pub struct TodoItem {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = TodoList, column = id))]
    #[orm(on_delete = "cascade")]
    pub list_id: i64,

    #[orm(foreign_key(entity = User, column = id, name = "fk_todo_items_created_by_user"))]
    pub created_by_user_id: i64,

    #[orm(nullable)]
    #[orm(foreign_key(entity = User, column = id, name = "fk_todo_items_completed_by_user"))]
    pub completed_by_user_id: Option<i64>,

    #[orm(length = 200)]
    pub title: String,

    pub position: i32,

    #[orm(unsafe_default_sql = "0")]
    pub is_completed: bool,

    #[orm(nullable)]
    pub completed_at: Option<String>,

    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    pub created_at: String,

    #[orm(rowversion)]
    pub version: Vec<u8>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "audit_events", schema = "todo", audit = TodoAudit)]
pub struct AuditEvent {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(length = 80)]
    pub event_name: String,

    #[orm(length = 200)]
    pub subject: String,
}
