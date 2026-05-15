use sql_orm::prelude::*;

struct SoftDeleteA;
struct SoftDeleteB;

impl EntityPolicy for SoftDeleteA {
    const POLICY_NAME: &'static str = "soft_delete";

    fn columns() -> &'static [ColumnMetadata] {
        &[]
    }
}

impl EntityPolicy for SoftDeleteB {
    const POLICY_NAME: &'static str = "soft_delete";

    fn columns() -> &'static [ColumnMetadata] {
        &[]
    }
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "soft_deleted_entities", schema = "audit", soft_delete = SoftDeleteA)]
#[orm(soft_delete = SoftDeleteB)]
struct SoftDeletedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}

fn main() {}
