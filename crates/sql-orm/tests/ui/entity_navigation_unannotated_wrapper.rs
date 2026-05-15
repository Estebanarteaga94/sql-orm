use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists")]
pub struct TodoList {
    #[orm(primary_key)]
    pub id: i64,

    pub owner: Navigation<User>,
}

fn main() {}
