use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    created_at: String,
}

#[derive(AuditFields)]
struct Timestamps {
    #[orm(column = "created_at")]
    created: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "audited_entities", schema = "audit", audit = Audit)]
#[orm(audit = Timestamps)]
struct AuditedEntity {
    #[orm(primary_key)]
    id: i64,
}

fn main() {}
