use sql_orm::query::{Expr, SqlFunction};
use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 180)]
    email: String,

    active: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct UserSummary {
    id: i64,
    email: String,
}

impl FromRow for UserSummary {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            id: row.get_required_typed::<i64>("id")?,
            email: row.get_required_typed::<String>("email")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
struct LowerEmail {
    lower_email: String,
}

impl FromRow for LowerEmail {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            lower_email: row.get_required_typed::<String>("lower_email")?,
        })
    }
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub users: DbSet<User>,
}

fn main() {
    let _build_query = |db: &AppDbContext| {
        let _all_future = db
            .users
            .query()
            .select((User::id, User::email))
            .filter(User::active.eq(true))
            .order_by(User::id.asc())
            .all_as::<UserSummary>();

        let _first_future = db
            .users
            .query()
            .select(SelectProjection::expr_as(
                Expr::function(SqlFunction::Lower, vec![Expr::from(User::email)]),
                "lower_email",
            ))
            .first_as::<LowerEmail>();

        let _array_projection = db.users.query().select([User::id, User::email]);
        let _vec_projection = db
            .users
            .query()
            .select(vec![SelectProjection::column(User::id)]);
    };
}
