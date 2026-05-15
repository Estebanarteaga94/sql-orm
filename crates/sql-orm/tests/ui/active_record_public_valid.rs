use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 180)]
    email: String,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub users: DbSet<User>,
}

fn accept_query(_query: DbSetQuery<User>) {}

fn main() {
    let _query = |db: &AppDbContext| {
        accept_query(User::query(db).filter(User::email.contains("@example.com")));
    };

    let _find = |db: &AppDbContext| {
        let future = User::find(db, 1_i64);
        let _ = future;
    };
}
