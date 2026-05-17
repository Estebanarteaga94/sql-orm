use sql_orm::prelude::*;
use std::collections::BTreeMap;

#[derive(AuditFields)]
struct Audit {
    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    #[orm(updatable = false)]
    created_at: String,

    #[orm(nullable)]
    #[orm(length = 120)]
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

struct PublicRow {
    values: BTreeMap<&'static str, SqlValue>,
}

impl Row for PublicRow {
    fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
        Ok(self.values.get(column).cloned())
    }
}

fn main() {
    let policy = Audit::metadata();
    assert_eq!(policy.name, "audit");
    assert_eq!(
        <Audit as EntityPolicy>::COLUMN_NAMES,
        &["created_at", "updated_by"]
    );

    let metadata = AuditedEntity::metadata();
    let audit = AuditedEntity::audit_policy().unwrap();
    assert_eq!(metadata.schema, "audit");
    assert_eq!(metadata.table, "audited_entities");
    assert_eq!(metadata.columns.len(), 4);
    assert!(metadata.column("created_at").is_some());
    assert!(metadata.column("updated_by").is_some());
    assert_eq!(audit.name, "audit");
    assert_eq!(audit.columns.len(), 2);
    assert_eq!(audit.columns[0].column_name, "created_at");
    assert_eq!(audit.columns[1].column_name, "updated_by");

    let row = PublicRow {
        values: BTreeMap::from([
            ("id", SqlValue::I64(1)),
            ("name", SqlValue::String("public".to_string())),
            (
                "created_at",
                SqlValue::String("2026-04-25T00:00:00".to_string()),
            ),
            ("updated_by", SqlValue::String("system".to_string())),
        ]),
    };

    let _entity = AuditedEntity::from_row(&row).unwrap();
}
