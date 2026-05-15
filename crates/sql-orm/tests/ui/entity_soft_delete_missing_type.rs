use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "soft_deleted_entities", schema = "audit", soft_delete = MissingSoftDelete)]
struct SoftDeletedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}

fn main() {}
