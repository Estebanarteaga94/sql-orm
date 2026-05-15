use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "audited_entities", schema = "audit", audit = MissingAudit)]
struct AuditedEntity {
    #[orm(primary_key)]
    id: i64,
}

fn main() {}
