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

    #[orm(foreign_key = "users.id")]
    pub owner_id: i64,

    #[orm(belongs_to(User, foreign_key = owner_id))]
    pub owner: Navigation<User>,
}

fn main() {}
