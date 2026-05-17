use sql_orm::prelude::*;

#[derive(Debug, Clone, PartialEq, FromRow)]
struct UserSummary {
    id: i64,
    #[orm(column = "email_address")]
    email: String,
    display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, FromRow)]
struct LowerEmail {
    #[orm(column = "lower_email")]
    value: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(column = "email_address", length = 180)]
    email: String,

    active: bool,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub users: DbSet<User>,
}

fn main() {
    let _query = |db: &AppDbContext| {
        let _all_future = db
            .users
            .query()
            .select((
                User::id,
                SelectProjection::expr_as(sql_orm::query::Expr::from(User::email), "email_address"),
                SelectProjection::expr_as(sql_orm::query::Expr::from(User::email), "display_name"),
            ))
            .all_as::<UserSummary>();

        let _first_future = db
            .users
            .query()
            .select(SelectProjection::expr_as(
                sql_orm::query::Expr::function(
                    sql_orm::query::SqlFunction::Lower,
                    vec![sql_orm::query::Expr::from(User::email)],
                ),
                "lower_email",
            ))
            .filter(User::active.eq(true))
            .first_as::<LowerEmail>();
    };
}
