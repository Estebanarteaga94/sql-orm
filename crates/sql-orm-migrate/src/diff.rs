use crate::{
    AddColumn, AddForeignKey, AlterColumn, ColumnSnapshot, CreateIndex, CreateSchema, CreateTable,
    DropColumn, DropForeignKey, DropIndex, DropSchema, DropTable, ForeignKeySnapshot,
    IndexSnapshot, MigrationOperation, ModelSnapshot, RenameColumn, RenameTable, SchemaSnapshot,
    TableSnapshot,
};
use std::collections::{BTreeMap, BTreeSet};

/// Computes the minimum stage-7 diff for schema and table creation/deletion.
pub fn diff_schema_and_table_operations(
    previous: &ModelSnapshot,
    current: &ModelSnapshot,
) -> Vec<MigrationOperation> {
    let previous_schemas = schema_map(previous);
    let current_schemas = schema_map(current);
    let mut operations = Vec::new();

    for (schema_name, current_schema) in &current_schemas {
        if !previous_schemas.contains_key(schema_name) {
            operations.push(MigrationOperation::CreateSchema(CreateSchema::new(
                schema_name.clone(),
            )));

            for table in &current_schema.tables {
                operations.push(MigrationOperation::CreateTable(CreateTable::new(
                    schema_name.clone(),
                    table.clone(),
                )));
            }

            continue;
        }

        let previous_tables = table_map(previous_schemas[schema_name]);
        let current_tables = table_map(current_schema);
        let mut consumed_previous_tables = BTreeSet::new();

        for (table_name, current_table) in &current_tables {
            if let Some(renamed_from) = current_table
                .renamed_from
                .as_deref()
                .filter(|renamed_from| *renamed_from != table_name)
                .filter(|renamed_from| !current_tables.contains_key(*renamed_from))
                .filter(|_| !previous_tables.contains_key(table_name))
                && previous_tables.contains_key(renamed_from)
                && consumed_previous_tables.insert(renamed_from.to_string())
            {
                operations.push(MigrationOperation::RenameTable(RenameTable::new(
                    schema_name.clone(),
                    renamed_from.to_string(),
                    table_name.clone(),
                )));
                continue;
            }

            if previous_tables.contains_key(table_name) {
                consumed_previous_tables.insert(table_name.clone());
                continue;
            }

            if !previous_tables.contains_key(table_name) {
                operations.push(MigrationOperation::CreateTable(CreateTable::new(
                    schema_name.clone(),
                    (*current_table).clone(),
                )));
            }
        }

        for table_name in previous_tables.keys() {
            if !current_tables.contains_key(table_name)
                && !consumed_previous_tables.contains(table_name)
            {
                operations.push(MigrationOperation::DropTable(DropTable::new(
                    schema_name.clone(),
                    table_name.clone(),
                )));
            }
        }
    }

    for (schema_name, previous_schema) in &previous_schemas {
        if current_schemas.contains_key(schema_name) {
            continue;
        }

        let previous_tables = table_map(previous_schema);
        for table_name in previous_tables.keys() {
            operations.push(MigrationOperation::DropTable(DropTable::new(
                schema_name.clone(),
                table_name.clone(),
            )));
        }

        operations.push(MigrationOperation::DropSchema(DropSchema::new(
            schema_name.clone(),
        )));
    }

    operations
}

/// Computes additive/removal/basic-alteration column operations for tables present
/// in both snapshots. Table creation/deletion remains the responsibility of
/// `diff_schema_and_table_operations`.
pub fn diff_column_operations(
    previous: &ModelSnapshot,
    current: &ModelSnapshot,
) -> Vec<MigrationOperation> {
    let previous_schemas = schema_map(previous);
    let current_schemas = schema_map(current);
    let mut operations = Vec::new();

    for (schema_name, current_schema) in &current_schemas {
        let Some(previous_schema) = previous_schemas.get(schema_name) else {
            continue;
        };

        for (table_name, previous_table, current_table) in
            matched_table_pairs(previous_schema, current_schema)
        {
            let previous_columns = column_map(previous_table);
            let current_columns = column_map(current_table);
            let mut consumed_previous_columns = BTreeSet::new();

            for (column_name, current_column) in &current_columns {
                if let Some(renamed_from) = current_column
                    .renamed_from
                    .as_deref()
                    .filter(|renamed_from| *renamed_from != column_name)
                    .filter(|renamed_from| !current_columns.contains_key(*renamed_from))
                    && let Some(previous_column) = previous_columns.get(renamed_from)
                {
                    consumed_previous_columns.insert(renamed_from.to_string());
                    operations.push(MigrationOperation::RenameColumn(RenameColumn::new(
                        schema_name.clone(),
                        table_name.to_string(),
                        renamed_from.to_string(),
                        column_name.clone(),
                    )));
                    push_followup_column_change(
                        &mut operations,
                        schema_name,
                        &table_name,
                        renamed_previous_column(previous_column, current_column),
                        current_column,
                    );
                    continue;
                }

                match previous_columns.get(column_name) {
                    None => operations.push(MigrationOperation::AddColumn(AddColumn::new(
                        schema_name.clone(),
                        table_name.to_string(),
                        (*current_column).clone(),
                    ))),
                    Some(previous_column) => {
                        consumed_previous_columns.insert(column_name.clone());
                        push_followup_column_change(
                            &mut operations,
                            schema_name,
                            &table_name,
                            (*previous_column).clone(),
                            current_column,
                        );
                    }
                }
            }

            for column_name in previous_columns.keys() {
                if !consumed_previous_columns.contains(column_name)
                    && !current_columns.contains_key(column_name)
                {
                    operations.push(MigrationOperation::DropColumn(DropColumn::new(
                        schema_name.clone(),
                        table_name.clone(),
                        column_name.clone(),
                    )));
                }
            }
        }
    }

    operations
}

fn push_followup_column_change(
    operations: &mut Vec<MigrationOperation>,
    schema_name: &str,
    table_name: &str,
    previous_column: ColumnSnapshot,
    current_column: &ColumnSnapshot,
) {
    if columns_equal_for_diff(&previous_column, current_column) {
        return;
    }

    if requires_drop_and_add(&previous_column, current_column) {
        operations.push(MigrationOperation::DropColumn(DropColumn::new(
            schema_name.to_string(),
            table_name.to_string(),
            current_column.name.clone(),
        )));
        operations.push(MigrationOperation::AddColumn(AddColumn::new(
            schema_name.to_string(),
            table_name.to_string(),
            current_column.clone(),
        )));
    } else {
        operations.push(MigrationOperation::AlterColumn(AlterColumn::new(
            schema_name.to_string(),
            table_name.to_string(),
            previous_column,
            current_column.clone(),
        )));
    }
}

fn renamed_previous_column(previous: &ColumnSnapshot, current: &ColumnSnapshot) -> ColumnSnapshot {
    let mut renamed = previous.clone();
    renamed.name = current.name.clone();
    renamed
}

fn columns_equal_for_diff(previous: &ColumnSnapshot, current: &ColumnSnapshot) -> bool {
    let mut normalized_current = current.clone();
    normalized_current.renamed_from = None;
    *previous == normalized_current
}

fn requires_drop_and_add(previous: &ColumnSnapshot, current: &ColumnSnapshot) -> bool {
    previous.computed_sql != current.computed_sql
}

/// Computes additive/removal operations for indexes and foreign keys in tables
/// present in both snapshots. Table creation/deletion remains the responsibility
/// of `diff_schema_and_table_operations`.
pub fn diff_relational_operations(
    previous: &ModelSnapshot,
    current: &ModelSnapshot,
) -> Vec<MigrationOperation> {
    let previous_schemas = schema_map(previous);
    let current_schemas = schema_map(current);
    let mut operations = Vec::new();

    for (schema_name, current_schema) in &current_schemas {
        let Some(previous_schema) = previous_schemas.get(schema_name) else {
            for table in &current_schema.tables {
                push_create_relational_operations(&mut operations, schema_name, table);
            }
            continue;
        };

        let previous_tables = table_map(previous_schema);
        let current_tables = table_map(current_schema);

        for (table_name, previous_table, current_table) in
            matched_table_pairs(previous_schema, current_schema)
        {
            let previous_indexes = index_map(previous_table);
            let current_indexes = index_map(current_table);

            for (index_name, index) in &current_indexes {
                match previous_indexes.get(index_name) {
                    None => operations.push(MigrationOperation::CreateIndex(CreateIndex::new(
                        schema_name.clone(),
                        table_name.to_string(),
                        (*index).clone(),
                    ))),
                    Some(previous_index) if *previous_index != *index => {
                        operations.push(MigrationOperation::DropIndex(DropIndex::new(
                            schema_name.clone(),
                            table_name.to_string(),
                            index_name.clone(),
                        )));
                        operations.push(MigrationOperation::CreateIndex(CreateIndex::new(
                            schema_name.clone(),
                            table_name.to_string(),
                            (*index).clone(),
                        )));
                    }
                    Some(_) => {}
                }
            }

            for index_name in previous_indexes.keys() {
                if !current_indexes.contains_key(index_name) {
                    operations.push(MigrationOperation::DropIndex(DropIndex::new(
                        schema_name.clone(),
                        table_name.to_string(),
                        index_name.clone(),
                    )));
                }
            }

            let previous_foreign_keys = foreign_key_map(previous_table);
            let current_foreign_keys = foreign_key_map(current_table);

            for (foreign_key_name, foreign_key) in &current_foreign_keys {
                match previous_foreign_keys.get(foreign_key_name) {
                    None => operations.push(MigrationOperation::AddForeignKey(AddForeignKey::new(
                        schema_name.clone(),
                        table_name.to_string(),
                        (*foreign_key).clone(),
                    ))),
                    Some(previous_foreign_key) if *previous_foreign_key != *foreign_key => {
                        operations.push(MigrationOperation::DropForeignKey(DropForeignKey::new(
                            schema_name.clone(),
                            table_name.to_string(),
                            foreign_key_name.clone(),
                        )));
                        operations.push(MigrationOperation::AddForeignKey(AddForeignKey::new(
                            schema_name.clone(),
                            table_name.to_string(),
                            (*foreign_key).clone(),
                        )));
                    }
                    Some(_) => {}
                }
            }

            for foreign_key_name in previous_foreign_keys.keys() {
                if !current_foreign_keys.contains_key(foreign_key_name) {
                    operations.push(MigrationOperation::DropForeignKey(DropForeignKey::new(
                        schema_name.clone(),
                        table_name.to_string(),
                        foreign_key_name.clone(),
                    )));
                }
            }
        }

        for (table_name, current_table) in current_tables {
            if previous_tables.contains_key(&table_name) {
                continue;
            }

            if current_table
                .renamed_from
                .as_deref()
                .is_some_and(|renamed_from| previous_tables.contains_key(renamed_from))
            {
                continue;
            }

            push_create_relational_operations(&mut operations, schema_name, current_table);
        }
    }

    operations
}

fn push_create_relational_operations(
    operations: &mut Vec<MigrationOperation>,
    schema_name: &str,
    table: &TableSnapshot,
) {
    for index in &table.indexes {
        operations.push(MigrationOperation::CreateIndex(CreateIndex::new(
            schema_name.to_string(),
            table.name.clone(),
            index.clone(),
        )));
    }

    for foreign_key in &table.foreign_keys {
        operations.push(MigrationOperation::AddForeignKey(AddForeignKey::new(
            schema_name.to_string(),
            table.name.clone(),
            foreign_key.clone(),
        )));
    }
}

fn schema_map(snapshot: &ModelSnapshot) -> BTreeMap<String, &SchemaSnapshot> {
    snapshot
        .schemas
        .iter()
        .map(|schema| (schema.name.clone(), schema))
        .collect()
}

fn table_map(schema: &SchemaSnapshot) -> BTreeMap<String, &TableSnapshot> {
    schema
        .tables
        .iter()
        .map(|table| (table.name.clone(), table))
        .collect()
}

fn matched_table_pairs<'a>(
    previous_schema: &'a SchemaSnapshot,
    current_schema: &'a SchemaSnapshot,
) -> Vec<(String, &'a TableSnapshot, &'a TableSnapshot)> {
    let previous_tables = table_map(previous_schema);
    let current_tables = table_map(current_schema);
    let mut consumed_previous_tables = BTreeSet::new();
    let mut pairs = Vec::new();

    for (table_name, current_table) in &current_tables {
        if let Some(previous_table) = previous_tables.get(table_name) {
            consumed_previous_tables.insert(table_name.clone());
            pairs.push((table_name.clone(), *previous_table, *current_table));
            continue;
        }

        if let Some(renamed_from) = current_table
            .renamed_from
            .as_deref()
            .filter(|renamed_from| *renamed_from != table_name)
            .filter(|renamed_from| !current_tables.contains_key(*renamed_from))
            && let Some(previous_table) = previous_tables.get(renamed_from)
            && consumed_previous_tables.insert(renamed_from.to_string())
        {
            pairs.push((table_name.clone(), *previous_table, *current_table));
        }
    }

    pairs
}

fn column_map(table: &TableSnapshot) -> BTreeMap<String, &ColumnSnapshot> {
    table
        .columns
        .iter()
        .map(|column| (column.name.clone(), column))
        .collect()
}

fn index_map(table: &TableSnapshot) -> BTreeMap<String, &IndexSnapshot> {
    table
        .indexes
        .iter()
        .map(|index| (index.name.clone(), index))
        .collect()
}

fn foreign_key_map(table: &TableSnapshot) -> BTreeMap<String, &ForeignKeySnapshot> {
    table
        .foreign_keys
        .iter()
        .map(|foreign_key| (foreign_key.name.clone(), foreign_key))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        diff_column_operations, diff_relational_operations, diff_schema_and_table_operations,
    };
    use crate::{
        AddColumn, AddForeignKey, AlterColumn, ColumnSnapshot, CreateIndex, CreateSchema,
        CreateTable, DropColumn, DropForeignKey, DropIndex, DropSchema, DropTable,
        ForeignKeySnapshot, IndexColumnSnapshot, IndexSnapshot, MigrationOperation, ModelSnapshot,
        RenameColumn, RenameTable, SchemaSnapshot, TableSnapshot,
    };
    use sql_orm_core::{IdentityMetadata, ReferentialAction, SqlServerType};

    fn column(
        name: &str,
        sql_type: SqlServerType,
        nullable: bool,
        max_length: Option<u32>,
    ) -> ColumnSnapshot {
        ColumnSnapshot::new(
            name,
            sql_type,
            nullable,
            name == "id",
            (name == "id").then(|| IdentityMetadata::new(1, 1)),
            None,
            None,
            name == "version",
            name != "id" && name != "version",
            name != "id" && name != "version",
            max_length,
            None,
            None,
        )
    }

    fn table(
        name: &str,
        columns: Vec<ColumnSnapshot>,
        indexes: Vec<IndexSnapshot>,
        foreign_keys: Vec<ForeignKeySnapshot>,
    ) -> TableSnapshot {
        TableSnapshot::new(
            name,
            columns,
            Some(format!("pk_{name}")),
            vec!["id".to_string()],
            indexes,
            foreign_keys,
        )
    }

    fn schema(name: &str, tables: Vec<TableSnapshot>) -> SchemaSnapshot {
        SchemaSnapshot::new(name, tables)
    }

    fn foreign_key(name: &str, schema: &str, table: &str, column: &str) -> ForeignKeySnapshot {
        ForeignKeySnapshot::new(
            name,
            vec![column.to_string()],
            schema,
            table,
            vec!["id".to_string()],
            ReferentialAction::NoAction,
            ReferentialAction::NoAction,
        )
    }

    fn composite_foreign_key(
        name: &str,
        schema: &str,
        table: &str,
        columns: &[&str],
        referenced_columns: &[&str],
        on_delete: ReferentialAction,
        on_update: ReferentialAction,
    ) -> ForeignKeySnapshot {
        ForeignKeySnapshot::new(
            name,
            columns.iter().map(|column| (*column).to_string()).collect(),
            schema,
            table,
            referenced_columns
                .iter()
                .map(|column| (*column).to_string())
                .collect(),
            on_delete,
            on_update,
        )
    }

    #[test]
    fn schema_and_table_diff_keeps_safe_operation_order() {
        let previous = ModelSnapshot::new(vec![
            schema("legacy", vec![table("old_orders", vec![], vec![], vec![])]),
            schema("sales", vec![table("orders", vec![], vec![], vec![])]),
        ]);
        let current = ModelSnapshot::new(vec![
            schema(
                "reporting",
                vec![table("daily_sales", vec![], vec![], vec![])],
            ),
            schema("sales", vec![table("orders", vec![], vec![], vec![])]),
        ]);

        let operations = diff_schema_and_table_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::CreateSchema(CreateSchema::new("reporting")),
                MigrationOperation::CreateTable(CreateTable::new(
                    "reporting",
                    table("daily_sales", vec![], vec![], vec![]),
                )),
                MigrationOperation::DropTable(DropTable::new("legacy", "old_orders")),
                MigrationOperation::DropSchema(DropSchema::new("legacy")),
            ]
        );
    }

    #[test]
    fn schema_and_table_diff_detects_table_creation_and_deletion_in_existing_schema() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![
                table("customers", vec![], vec![], vec![]),
                table("orders", vec![], vec![], vec![]),
            ],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![
                table("customers", vec![], vec![], vec![]),
                table("invoices", vec![], vec![], vec![]),
            ],
        )]);

        let operations = diff_schema_and_table_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::CreateTable(CreateTable::new(
                    "sales",
                    table("invoices", vec![], vec![], vec![]),
                )),
                MigrationOperation::DropTable(DropTable::new("sales", "orders")),
            ]
        );
    }

    #[test]
    fn schema_and_table_diff_returns_empty_for_equal_snapshots() {
        let snapshot = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![column("id", SqlServerType::BigInt, false, None)],
                vec![],
                vec![],
            )],
        )]);

        let operations = diff_schema_and_table_operations(&snapshot, &snapshot);

        assert!(operations.is_empty());
    }

    #[test]
    fn schema_and_table_diff_emits_explicit_table_rename_without_drop_and_add() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table("customers", vec![], vec![], vec![])],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table("clients", vec![], vec![], vec![]).with_renamed_from("customers")],
        )]);

        let operations = diff_schema_and_table_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![MigrationOperation::RenameTable(RenameTable::new(
                "sales",
                "customers",
                "clients",
            ))]
        );
    }

    #[test]
    fn column_diff_detects_add_and_drop_in_shared_table() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![
                    column("id", SqlServerType::BigInt, false, None),
                    column("email", SqlServerType::NVarChar, false, Some(160)),
                ],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![
                    column("id", SqlServerType::BigInt, false, None),
                    column("version", SqlServerType::RowVersion, false, None),
                ],
                vec![],
                vec![],
            )],
        )]);

        let operations = diff_column_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::AddColumn(AddColumn::new(
                    "sales",
                    "customers",
                    column("version", SqlServerType::RowVersion, false, None),
                )),
                MigrationOperation::DropColumn(DropColumn::new("sales", "customers", "email")),
            ]
        );
    }

    #[test]
    fn column_diff_detects_basic_alterations() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![column("email", SqlServerType::NVarChar, false, Some(160))],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![column("email", SqlServerType::NVarChar, true, Some(255))],
                vec![],
                vec![],
            )],
        )]);

        let operations = diff_column_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![MigrationOperation::AlterColumn(AlterColumn::new(
                "sales",
                "customers",
                column("email", SqlServerType::NVarChar, false, Some(160)),
                column("email", SqlServerType::NVarChar, true, Some(255)),
            ))]
        );
    }

    #[test]
    fn column_diff_renames_column_when_explicit_hint_matches_previous_name() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![column("email", SqlServerType::NVarChar, false, Some(160))],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![
                    column("email_address", SqlServerType::NVarChar, false, Some(160))
                        .with_renamed_from("email"),
                ],
                vec![],
                vec![],
            )],
        )]);

        let operations = diff_column_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![MigrationOperation::RenameColumn(RenameColumn::new(
                "sales",
                "customers",
                "email",
                "email_address",
            ))]
        );
    }

    #[test]
    fn column_diff_renames_then_alters_column_when_shape_changes() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![column("email", SqlServerType::NVarChar, false, Some(160))],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![
                    column("email_address", SqlServerType::NVarChar, true, Some(255))
                        .with_renamed_from("email"),
                ],
                vec![],
                vec![],
            )],
        )]);

        let operations = diff_column_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::RenameColumn(RenameColumn::new(
                    "sales",
                    "customers",
                    "email",
                    "email_address",
                )),
                MigrationOperation::AlterColumn(AlterColumn::new(
                    "sales",
                    "customers",
                    column("email_address", SqlServerType::NVarChar, false, Some(160)),
                    column("email_address", SqlServerType::NVarChar, true, Some(255))
                        .with_renamed_from("email"),
                )),
            ]
        );
    }

    #[test]
    fn column_diff_uses_renamed_table_as_shared_context() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![column("id", SqlServerType::BigInt, false, None)],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![
                table(
                    "clients",
                    vec![
                        column("id", SqlServerType::BigInt, false, None),
                        column("email", SqlServerType::NVarChar, false, Some(180)),
                    ],
                    vec![],
                    vec![],
                )
                .with_renamed_from("customers"),
            ],
        )]);

        let operations = diff_column_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![MigrationOperation::AddColumn(AddColumn::new(
                "sales",
                "clients",
                column("email", SqlServerType::NVarChar, false, Some(180)),
            ))]
        );
    }

    #[test]
    fn column_diff_recreates_column_when_computed_expression_changes() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "order_lines",
                vec![ColumnSnapshot::new(
                    "line_total",
                    SqlServerType::Decimal,
                    false,
                    false,
                    None,
                    None,
                    Some("[unit_price] * [quantity]".to_string()),
                    false,
                    false,
                    false,
                    None,
                    Some(18),
                    Some(2),
                )],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "order_lines",
                vec![ColumnSnapshot::new(
                    "line_total",
                    SqlServerType::Decimal,
                    false,
                    false,
                    None,
                    None,
                    Some("[unit_price] * [quantity] * (1 - [discount])".to_string()),
                    false,
                    false,
                    false,
                    None,
                    Some(18),
                    Some(2),
                )],
                vec![],
                vec![],
            )],
        )]);

        let operations = diff_column_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::DropColumn(DropColumn::new(
                    "sales",
                    "order_lines",
                    "line_total",
                )),
                MigrationOperation::AddColumn(AddColumn::new(
                    "sales",
                    "order_lines",
                    ColumnSnapshot::new(
                        "line_total",
                        SqlServerType::Decimal,
                        false,
                        false,
                        None,
                        None,
                        Some("[unit_price] * [quantity] * (1 - [discount])".to_string()),
                        false,
                        false,
                        false,
                        None,
                        Some(18),
                        Some(2),
                    ),
                )),
            ]
        );
    }

    #[test]
    fn column_diff_recreates_column_when_switching_between_regular_and_computed() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "order_lines",
                vec![ColumnSnapshot::new(
                    "line_total",
                    SqlServerType::Decimal,
                    false,
                    false,
                    None,
                    None,
                    None,
                    false,
                    true,
                    true,
                    None,
                    Some(18),
                    Some(2),
                )],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "order_lines",
                vec![ColumnSnapshot::new(
                    "line_total",
                    SqlServerType::Decimal,
                    false,
                    false,
                    None,
                    None,
                    Some("[unit_price] * [quantity]".to_string()),
                    false,
                    false,
                    false,
                    None,
                    Some(18),
                    Some(2),
                )],
                vec![],
                vec![],
            )],
        )]);

        let operations = diff_column_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::DropColumn(DropColumn::new(
                    "sales",
                    "order_lines",
                    "line_total",
                )),
                MigrationOperation::AddColumn(AddColumn::new(
                    "sales",
                    "order_lines",
                    ColumnSnapshot::new(
                        "line_total",
                        SqlServerType::Decimal,
                        false,
                        false,
                        None,
                        None,
                        Some("[unit_price] * [quantity]".to_string()),
                        false,
                        false,
                        false,
                        None,
                        Some(18),
                        Some(2),
                    ),
                )),
            ]
        );
    }

    #[test]
    fn column_diff_ignores_tables_handled_by_table_diff() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![column("email", SqlServerType::NVarChar, false, Some(160))],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "orders",
                vec![column("customer_id", SqlServerType::BigInt, false, None)],
                vec![],
                vec![],
            )],
        )]);

        let operations = diff_column_operations(&previous, &current);

        assert!(operations.is_empty());
    }

    #[test]
    fn relational_diff_detects_index_and_foreign_key_additions_and_removals() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "orders",
                vec![column("customer_id", SqlServerType::BigInt, false, None)],
                vec![IndexSnapshot::new(
                    "ix_orders_customer_id",
                    vec![IndexColumnSnapshot::asc("customer_id")],
                    false,
                )],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "orders",
                vec![column("customer_id", SqlServerType::BigInt, false, None)],
                vec![],
                vec![foreign_key(
                    "fk_orders_customer_id_customers",
                    "sales",
                    "customers",
                    "customer_id",
                )],
            )],
        )]);

        let operations = diff_relational_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::DropIndex(DropIndex::new(
                    "sales",
                    "orders",
                    "ix_orders_customer_id",
                )),
                MigrationOperation::AddForeignKey(AddForeignKey::new(
                    "sales",
                    "orders",
                    foreign_key(
                        "fk_orders_customer_id_customers",
                        "sales",
                        "customers",
                        "customer_id",
                    ),
                )),
            ]
        );
    }

    #[test]
    fn relational_diff_emits_indexes_and_foreign_keys_for_new_tables() {
        let previous = ModelSnapshot::new(vec![schema("sales", vec![])]);
        let current = ModelSnapshot::new(vec![
            schema(
                "analytics",
                vec![table(
                    "daily_sales",
                    vec![column("customer_id", SqlServerType::BigInt, false, None)],
                    vec![IndexSnapshot::new(
                        "ix_daily_sales_customer_id",
                        vec![IndexColumnSnapshot::asc("customer_id")],
                        false,
                    )],
                    vec![],
                )],
            ),
            schema(
                "sales",
                vec![table(
                    "orders",
                    vec![column("customer_id", SqlServerType::BigInt, false, None)],
                    vec![IndexSnapshot::new(
                        "ix_orders_customer_id",
                        vec![IndexColumnSnapshot::asc("customer_id")],
                        false,
                    )],
                    vec![foreign_key(
                        "fk_orders_customer_id_customers",
                        "sales",
                        "customers",
                        "customer_id",
                    )],
                )],
            ),
        ]);

        let operations = diff_relational_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::CreateIndex(CreateIndex::new(
                    "analytics",
                    "daily_sales",
                    IndexSnapshot::new(
                        "ix_daily_sales_customer_id",
                        vec![IndexColumnSnapshot::asc("customer_id")],
                        false,
                    ),
                )),
                MigrationOperation::CreateIndex(CreateIndex::new(
                    "sales",
                    "orders",
                    IndexSnapshot::new(
                        "ix_orders_customer_id",
                        vec![IndexColumnSnapshot::asc("customer_id")],
                        false,
                    ),
                )),
                MigrationOperation::AddForeignKey(AddForeignKey::new(
                    "sales",
                    "orders",
                    foreign_key(
                        "fk_orders_customer_id_customers",
                        "sales",
                        "customers",
                        "customer_id",
                    ),
                )),
            ]
        );
    }

    #[test]
    fn relational_diff_recreates_foreign_key_when_definition_changes() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "orders",
                vec![column("customer_id", SqlServerType::BigInt, false, None)],
                vec![],
                vec![foreign_key(
                    "fk_orders_customer_id_customers",
                    "dbo",
                    "customers",
                    "customer_id",
                )],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "orders",
                vec![column("customer_id", SqlServerType::BigInt, false, None)],
                vec![IndexSnapshot::new(
                    "ix_orders_customer_id",
                    vec![IndexColumnSnapshot::asc("customer_id")],
                    false,
                )],
                vec![foreign_key(
                    "fk_orders_customer_id_customers",
                    "sales",
                    "customers",
                    "customer_id",
                )],
            )],
        )]);

        let operations = diff_relational_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::CreateIndex(CreateIndex::new(
                    "sales",
                    "orders",
                    IndexSnapshot::new(
                        "ix_orders_customer_id",
                        vec![IndexColumnSnapshot::asc("customer_id")],
                        false,
                    ),
                )),
                MigrationOperation::DropForeignKey(DropForeignKey::new(
                    "sales",
                    "orders",
                    "fk_orders_customer_id_customers",
                )),
                MigrationOperation::AddForeignKey(AddForeignKey::new(
                    "sales",
                    "orders",
                    foreign_key(
                        "fk_orders_customer_id_customers",
                        "sales",
                        "customers",
                        "customer_id",
                    ),
                )),
            ]
        );
    }

    #[test]
    fn relational_diff_recreates_composite_foreign_key_when_columns_or_actions_change() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "order_allocations",
                vec![
                    column("customer_id", SqlServerType::BigInt, false, None),
                    column("branch_id", SqlServerType::BigInt, false, None),
                ],
                vec![],
                vec![composite_foreign_key(
                    "fk_order_allocations_customer_branch_customers",
                    "sales",
                    "customers",
                    &["customer_id", "branch_id"],
                    &["id", "branch_id"],
                    ReferentialAction::NoAction,
                    ReferentialAction::NoAction,
                )],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "order_allocations",
                vec![
                    column("customer_id", SqlServerType::BigInt, false, None),
                    column("branch_id", SqlServerType::BigInt, false, None),
                ],
                vec![],
                vec![composite_foreign_key(
                    "fk_order_allocations_customer_branch_customers",
                    "sales",
                    "customers",
                    &["customer_id", "branch_id"],
                    &["id", "branch_id"],
                    ReferentialAction::SetDefault,
                    ReferentialAction::Cascade,
                )],
            )],
        )]);

        let operations = diff_relational_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::DropForeignKey(DropForeignKey::new(
                    "sales",
                    "order_allocations",
                    "fk_order_allocations_customer_branch_customers",
                )),
                MigrationOperation::AddForeignKey(AddForeignKey::new(
                    "sales",
                    "order_allocations",
                    composite_foreign_key(
                        "fk_order_allocations_customer_branch_customers",
                        "sales",
                        "customers",
                        &["customer_id", "branch_id"],
                        &["id", "branch_id"],
                        ReferentialAction::SetDefault,
                        ReferentialAction::Cascade,
                    ),
                )),
            ]
        );
    }

    #[test]
    fn relational_diff_recreates_index_when_composite_definition_changes() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "orders",
                vec![
                    column("customer_id", SqlServerType::BigInt, false, None),
                    column("total_cents", SqlServerType::BigInt, false, None),
                ],
                vec![IndexSnapshot::new(
                    "ix_orders_customer_total",
                    vec![IndexColumnSnapshot::asc("customer_id")],
                    false,
                )],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "orders",
                vec![
                    column("customer_id", SqlServerType::BigInt, false, None),
                    column("total_cents", SqlServerType::BigInt, false, None),
                ],
                vec![IndexSnapshot::new(
                    "ix_orders_customer_total",
                    vec![
                        IndexColumnSnapshot::asc("customer_id"),
                        IndexColumnSnapshot::desc("total_cents"),
                    ],
                    false,
                )],
                vec![],
            )],
        )]);

        let operations = diff_relational_operations(&previous, &current);

        assert_eq!(
            operations,
            vec![
                MigrationOperation::DropIndex(DropIndex::new(
                    "sales",
                    "orders",
                    "ix_orders_customer_total",
                )),
                MigrationOperation::CreateIndex(CreateIndex::new(
                    "sales",
                    "orders",
                    IndexSnapshot::new(
                        "ix_orders_customer_total",
                        vec![
                            IndexColumnSnapshot::asc("customer_id"),
                            IndexColumnSnapshot::desc("total_cents"),
                        ],
                        false,
                    ),
                )),
            ]
        );
    }

    #[test]
    fn full_diff_on_minimal_snapshots_is_stable_when_combined() {
        let previous = ModelSnapshot::new(vec![schema(
            "sales",
            vec![table(
                "customers",
                vec![
                    column("id", SqlServerType::BigInt, false, None),
                    column("email", SqlServerType::NVarChar, false, Some(160)),
                ],
                vec![],
                vec![],
            )],
        )]);
        let current = ModelSnapshot::new(vec![
            schema(
                "reporting",
                vec![table(
                    "daily_sales",
                    vec![column("id", SqlServerType::BigInt, false, None)],
                    vec![],
                    vec![],
                )],
            ),
            schema(
                "sales",
                vec![
                    table(
                        "customers",
                        vec![
                            column("id", SqlServerType::BigInt, false, None),
                            column("email", SqlServerType::NVarChar, true, Some(255)),
                            column("version", SqlServerType::RowVersion, false, None),
                        ],
                        vec![IndexSnapshot::new(
                            "ix_customers_email",
                            vec![IndexColumnSnapshot::asc("email")],
                            true,
                        )],
                        vec![],
                    ),
                    table(
                        "orders",
                        vec![
                            column("id", SqlServerType::BigInt, false, None),
                            column("customer_id", SqlServerType::BigInt, false, None),
                        ],
                        vec![IndexSnapshot::new(
                            "ix_orders_customer_id",
                            vec![IndexColumnSnapshot::asc("customer_id")],
                            false,
                        )],
                        vec![foreign_key(
                            "fk_orders_customer_id_customers",
                            "sales",
                            "customers",
                            "customer_id",
                        )],
                    ),
                ],
            ),
        ]);

        let mut operations = diff_schema_and_table_operations(&previous, &current);
        operations.extend(diff_column_operations(&previous, &current));
        operations.extend(diff_relational_operations(&previous, &current));

        assert_eq!(
            operations,
            vec![
                MigrationOperation::CreateSchema(CreateSchema::new("reporting")),
                MigrationOperation::CreateTable(CreateTable::new(
                    "reporting",
                    table(
                        "daily_sales",
                        vec![column("id", SqlServerType::BigInt, false, None)],
                        vec![],
                        vec![],
                    ),
                )),
                MigrationOperation::CreateTable(CreateTable::new(
                    "sales",
                    table(
                        "orders",
                        vec![
                            column("id", SqlServerType::BigInt, false, None),
                            column("customer_id", SqlServerType::BigInt, false, None),
                        ],
                        vec![IndexSnapshot::new(
                            "ix_orders_customer_id",
                            vec![IndexColumnSnapshot::asc("customer_id")],
                            false,
                        )],
                        vec![foreign_key(
                            "fk_orders_customer_id_customers",
                            "sales",
                            "customers",
                            "customer_id",
                        )],
                    ),
                )),
                MigrationOperation::AlterColumn(AlterColumn::new(
                    "sales",
                    "customers",
                    column("email", SqlServerType::NVarChar, false, Some(160)),
                    column("email", SqlServerType::NVarChar, true, Some(255)),
                )),
                MigrationOperation::AddColumn(AddColumn::new(
                    "sales",
                    "customers",
                    column("version", SqlServerType::RowVersion, false, None),
                )),
                MigrationOperation::CreateIndex(CreateIndex::new(
                    "sales",
                    "customers",
                    IndexSnapshot::new(
                        "ix_customers_email",
                        vec![IndexColumnSnapshot::asc("email")],
                        true,
                    ),
                )),
                MigrationOperation::CreateIndex(CreateIndex::new(
                    "sales",
                    "orders",
                    IndexSnapshot::new(
                        "ix_orders_customer_id",
                        vec![IndexColumnSnapshot::asc("customer_id")],
                        false,
                    ),
                )),
                MigrationOperation::AddForeignKey(AddForeignKey::new(
                    "sales",
                    "orders",
                    foreign_key(
                        "fk_orders_customer_id_customers",
                        "sales",
                        "customers",
                        "customer_id",
                    ),
                )),
            ]
        );
    }
}
