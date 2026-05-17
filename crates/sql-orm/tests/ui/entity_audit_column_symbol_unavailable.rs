use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    created_at: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "audited_entities", schema = "audit", audit = Audit)]
struct AuditedEntity {
    #[orm(primary_key)]
    id: i64,
}

fn main() {
    let _ = AuditedEntity::created_at;
}
