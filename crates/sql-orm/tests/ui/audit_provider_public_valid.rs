use sql_orm::prelude::*;
use std::sync::Arc;

#[derive(AuditFields)]
struct Audit {
    #[orm(nullable)]
    updated_by: Option<String>,
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

#[derive(Insertable)]
#[orm(entity = AuditedEntity)]
struct NewAuditedEntity {
    name: String,
}

#[derive(Changeset)]
#[orm(entity = AuditedEntity)]
struct AuditedEntityChanges {
    name: Option<String>,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    audited_entities: DbSet<AuditedEntity>,
}

struct PublicAuditProvider;

impl AuditProvider for PublicAuditProvider {
    fn values(&self, context: AuditContext<'_>) -> Result<Vec<ColumnValue>, OrmError> {
        let _entity = context.entity;
        let _operation = context.operation;
        let _request_values = context.request_values;

        Ok(vec![ColumnValue::new(
            "updated_by",
            SqlValue::String("public-provider".to_string()),
        )])
    }
}

fn main() {
    let provider: Arc<dyn AuditProvider> = Arc::new(PublicAuditProvider);
    let request_values = AuditRequestValues::new(vec![ColumnValue::new(
        "updated_by",
        SqlValue::String("public-request".to_string()),
    )]);

    let _with_provider = AppDbContext::with_audit_provider;
    let _with_request_values = AppDbContext::with_audit_request_values;
    let _clear_request_values = AppDbContext::clear_audit_request_values;
    let _shared_with_provider = SharedConnection::with_audit_provider;
    let _shared_with_request_values = SharedConnection::with_audit_request_values;
    let _shared_clear_request_values = SharedConnection::clear_audit_request_values;

    let context = AuditContext {
        entity: AuditedEntity::metadata(),
        operation: AuditOperation::Update,
        request_values: Some(&request_values),
    };

    let _resolved = resolve_audit_values(
        vec![ColumnValue::new(
            "name",
            SqlValue::String("explicit".to_string()),
        )],
        context,
        Some(provider.as_ref()),
    )
    .unwrap();
}
