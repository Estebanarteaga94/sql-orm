use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "legacy_fragments", schema = "dbo")]
pub struct LegacyFragments {
    #[orm(primary_key)]
    id: i64,

    #[orm(default_sql = "SYSUTCDATETIME()")]
    created_at: String,

}

fn main() {}
