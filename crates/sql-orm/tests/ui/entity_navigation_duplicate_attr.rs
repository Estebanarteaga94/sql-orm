use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "profiles")]
pub struct Profile {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub user_id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "accounts")]
pub struct Account {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(belongs_to(User, foreign_key = user_id))]
    #[orm(has_one(Profile, foreign_key = user_id))]
    pub user: Navigation<User>,

    #[orm(foreign_key(entity = User, column = id))]
    pub user_id: i64,
}

fn main() {}
