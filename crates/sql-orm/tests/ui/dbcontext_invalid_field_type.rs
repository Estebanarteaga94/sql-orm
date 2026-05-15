use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub users: User,
}

fn main() {}
