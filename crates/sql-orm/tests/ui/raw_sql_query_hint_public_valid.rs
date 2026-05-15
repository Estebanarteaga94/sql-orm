use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    name: String,
}

#[derive(DbContext)]
struct AppDb {
    users: DbSet<User>,
}

fn accepts_query_hint_from_prelude(db: &AppDb) {
    let _query = db
        .raw::<User>("SELECT id, name FROM dbo.users WHERE id = @P1")
        .param(7_i64)
        .query_hint(QueryHint::Recompile);
}

fn main() {}
