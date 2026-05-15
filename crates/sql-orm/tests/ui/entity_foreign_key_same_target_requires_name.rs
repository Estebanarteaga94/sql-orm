use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_items")]
pub struct TodoItem {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub created_by_user_id: i64,

    #[orm(nullable)]
    #[orm(foreign_key(entity = User, column = id))]
    pub completed_by_user_id: Option<i64>,
}

fn main() {}
