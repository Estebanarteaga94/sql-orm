use sql_orm::migrate::{
    MigrationOperation, ModelSnapshot, diff_column_operations, diff_schema_and_table_operations,
};
use sql_orm::prelude::*;
use sql_orm::sqlserver::SqlServerCompiler;

#[derive(AuditFields)]
#[allow(dead_code)]
struct Audit {
    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    #[orm(insertable = false)]
    #[orm(updatable = false)]
    created_at: String,

    #[orm(column = "created_by_user_id")]
    #[orm(nullable)]
    created_by: Option<i64>,

    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
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
    #[orm(unsafe_default_sql = "'new'")]
    status: Option<String>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "audited_entities", schema = "audit")]
struct PlainEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,

    #[orm(length = 40)]
    #[orm(unsafe_default_sql = "'new'")]
    status: Option<String>,
}

#[derive(SoftDeleteFields)]
#[allow(dead_code)]
struct SoftDelete {
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
    soft_delete = SoftDelete
)]
struct SoftDeletedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "soft_deleted_entities", schema = "audit")]
struct PlainSoftDeletedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,
}

#[test]
fn new_audited_entity_migration_creates_table_with_audit_columns() {
    let previous = ModelSnapshot::default();
    let current = ModelSnapshot::from_entities(&[AuditedEntity::metadata()]);
    let operations = diff_schema_and_table_operations(&previous, &current);

    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].schema_name(), "audit");
    assert_eq!(operations[0].table_name(), None);
    assert_eq!(operations[1].schema_name(), "audit");
    assert_eq!(operations[1].table_name(), Some("audited_entities"));

    let sql = SqlServerCompiler::compile_migration_operations(&operations)
        .expect("audited migration should compile");

    assert_eq!(
        sql[0],
        "IF SCHEMA_ID(N'audit') IS NULL EXEC(N'CREATE SCHEMA [audit]')"
    );
    assert_eq!(
        sql[1],
        "CREATE TABLE [audit].[audited_entities] (\n    [id] bigint IDENTITY(1, 1) NOT NULL,\n    [name] nvarchar(120) NOT NULL,\n    [status] nvarchar(40) NULL DEFAULT 'new',\n    [created_at] datetime2 NOT NULL DEFAULT SYSUTCDATETIME(),\n    [created_by_user_id] bigint NULL,\n    [updated_at] datetime2 NULL DEFAULT SYSUTCDATETIME(),\n    [updated_by] nvarchar(120) NULL,\n    PRIMARY KEY ([id])\n)"
    );
}

#[test]
fn new_soft_deleted_entity_migration_creates_table_with_soft_delete_columns() {
    let previous = ModelSnapshot::default();
    let current = ModelSnapshot::from_entities(&[SoftDeletedEntity::metadata()]);
    let operations = diff_schema_and_table_operations(&previous, &current);

    assert_eq!(operations.len(), 2);
    assert_eq!(operations[0].schema_name(), "audit");
    assert_eq!(operations[0].table_name(), None);
    assert_eq!(operations[1].schema_name(), "audit");
    assert_eq!(operations[1].table_name(), Some("soft_deleted_entities"));

    let sql = SqlServerCompiler::compile_migration_operations(&operations)
        .expect("soft delete migration should compile");

    assert_eq!(
        sql[0],
        "IF SCHEMA_ID(N'audit') IS NULL EXEC(N'CREATE SCHEMA [audit]')"
    );
    assert_eq!(
        sql[1],
        "CREATE TABLE [audit].[soft_deleted_entities] (\n    [id] bigint IDENTITY(1, 1) NOT NULL,\n    [name] nvarchar(120) NOT NULL,\n    [deleted_at] datetime2 NULL,\n    [deleted_by] nvarchar(120) NULL,\n    PRIMARY KEY ([id])\n)"
    );
}

#[test]
fn enabling_audit_on_existing_entity_emits_add_column_for_each_audit_column() {
    let previous = ModelSnapshot::from_entities(&[PlainEntity::metadata()]);
    let current = ModelSnapshot::from_entities(&[AuditedEntity::metadata()]);
    let operations = diff_column_operations(&previous, &current);

    assert_eq!(operations.len(), 4);

    let added_columns = operations
        .iter()
        .map(|operation| match operation {
            MigrationOperation::AddColumn(operation) => {
                assert_eq!(operation.schema_name, "audit");
                assert_eq!(operation.table_name, "audited_entities");
                operation.column.clone()
            }
            other => panic!("expected AddColumn operation, got {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        added_columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec![
            "created_at",
            "created_by_user_id",
            "updated_at",
            "updated_by",
        ]
    );

    let created_at = &added_columns[0];
    assert_eq!(created_at.sql_type, SqlServerType::DateTime2);
    assert_eq!(created_at.default_sql.as_deref(), Some("SYSUTCDATETIME()"));
    assert!(!created_at.nullable);
    assert!(!created_at.insertable);
    assert!(!created_at.updatable);

    let created_by = &added_columns[1];
    assert_eq!(created_by.sql_type, SqlServerType::BigInt);
    assert!(created_by.nullable);
    assert!(created_by.insertable);
    assert!(created_by.updatable);

    let updated_at = &added_columns[2];
    assert_eq!(updated_at.sql_type, SqlServerType::DateTime2);
    assert_eq!(updated_at.default_sql.as_deref(), Some("SYSUTCDATETIME()"));
    assert!(updated_at.nullable);
    assert!(!updated_at.insertable);
    assert!(updated_at.updatable);

    let updated_by = &added_columns[3];
    assert_eq!(updated_by.sql_type, SqlServerType::NVarChar);
    assert_eq!(updated_by.max_length, Some(120));
    assert!(updated_by.nullable);
    assert!(updated_by.insertable);
    assert!(updated_by.updatable);
}

#[test]
fn enabling_soft_delete_on_existing_entity_emits_add_column_for_each_soft_delete_column() {
    let previous = ModelSnapshot::from_entities(&[PlainSoftDeletedEntity::metadata()]);
    let current = ModelSnapshot::from_entities(&[SoftDeletedEntity::metadata()]);
    let operations = diff_column_operations(&previous, &current);

    assert_eq!(operations.len(), 2);

    let added_columns = operations
        .iter()
        .map(|operation| match operation {
            MigrationOperation::AddColumn(operation) => {
                assert_eq!(operation.schema_name, "audit");
                assert_eq!(operation.table_name, "soft_deleted_entities");
                operation.column.clone()
            }
            other => panic!("expected AddColumn operation, got {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        added_columns
            .iter()
            .map(|column| column.name.as_str())
            .collect::<Vec<_>>(),
        vec!["deleted_at", "deleted_by"]
    );

    let deleted_at = &added_columns[0];
    assert_eq!(deleted_at.sql_type, SqlServerType::DateTime2);
    assert!(deleted_at.nullable);
    assert!(!deleted_at.insertable);
    assert!(deleted_at.updatable);

    let deleted_by = &added_columns[1];
    assert_eq!(deleted_by.sql_type, SqlServerType::NVarChar);
    assert_eq!(deleted_by.max_length, Some(120));
    assert!(deleted_by.nullable);
    assert!(!deleted_by.insertable);
    assert!(deleted_by.updatable);
}

#[test]
fn removing_audit_from_existing_entity_emits_drop_column_for_each_audit_column() {
    let previous = ModelSnapshot::from_entities(&[AuditedEntity::metadata()]);
    let current = ModelSnapshot::from_entities(&[PlainEntity::metadata()]);
    let operations = diff_column_operations(&previous, &current);

    assert_eq!(operations.len(), 4);

    let dropped_columns = operations
        .iter()
        .map(|operation| match operation {
            MigrationOperation::DropColumn(operation) => {
                assert_eq!(operation.schema_name, "audit");
                assert_eq!(operation.table_name, "audited_entities");
                operation.column_name.as_str()
            }
            other => panic!("expected DropColumn operation, got {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(
        dropped_columns,
        vec![
            "created_at",
            "created_by_user_id",
            "updated_at",
            "updated_by",
        ]
    );
}

#[test]
fn removing_soft_delete_from_existing_entity_emits_drop_column_for_each_soft_delete_column() {
    let previous = ModelSnapshot::from_entities(&[SoftDeletedEntity::metadata()]);
    let current = ModelSnapshot::from_entities(&[PlainSoftDeletedEntity::metadata()]);
    let operations = diff_column_operations(&previous, &current);

    assert_eq!(operations.len(), 2);

    let dropped_columns = operations
        .iter()
        .map(|operation| match operation {
            MigrationOperation::DropColumn(operation) => {
                assert_eq!(operation.schema_name, "audit");
                assert_eq!(operation.table_name, "soft_deleted_entities");
                operation.column_name.as_str()
            }
            other => panic!("expected DropColumn operation, got {other:?}"),
        })
        .collect::<Vec<_>>();

    assert_eq!(dropped_columns, vec!["deleted_at", "deleted_by"]);
}
