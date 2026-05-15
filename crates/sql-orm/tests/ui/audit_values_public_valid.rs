use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    #[orm(created_at)]
    #[orm(sql_type = "datetime2")]
    #[orm(updatable = false)]
    created_at: String,

    #[orm(created_by)]
    #[orm(column = "created_by_user_id")]
    created_by: Option<i64>,

    #[orm(updated_at)]
    #[orm(nullable)]
    updated_at: Option<String>,

    #[orm(updated_by)]
    #[orm(length = 120)]
    updated_by: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "audited_entities", schema = "audit", audit = Audit)]
struct AuditedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    audited_entities: DbSet<AuditedEntity>,
}

fn assert_audit_values<T: AuditValues>(values: T) -> Vec<ColumnValue> {
    values.audit_values()
}

fn main() {
    let audit = Audit {
        created_at: "2026-04-28T00:00:00Z".to_string(),
        created_by: Some(42),
        updated_at: None,
        updated_by: "system".to_string(),
    };

    let values = assert_audit_values(audit);

    assert_eq!(values[0].column_name, "created_at");
    assert_eq!(values[1].column_name, "created_by_user_id");
    assert_eq!(values[2].column_name, "updated_at");
    assert_eq!(values[3].column_name, "updated_by");

    let _with_typed_values = AppDbContext::with_audit_values::<Audit>;
    let _shared_with_typed_values = SharedConnection::with_audit_values::<Audit>;
}
