use sql_orm::prelude::*;

struct SoftDelete;

impl EntityPolicy for SoftDelete {
    const POLICY_NAME: &'static str = "soft_delete";
    const COLUMN_NAMES: &'static [&'static str] = &["deleted_at"];

    fn columns() -> &'static [ColumnMetadata] {
        &[]
    }
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "soft_deleted_entities", schema = "audit", soft_delete = SoftDelete)]
struct SoftDeletedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    deleted_at: String,
}

fn main() {}
