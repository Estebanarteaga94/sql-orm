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

fn main() {
    let _save = |db: &AppDbContext| {
        let mut user = User {
            id: 0,
            email: "ana@example.com".to_string(),
        };

        let future = user.save(db);
        let _ = future;
    };
}
