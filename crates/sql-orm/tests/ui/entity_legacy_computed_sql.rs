use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "legacy_computed", schema = "dbo")]
pub struct LegacyComputed {
    #[orm(primary_key)]
    id: i64,

    #[orm(computed_sql = "[first_name] + [last_name]")]
    full_name: String,
}

fn main() {}
