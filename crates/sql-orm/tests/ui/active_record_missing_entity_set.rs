use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "dbo")]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub orders: DbSet<Order>,
}

fn main() {
    let _query = |db: &AppDbContext| {
        let _ = User::query(db);
    };
}
