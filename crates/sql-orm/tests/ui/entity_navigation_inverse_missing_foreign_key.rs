use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(has_many(TodoList, foreign_key = owner_id))]
    pub lists: Collection<TodoList>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists")]
pub struct TodoList {
    #[orm(primary_key)]
    pub id: i64,

    pub owner_id: i64,
}

fn main() {}
