use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users")]
struct User {
    #[orm(primary_key)]
    id: i64,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    users: DbSet<User>,
}

fn assert_no_ast_bypass(query: DbSetQuery<User>) {
    let _borrowed = query.select_query();
    let _owned = query.into_select_query();
}

fn main() {}
