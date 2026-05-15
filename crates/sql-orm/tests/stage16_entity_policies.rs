use sql_orm::{EntityPersist, migrate::ModelSnapshot, prelude::*};
use std::collections::BTreeMap;

#[derive(AuditFields)]
#[allow(dead_code)]
struct Audit {
    #[orm(default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    #[orm(insertable = false)]
    #[orm(updatable = false)]
    created_at: String,

    #[orm(column = "created_by_user_id")]
    #[orm(nullable)]
    created_by: Option<i64>,

    #[orm(default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    #[orm(insertable = false)]
    updated_at: Option<String>,

    #[orm(nullable)]
    #[orm(length = 120)]
    updated_by: Option<String>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "audited_entities", schema = "audit", audit = Audit)]
struct AuditedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,

    #[orm(length = 40)]
    #[orm(default_sql = "'new'")]
    status: Option<String>,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = AuditedEntity)]
struct NewAuditedEntity {
    name: String,
    status: Option<String>,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = AuditedEntity)]
struct UpdateAuditedEntity {
    name: Option<String>,
    status: Option<Option<String>>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "archived_entities", schema = "audit")]
struct ArchivedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}

#[derive(SoftDeleteFields)]
#[allow(dead_code)]
struct SoftDeletePolicy {
    #[orm(sql_type = "datetime2")]
    deleted_at: Option<String>,

    #[orm(nullable)]
    #[orm(length = 120)]
    deleted_by: Option<String>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(
    table = "soft_deleted_entities",
    schema = "audit",
    soft_delete = SoftDeletePolicy
)]
struct SoftDeletedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,
}

struct TestRow {
    values: BTreeMap<&'static str, SqlValue>,
}

impl Row for TestRow {
    fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
        Ok(self.values.get(column).cloned())
    }
}

#[test]
fn audit_policy_columns_are_expanded_into_entity_metadata() {
    let metadata = AuditedEntity::metadata();
    let audit = AuditedEntity::audit_policy().expect("audit policy");

    assert_eq!(metadata.rust_name, "AuditedEntity");
    assert_eq!(metadata.schema, "audit");
    assert_eq!(metadata.table, "audited_entities");
    assert_eq!(metadata.renamed_from, None);
    assert_eq!(metadata.primary_key.columns, &["id"]);
    assert!(metadata.indexes.is_empty());
    assert!(metadata.foreign_keys.is_empty());

    assert_eq!(metadata.columns.len(), 7);
    assert_eq!(metadata.columns[0].column_name, "id");
    assert_eq!(metadata.columns[1].column_name, "name");
    assert_eq!(metadata.columns[2].column_name, "status");
    assert_eq!(metadata.columns[3].column_name, "created_at");
    assert_eq!(metadata.columns[4].column_name, "created_by_user_id");
    assert_eq!(metadata.columns[5].column_name, "updated_at");
    assert_eq!(metadata.columns[6].column_name, "updated_by");
    assert_eq!(audit.name, "audit");
    assert_eq!(audit.columns.len(), 4);
    assert_eq!(audit.columns[0].column_name, "created_at");
    assert_eq!(audit.columns[1].column_name, "created_by_user_id");
    assert_eq!(audit.columns[2].column_name, "updated_at");
    assert_eq!(audit.columns[3].column_name, "updated_by");

    let id = metadata
        .column("id")
        .expect("entity column should be present");
    assert_eq!(id.rust_field, "id");
    assert_eq!(id.sql_type, SqlServerType::BigInt);
    assert!(!id.nullable);
    assert!(id.primary_key);
    assert_eq!(id.identity, Some(IdentityMetadata::new(1, 1)));
    assert_eq!(id.default_sql, None);
    assert!(!id.insertable);
    assert!(!id.updatable);

    let name = metadata
        .column("name")
        .expect("entity column should be present");
    assert_eq!(name.rust_field, "name");
    assert_eq!(name.sql_type, SqlServerType::NVarChar);
    assert_eq!(name.max_length, Some(120));
    assert!(!name.nullable);
    assert_eq!(name.default_sql, None);
    assert!(name.insertable);
    assert!(name.updatable);

    let status = metadata
        .column("status")
        .expect("entity column should be present");
    assert_eq!(status.rust_field, "status");
    assert_eq!(status.sql_type, SqlServerType::NVarChar);
    assert_eq!(status.max_length, Some(40));
    assert!(status.nullable);
    assert_eq!(status.default_sql, Some("'new'"));
    assert!(status.insertable);
    assert!(status.updatable);

    let created_at = metadata
        .column("created_at")
        .expect("audit column should be present");
    assert_eq!(created_at.rust_field, "created_at");
    assert_eq!(created_at.sql_type, SqlServerType::DateTime2);
    assert_eq!(created_at.default_sql, Some("SYSUTCDATETIME()"));
    assert!(!created_at.nullable);
    assert!(!created_at.insertable);
    assert!(!created_at.updatable);

    let created_by = metadata
        .column("created_by_user_id")
        .expect("audit column should be present");
    assert_eq!(created_by.rust_field, "created_by");
    assert_eq!(created_by.sql_type, SqlServerType::BigInt);
    assert!(created_by.nullable);
    assert_eq!(created_by.default_sql, None);
    assert!(created_by.insertable);
    assert!(created_by.updatable);

    let updated_at = metadata
        .column("updated_at")
        .expect("audit column should be present");
    assert_eq!(updated_at.rust_field, "updated_at");
    assert_eq!(updated_at.sql_type, SqlServerType::DateTime2);
    assert!(updated_at.nullable);
    assert_eq!(updated_at.default_sql, Some("SYSUTCDATETIME()"));
    assert!(!updated_at.insertable);
    assert!(updated_at.updatable);

    let updated_by = metadata
        .column("updated_by")
        .expect("audit column should be present");
    assert!(updated_by.nullable);
    assert_eq!(updated_by.max_length, Some(120));
}

#[test]
fn entity_without_audit_exposes_empty_audit_runtime_contract() {
    assert_eq!(SoftDeletedEntity::audit_policy(), None);
}

#[test]
fn soft_delete_policy_columns_are_expanded_into_entity_metadata() {
    let metadata = SoftDeletedEntity::metadata();

    assert_eq!(metadata.columns.len(), 4);
    assert_eq!(metadata.columns[0].column_name, "id");
    assert_eq!(metadata.columns[1].column_name, "name");
    assert_eq!(metadata.columns[2].column_name, "deleted_at");
    assert_eq!(metadata.columns[3].column_name, "deleted_by");

    let deleted_at = metadata
        .column("deleted_at")
        .expect("soft delete column should be present");
    assert_eq!(deleted_at.rust_field, "deleted_at");
    assert_eq!(deleted_at.sql_type, SqlServerType::DateTime2);
    assert!(deleted_at.nullable);
    assert!(!deleted_at.insertable);
    assert!(deleted_at.updatable);

    let deleted_by = metadata
        .column("deleted_by")
        .expect("soft delete column should be present");
    assert_eq!(deleted_by.rust_field, "deleted_by");
    assert_eq!(deleted_by.max_length, Some(120));
    assert!(deleted_by.nullable);

    let soft_delete = SoftDeletedEntity::soft_delete_policy().expect("soft delete policy");
    assert_eq!(soft_delete.name, "soft_delete");
    assert_eq!(soft_delete.columns.len(), 2);
    assert_eq!(soft_delete.columns[0].column_name, "deleted_at");
    assert_eq!(soft_delete.columns[1].column_name, "deleted_by");
}

#[test]
fn audited_entity_from_row_materializes_only_real_entity_fields() {
    let row = TestRow {
        values: BTreeMap::from([
            ("id", SqlValue::I64(7)),
            ("name", SqlValue::String("sample".to_string())),
        ]),
    };

    let entity = AuditedEntity::from_row(&row).expect("audited entity should materialize");

    assert_eq!(
        entity,
        AuditedEntity {
            id: 7,
            name: "sample".to_string(),
            status: None,
        }
    );
}

#[test]
fn audited_entity_from_row_ignores_audit_metadata_columns_when_present() {
    let row = TestRow {
        values: BTreeMap::from([
            ("id", SqlValue::I64(9)),
            ("name", SqlValue::String("with audit columns".to_string())),
            (
                "created_at",
                SqlValue::String("2026-04-25T00:00:00".to_string()),
            ),
            ("updated_by", SqlValue::String("system".to_string())),
        ]),
    };

    let entity = AuditedEntity::from_row(&row).expect("audited entity should materialize");

    assert_eq!(
        entity,
        AuditedEntity {
            id: 9,
            name: "with audit columns".to_string(),
            status: None,
        }
    );
}

#[test]
fn insertable_and_changeset_do_not_auto_fill_audit_columns() {
    let values = <NewAuditedEntity as Insertable<AuditedEntity>>::values(&NewAuditedEntity {
        name: "new".to_string(),
        status: None,
    });
    let changes =
        <UpdateAuditedEntity as Changeset<AuditedEntity>>::changes(&UpdateAuditedEntity {
            name: Some("updated".to_string()),
            status: Some(None),
        });

    assert_eq!(
        column_names(&values),
        vec!["name", "status"],
        "DbSet::insert only receives explicit Insertable fields in the MVP"
    );
    assert_eq!(
        column_names(&changes),
        vec!["name", "status"],
        "DbSet::update only receives explicit Changeset fields in the MVP"
    );
    assert_no_audit_columns(&values);
    assert_no_audit_columns(&changes);
}

#[test]
fn entity_persist_for_active_record_and_save_changes_does_not_auto_fill_audit_columns() {
    let entity = AuditedEntity {
        id: 0,
        name: "tracked".to_string(),
        status: Some("new".to_string()),
    };

    let insert_values = <AuditedEntity as EntityPersist>::insert_values(&entity);
    let update_changes = <AuditedEntity as EntityPersist>::update_changes(&entity);

    assert_eq!(
        column_names(&insert_values),
        vec!["name", "status"],
        "Active Record insert/save routes use only real entity fields in the MVP"
    );
    assert_eq!(
        column_names(&update_changes),
        vec!["name", "status"],
        "Active Record update/save_changes routes use only real entity fields in the MVP"
    );
    assert_no_audit_columns(&insert_values);
    assert_no_audit_columns(&update_changes);
}

#[test]
fn model_snapshot_includes_audit_columns_without_special_pipeline() {
    let snapshot =
        ModelSnapshot::from_entities(&[AuditedEntity::metadata(), ArchivedEntity::metadata()]);
    let schema = snapshot
        .schema("audit")
        .expect("audit schema should be present");

    assert_eq!(schema.tables.len(), 2);
    assert_eq!(schema.tables[0].name, "archived_entities");
    assert_eq!(schema.tables[1].name, "audited_entities");

    let table = schema
        .table("audited_entities")
        .expect("audited table should be present");
    let column_names = table
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(
        column_names,
        vec![
            "id",
            "name",
            "status",
            "created_at",
            "created_by_user_id",
            "updated_at",
            "updated_by",
        ]
    );
    assert_eq!(table.primary_key_columns, vec!["id"]);

    let created_at = table
        .column("created_at")
        .expect("audit column should be present in snapshot");
    assert_eq!(created_at.sql_type, SqlServerType::DateTime2);
    assert_eq!(created_at.default_sql.as_deref(), Some("SYSUTCDATETIME()"));
    assert!(!created_at.nullable);
    assert!(!created_at.primary_key);
    assert!(!created_at.insertable);
    assert!(!created_at.updatable);

    let created_by = table
        .column("created_by_user_id")
        .expect("renamed audit column should be present in snapshot");
    assert_eq!(created_by.sql_type, SqlServerType::BigInt);
    assert!(created_by.nullable);
    assert!(created_by.insertable);
    assert!(created_by.updatable);

    let updated_by = table
        .column("updated_by")
        .expect("audit column should be present in snapshot");
    assert_eq!(updated_by.sql_type, SqlServerType::NVarChar);
    assert_eq!(updated_by.max_length, Some(120));
    assert!(updated_by.nullable);

    let json = snapshot
        .to_json_pretty()
        .expect("audited snapshot should serialize");
    assert!(json.contains("\"created_by_user_id\""));
    assert!(json.contains("\"SYSUTCDATETIME()\""));

    let roundtripped =
        ModelSnapshot::from_json(&json).expect("audited snapshot should deserialize");
    assert_eq!(roundtripped, snapshot);
    assert_eq!(
        ModelSnapshot::from_entities(&[AuditedEntity::metadata(), ArchivedEntity::metadata()]),
        snapshot
    );
}

#[test]
fn model_snapshot_includes_soft_delete_columns_without_special_pipeline() {
    let snapshot = ModelSnapshot::from_entities(&[SoftDeletedEntity::metadata()]);
    let schema = snapshot
        .schema("audit")
        .expect("audit schema should be present");
    let table = schema
        .table("soft_deleted_entities")
        .expect("soft deleted table should be present");
    let column_names = table
        .columns
        .iter()
        .map(|column| column.name.as_str())
        .collect::<Vec<_>>();

    assert_eq!(column_names, vec!["id", "name", "deleted_at", "deleted_by"]);
    assert_eq!(table.primary_key_columns, vec!["id"]);

    let deleted_at = table
        .column("deleted_at")
        .expect("soft delete column should be present in snapshot");
    assert_eq!(deleted_at.sql_type, SqlServerType::DateTime2);
    assert!(deleted_at.nullable);
    assert!(!deleted_at.insertable);
    assert!(deleted_at.updatable);

    let deleted_by = table
        .column("deleted_by")
        .expect("soft delete column should be present in snapshot");
    assert_eq!(deleted_by.sql_type, SqlServerType::NVarChar);
    assert_eq!(deleted_by.max_length, Some(120));
    assert!(deleted_by.nullable);

    let json = snapshot
        .to_json_pretty()
        .expect("soft delete snapshot should serialize");
    assert!(json.contains("\"deleted_at\""));
    assert!(json.contains("\"deleted_by\""));

    let roundtripped =
        ModelSnapshot::from_json(&json).expect("soft delete snapshot should deserialize");
    assert_eq!(roundtripped, snapshot);
}

fn column_names(values: &[ColumnValue]) -> Vec<&'static str> {
    values.iter().map(|value| value.column_name).collect()
}

fn assert_no_audit_columns(values: &[ColumnValue]) {
    let columns = column_names(values);

    for audit_column in [
        "created_at",
        "created_by_user_id",
        "updated_at",
        "updated_by",
    ] {
        assert!(
            !columns.contains(&audit_column),
            "{audit_column} should remain metadata/schema only in the MVP"
        );
    }
}
