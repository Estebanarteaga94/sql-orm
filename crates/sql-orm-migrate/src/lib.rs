//! Migration support foundations.

use sql_orm_core::CrateIdentity;

mod diff;
mod filesystem;
mod operation;
mod snapshot;

pub use diff::{
    diff_column_operations, diff_relational_operations, diff_schema_and_table_operations,
};
pub use filesystem::{
    MigrationEntry, MigrationScaffold, build_database_downgrade_script,
    build_database_update_script, create_migration_scaffold,
    create_migration_scaffold_with_snapshot, latest_migration, list_migrations,
    read_latest_model_snapshot, read_model_snapshot, write_migration_down_sql,
    write_migration_up_sql, write_model_snapshot,
};
pub use operation::{
    AddColumn, AddForeignKey, AlterColumn, CreateIndex, CreateSchema, CreateTable, DropColumn,
    DropForeignKey, DropIndex, DropSchema, DropTable, MigrationOperation, RenameColumn,
    RenameTable,
};
pub use snapshot::{
    ColumnSnapshot, ForeignKeySnapshot, IndexColumnSnapshot, IndexSnapshot, ModelSnapshot,
    SchemaSnapshot, TableSnapshot,
};

/// Placeholder migration engine marker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationEngine;

pub const CRATE_IDENTITY: CrateIdentity = CrateIdentity {
    name: "sql-orm-migrate",
    responsibility: "code-first snapshots, diffs and migration operations",
};

#[cfg(test)]
mod tests {
    use super::{
        AddColumn, AddForeignKey, AlterColumn, CRATE_IDENTITY, ColumnSnapshot, CreateIndex,
        CreateSchema, CreateTable, DropColumn, DropForeignKey, DropIndex, DropSchema, DropTable,
        ForeignKeySnapshot, IndexColumnSnapshot, IndexSnapshot, MigrationEngine,
        MigrationOperation, ModelSnapshot, RenameColumn, RenameTable, SchemaSnapshot,
        TableSnapshot,
    };
    use sql_orm_core::{
        ColumnMetadata, EntityMetadata, ForeignKeyMetadata, IdentityMetadata, IndexColumnMetadata,
        IndexMetadata, PrimaryKeyMetadata, ReferentialAction, SqlServerType,
    };

    const CUSTOMER_COLUMNS: [ColumnMetadata; 3] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: Some(IdentityMetadata::new(1, 1)),
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "email",
            column_name: "email",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(160),
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "version",
            column_name: "version",
            renamed_from: None,
            sql_type: SqlServerType::RowVersion,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: true,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    const CUSTOMER_PK_COLUMNS: [&str; 1] = ["id"];
    const CUSTOMER_INDEX_COLUMNS: [IndexColumnMetadata; 1] = [IndexColumnMetadata::asc("email")];
    const CUSTOMER_INDEXES: [IndexMetadata; 1] = [IndexMetadata {
        name: "ix_customers_email",
        columns: &CUSTOMER_INDEX_COLUMNS,
        unique: true,
    }];
    const CUSTOMER_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "Customer",
        schema: "sales",
        table: "customers",
        renamed_from: None,
        columns: &CUSTOMER_COLUMNS,
        primary_key: PrimaryKeyMetadata::new(Some("pk_customers"), &CUSTOMER_PK_COLUMNS),
        indexes: &CUSTOMER_INDEXES,
        foreign_keys: &[],
        navigations: &[],
    };

    const TENANT_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: Some(IdentityMetadata::new(100, 5)),
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "display_name",
            column_name: "display_name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: Some("'tenant'"),
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
    ];

    const TENANT_PK_COLUMNS: [&str; 1] = ["id"];
    const TENANT_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "Tenant",
        schema: "admin",
        table: "tenants",
        renamed_from: None,
        columns: &TENANT_COLUMNS,
        primary_key: PrimaryKeyMetadata::new(None, &TENANT_PK_COLUMNS),
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    const COMPOSITE_ORDER_COLUMNS: [ColumnMetadata; 3] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: Some(IdentityMetadata::new(1, 1)),
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "customer_id",
            column_name: "customer_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "total_cents",
            column_name: "total_cents",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];
    const COMPOSITE_ORDER_PK_COLUMNS: [&str; 1] = ["id"];
    const COMPOSITE_ORDER_INDEX_COLUMNS: [IndexColumnMetadata; 2] = [
        IndexColumnMetadata::asc("customer_id"),
        IndexColumnMetadata::desc("total_cents"),
    ];
    const COMPOSITE_ORDER_INDEXES: [IndexMetadata; 1] = [IndexMetadata {
        name: "ix_orders_customer_total",
        columns: &COMPOSITE_ORDER_INDEX_COLUMNS,
        unique: false,
    }];
    const COMPOSITE_ORDER_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "CompositeOrder",
        schema: "sales",
        table: "orders",
        renamed_from: None,
        columns: &COMPOSITE_ORDER_COLUMNS,
        primary_key: PrimaryKeyMetadata::new(Some("pk_orders"), &COMPOSITE_ORDER_PK_COLUMNS),
        indexes: &COMPOSITE_ORDER_INDEXES,
        foreign_keys: &[],
        navigations: &[],
    };

    const ORDER_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: Some(IdentityMetadata::new(1, 1)),
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "customer_id",
            column_name: "customer_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    const ORDER_PK_COLUMNS: [&str; 1] = ["id"];
    const ORDER_FOREIGN_KEYS: [ForeignKeyMetadata; 1] = [ForeignKeyMetadata::new(
        "fk_orders_customer_id_customers",
        &["customer_id"],
        "sales",
        "customers",
        &["id"],
        ReferentialAction::NoAction,
        ReferentialAction::NoAction,
    )];
    const ORDER_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "Order",
        schema: "sales",
        table: "orders",
        renamed_from: None,
        columns: &ORDER_COLUMNS,
        primary_key: PrimaryKeyMetadata::new(Some("pk_orders"), &ORDER_PK_COLUMNS),
        indexes: &[],
        foreign_keys: &ORDER_FOREIGN_KEYS,
        navigations: &[],
    };

    const ORDER_ALLOCATION_COLUMNS: [ColumnMetadata; 3] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: Some(IdentityMetadata::new(1, 1)),
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "customer_id",
            column_name: "customer_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "branch_id",
            column_name: "branch_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];
    const ORDER_ALLOCATION_PK_COLUMNS: [&str; 1] = ["id"];
    const ORDER_ALLOCATION_FOREIGN_KEYS: [ForeignKeyMetadata; 1] = [ForeignKeyMetadata::new(
        "fk_order_allocations_customer_branch_customers",
        &["customer_id", "branch_id"],
        "sales",
        "customers",
        &["id", "branch_id"],
        ReferentialAction::SetDefault,
        ReferentialAction::Cascade,
    )];
    const ORDER_ALLOCATION_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "OrderAllocation",
        schema: "sales",
        table: "order_allocations",
        renamed_from: None,
        columns: &ORDER_ALLOCATION_COLUMNS,
        primary_key: PrimaryKeyMetadata::new(
            Some("pk_order_allocations"),
            &ORDER_ALLOCATION_PK_COLUMNS,
        ),
        indexes: &[],
        foreign_keys: &ORDER_ALLOCATION_FOREIGN_KEYS,
        navigations: &[],
    };

    #[test]
    fn declares_migration_boundary() {
        let engine = MigrationEngine;
        assert_eq!(engine, MigrationEngine);
        assert!(CRATE_IDENTITY.responsibility.contains("migration"));
    }

    #[test]
    fn model_snapshot_exposes_schema_table_column_and_index_lookups() {
        let snapshot = ModelSnapshot::new(vec![SchemaSnapshot::new(
            "sales",
            vec![TableSnapshot::new(
                "customers",
                vec![
                    ColumnSnapshot::new(
                        "id",
                        SqlServerType::BigInt,
                        false,
                        true,
                        Some(IdentityMetadata::new(1, 1)),
                        None,
                        None,
                        false,
                        false,
                        false,
                        None,
                        None,
                        None,
                    ),
                    ColumnSnapshot::new(
                        "email",
                        SqlServerType::NVarChar,
                        false,
                        false,
                        None,
                        None,
                        None,
                        false,
                        true,
                        true,
                        Some(160),
                        None,
                        None,
                    ),
                ],
                Some("pk_customers".to_string()),
                vec!["id".to_string()],
                vec![IndexSnapshot::new(
                    "ix_customers_email",
                    vec![IndexColumnSnapshot::asc("email")],
                    false,
                )],
                vec![],
            )],
        )]);

        let schema = snapshot.schema("sales").expect("schema must exist");
        let table = schema.table("customers").expect("table must exist");
        let id = table.column("id").expect("column must exist");
        let index = table.index("ix_customers_email").expect("index must exist");

        assert_eq!(table.primary_key_name.as_deref(), Some("pk_customers"));
        assert_eq!(table.primary_key_columns, vec!["id"]);
        assert_eq!(id.identity, Some(IdentityMetadata::new(1, 1)));
        assert_eq!(index.columns, vec![IndexColumnSnapshot::asc("email")]);
    }

    #[test]
    fn column_snapshot_preserves_sql_server_specific_shape() {
        let column = ColumnSnapshot::new(
            "version",
            SqlServerType::RowVersion,
            false,
            false,
            None,
            Some("CONVERT(binary(8), 0)".to_string()),
            Some("([major] + [minor])".to_string()),
            true,
            false,
            false,
            Some(8),
            Some(18),
            Some(4),
        );

        assert_eq!(column.name, "version");
        assert_eq!(column.sql_type, SqlServerType::RowVersion);
        assert_eq!(column.default_sql.as_deref(), Some("CONVERT(binary(8), 0)"));
        assert_eq!(column.computed_sql.as_deref(), Some("([major] + [minor])"));
        assert!(column.rowversion);
        assert!(!column.insertable);
        assert!(!column.updatable);
        assert_eq!(column.max_length, Some(8));
        assert_eq!(column.precision, Some(18));
        assert_eq!(column.scale, Some(4));
    }

    #[test]
    fn table_snapshot_can_be_built_from_entity_metadata() {
        let table = TableSnapshot::from(&CUSTOMER_METADATA);

        assert_eq!(table.name, "customers");
        assert_eq!(table.primary_key_name.as_deref(), Some("pk_customers"));
        assert_eq!(table.primary_key_columns, vec!["id"]);
        assert_eq!(table.columns.len(), 3);
        assert_eq!(table.columns[0].name, "id");
        assert_eq!(table.columns[1].name, "email");
        assert_eq!(table.indexes.len(), 1);
        assert_eq!(table.indexes[0].name, "ix_customers_email");
        assert!(table.indexes[0].unique);
        assert!(table.foreign_keys.is_empty());
    }

    #[test]
    fn table_snapshot_preserves_foreign_keys_from_entity_metadata() {
        let table = TableSnapshot::from(&ORDER_METADATA);
        let foreign_key = table
            .foreign_key("fk_orders_customer_id_customers")
            .expect("foreign key must exist");

        assert_eq!(table.foreign_keys.len(), 1);
        assert_eq!(foreign_key.columns, vec!["customer_id"]);
        assert_eq!(foreign_key.referenced_schema, "sales");
        assert_eq!(foreign_key.referenced_table, "customers");
        assert_eq!(foreign_key.referenced_columns, vec!["id"]);
        assert_eq!(foreign_key.on_delete, ReferentialAction::NoAction);
        assert_eq!(foreign_key.on_update, ReferentialAction::NoAction);
    }

    #[test]
    fn table_snapshot_preserves_composite_foreign_keys_from_entity_metadata() {
        let table = TableSnapshot::from(&ORDER_ALLOCATION_METADATA);
        let foreign_key = table
            .foreign_key("fk_order_allocations_customer_branch_customers")
            .expect("composite foreign key must exist");

        assert_eq!(table.foreign_keys.len(), 1);
        assert_eq!(foreign_key.columns, vec!["customer_id", "branch_id"]);
        assert_eq!(foreign_key.referenced_schema, "sales");
        assert_eq!(foreign_key.referenced_table, "customers");
        assert_eq!(foreign_key.referenced_columns, vec!["id", "branch_id"]);
        assert_eq!(foreign_key.on_delete, ReferentialAction::SetDefault);
        assert_eq!(foreign_key.on_update, ReferentialAction::Cascade);
    }

    #[test]
    fn table_snapshot_preserves_composite_indexes_from_entity_metadata() {
        let table = TableSnapshot::from(&COMPOSITE_ORDER_METADATA);
        let index = table
            .index("ix_orders_customer_total")
            .expect("composite index must exist");

        assert_eq!(table.indexes.len(), 1);
        assert_eq!(
            index.columns,
            vec![
                IndexColumnSnapshot::asc("customer_id"),
                IndexColumnSnapshot::desc("total_cents"),
            ]
        );
        assert!(!index.unique);
    }

    #[test]
    fn model_snapshot_groups_entities_by_schema_and_sorts_tables() {
        let snapshot =
            ModelSnapshot::from_entities(&[&ORDER_METADATA, &TENANT_METADATA, &CUSTOMER_METADATA]);

        assert_eq!(snapshot.schemas.len(), 2);
        assert_eq!(snapshot.schemas[0].name, "admin");
        assert_eq!(snapshot.schemas[1].name, "sales");

        let admin = snapshot.schema("admin").expect("admin schema must exist");
        assert_eq!(admin.tables.len(), 1);
        assert_eq!(admin.tables[0].name, "tenants");

        let sales = snapshot.schema("sales").expect("sales schema must exist");
        assert_eq!(
            sales
                .tables
                .iter()
                .map(|table| table.name.as_str())
                .collect::<Vec<_>>(),
            vec!["customers", "orders"]
        );
        assert_eq!(
            sales
                .table("customers")
                .expect("customers table must exist")
                .column("email")
                .expect("email column must exist")
                .max_length,
            Some(160)
        );
    }

    #[test]
    fn migration_operations_cover_minimum_stage_seven_surface() {
        let create_schema = MigrationOperation::CreateSchema(CreateSchema::new("sales"));
        let drop_schema = MigrationOperation::DropSchema(DropSchema::new("legacy"));
        let create_table = MigrationOperation::CreateTable(CreateTable::new(
            "sales",
            TableSnapshot::from(&CUSTOMER_METADATA),
        ));
        let drop_table = MigrationOperation::DropTable(DropTable::new("sales", "customers"));
        let add_column = MigrationOperation::AddColumn(AddColumn::new(
            "sales",
            "customers",
            ColumnSnapshot::from(&CUSTOMER_COLUMNS[1]),
        ));
        let drop_column =
            MigrationOperation::DropColumn(DropColumn::new("sales", "customers", "email"));
        let rename_column = MigrationOperation::RenameColumn(RenameColumn::new(
            "sales",
            "customers",
            "email",
            "email_address",
        ));
        let rename_table =
            MigrationOperation::RenameTable(RenameTable::new("sales", "customers", "clients"));
        let alter_column = MigrationOperation::AlterColumn(AlterColumn::new(
            "sales",
            "customers",
            ColumnSnapshot::from(&CUSTOMER_COLUMNS[1]),
            ColumnSnapshot::new(
                "email",
                SqlServerType::NVarChar,
                false,
                false,
                None,
                None,
                None,
                false,
                true,
                true,
                Some(255),
                None,
                None,
            ),
        ));
        let create_index = MigrationOperation::CreateIndex(CreateIndex::new(
            "sales",
            "customers",
            IndexSnapshot::new(
                "ix_customers_email",
                vec![IndexColumnSnapshot::asc("email")],
                true,
            ),
        ));
        let drop_index = MigrationOperation::DropIndex(DropIndex::new(
            "sales",
            "customers",
            "ix_customers_email",
        ));
        let add_foreign_key = MigrationOperation::AddForeignKey(AddForeignKey::new(
            "sales",
            "orders",
            ForeignKeySnapshot::new(
                "fk_orders_customer_id_customers",
                vec!["customer_id".to_string()],
                "sales",
                "customers",
                vec!["id".to_string()],
                ReferentialAction::NoAction,
                ReferentialAction::NoAction,
            ),
        ));
        let drop_foreign_key = MigrationOperation::DropForeignKey(DropForeignKey::new(
            "sales",
            "orders",
            "fk_orders_customer_id_customers",
        ));

        assert_eq!(create_schema.schema_name(), "sales");
        assert_eq!(drop_schema.schema_name(), "legacy");
        assert_eq!(create_table.schema_name(), "sales");
        assert_eq!(create_table.table_name(), Some("customers"));
        assert_eq!(drop_table.table_name(), Some("customers"));
        assert_eq!(add_column.table_name(), Some("customers"));
        assert_eq!(drop_column.table_name(), Some("customers"));
        assert_eq!(rename_column.table_name(), Some("customers"));
        assert_eq!(rename_table.table_name(), Some("clients"));
        assert_eq!(alter_column.table_name(), Some("customers"));
        assert_eq!(create_index.table_name(), Some("customers"));
        assert_eq!(drop_index.table_name(), Some("customers"));
        assert_eq!(add_foreign_key.table_name(), Some("orders"));
        assert_eq!(drop_foreign_key.table_name(), Some("orders"));
    }

    #[test]
    fn alter_column_retains_previous_and_next_shapes() {
        let previous = ColumnSnapshot::from(&CUSTOMER_COLUMNS[1]);
        let next = ColumnSnapshot::new(
            "email",
            SqlServerType::NVarChar,
            true,
            false,
            None,
            Some("'unknown'".to_string()),
            None,
            false,
            true,
            true,
            Some(255),
            None,
            None,
        );

        let operation = AlterColumn::new("sales", "customers", previous.clone(), next.clone());

        assert_eq!(operation.schema_name, "sales");
        assert_eq!(operation.table_name, "customers");
        assert_eq!(operation.previous, previous);
        assert_eq!(operation.next, next);
    }
}
