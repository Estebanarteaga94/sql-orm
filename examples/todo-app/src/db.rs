use crate::domain::{AuditEvent, TodoItem, TodoList, User};
use sql_orm::prelude::*;

#[derive(DbContext, Debug, Clone)]
pub struct TodoAppDbContext {
    pub users: DbSet<User>,
    pub todo_lists: DbSet<TodoList>,
    pub todo_items: DbSet<TodoItem>,
    pub audit_events: DbSet<AuditEvent>,
}
