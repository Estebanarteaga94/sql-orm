use crate::quoting::{quote_identifier, quote_qualified_identifier};
use sql_orm_core::{OrmError, ReferentialAction, SqlServerType, quote_sql_string_literal};
use sql_orm_migrate::{
    AddColumn, AddForeignKey, AlterColumn, ColumnSnapshot, CreateIndex, CreateSchema, CreateTable,
    DropColumn, DropForeignKey, DropIndex, DropSchema, DropTable, IndexColumnSnapshot,
    MigrationOperation, RenameColumn, RenameTable,
};

const MIGRATIONS_HISTORY_SCHEMA: &str = "dbo";
const MIGRATIONS_HISTORY_TABLE: &str = "__sql_orm_migrations";

impl crate::SqlServerCompiler {
    pub fn compile_migration_operations(
        operations: &[MigrationOperation],
    ) -> Result<Vec<String>, OrmError> {
        operations.iter().map(compile_operation).collect()
    }

    pub fn compile_migrations_history_table() -> Result<String, OrmError> {
        let table =
            quote_qualified_identifier(MIGRATIONS_HISTORY_SCHEMA, MIGRATIONS_HISTORY_TABLE)?;

        let object_name = quote_sql_string_literal(&format!(
            "{MIGRATIONS_HISTORY_SCHEMA}.{MIGRATIONS_HISTORY_TABLE}"
        ));
        Ok(format!(
            "IF OBJECT_ID({object_name}, N'U') IS NULL\nBEGIN\n    CREATE TABLE {table} (\n        [id] nvarchar(150) NOT NULL PRIMARY KEY,\n        [name] nvarchar(255) NOT NULL,\n        [applied_at] datetime2 NOT NULL DEFAULT SYSUTCDATETIME(),\n        [checksum] nvarchar(128) NOT NULL,\n        [orm_version] nvarchar(50) NOT NULL\n    );\nEND",
        ))
    }
}

fn compile_operation(operation: &MigrationOperation) -> Result<String, OrmError> {
    match operation {
        MigrationOperation::CreateSchema(operation) => compile_create_schema(operation),
        MigrationOperation::DropSchema(operation) => compile_drop_schema(operation),
        MigrationOperation::CreateTable(operation) => compile_create_table(operation),
        MigrationOperation::DropTable(operation) => compile_drop_table(operation),
        MigrationOperation::RenameTable(operation) => compile_rename_table(operation),
        MigrationOperation::RenameColumn(operation) => compile_rename_column(operation),
        MigrationOperation::AddColumn(operation) => compile_add_column(operation),
        MigrationOperation::DropColumn(operation) => compile_drop_column(operation),
        MigrationOperation::AlterColumn(operation) => compile_alter_column(operation),
        MigrationOperation::CreateIndex(operation) => compile_create_index(operation),
        MigrationOperation::DropIndex(operation) => compile_drop_index(operation),
        MigrationOperation::AddForeignKey(operation) => compile_add_foreign_key(operation),
        MigrationOperation::DropForeignKey(operation) => compile_drop_foreign_key(operation),
    }
}

fn compile_create_schema(operation: &CreateSchema) -> Result<String, OrmError> {
    let schema = crate::quote_identifier(&operation.schema_name)?;
    let schema_name = quote_sql_string_literal(&operation.schema_name);
    let create_schema = quote_sql_string_literal(&format!("CREATE SCHEMA {schema}"));
    Ok(format!(
        "IF SCHEMA_ID({schema_name}) IS NULL EXEC({create_schema})",
    ))
}

fn compile_drop_schema(operation: &DropSchema) -> Result<String, OrmError> {
    Ok(format!(
        "DROP SCHEMA {}",
        quote_identifier(&operation.schema_name)?
    ))
}

fn compile_create_table(operation: &CreateTable) -> Result<String, OrmError> {
    let table_name = quote_qualified_identifier(&operation.schema_name, &operation.table.name)?;
    let mut definitions = operation
        .table
        .columns
        .iter()
        .map(compile_column_definition)
        .collect::<Result<Vec<_>, _>>()?;

    if !operation.table.primary_key_columns.is_empty() {
        let columns = operation
            .table
            .primary_key_columns
            .iter()
            .map(|column| crate::quote_identifier(column))
            .collect::<Result<Vec<_>, _>>()?
            .join(", ");

        definitions.push(match &operation.table.primary_key_name {
            Some(name) => format!(
                "CONSTRAINT {} PRIMARY KEY ({columns})",
                crate::quote_identifier(name)?
            ),
            None => format!("PRIMARY KEY ({columns})"),
        });
    }

    Ok(format!(
        "CREATE TABLE {table_name} (\n    {}\n)",
        definitions.join(",\n    ")
    ))
}

fn compile_drop_table(operation: &DropTable) -> Result<String, OrmError> {
    Ok(format!(
        "DROP TABLE {}",
        quote_qualified_identifier(&operation.schema_name, &operation.table_name)?
    ))
}

fn compile_add_column(operation: &AddColumn) -> Result<String, OrmError> {
    Ok(format!(
        "ALTER TABLE {} ADD {}",
        quote_qualified_identifier(&operation.schema_name, &operation.table_name)?,
        compile_column_definition(&operation.column)?,
    ))
}

fn compile_rename_table(operation: &RenameTable) -> Result<String, OrmError> {
    let qualified_table =
        quote_qualified_identifier(&operation.schema_name, &operation.previous_table_name)?;
    let qualified_table = quote_sql_string_literal(&qualified_table);
    let next_name = quote_sql_string_literal(&operation.next_table_name);

    Ok(format!(
        "EXEC sp_rename {qualified_table}, {next_name}, N'OBJECT'",
    ))
}

fn compile_rename_column(operation: &RenameColumn) -> Result<String, OrmError> {
    let qualified_column = format!(
        "{}.{}",
        quote_qualified_identifier(&operation.schema_name, &operation.table_name)?,
        quote_identifier(&operation.previous_column_name)?,
    );
    let qualified_column = quote_sql_string_literal(&qualified_column);
    let next_name = quote_sql_string_literal(&operation.next_column_name);

    Ok(format!(
        "EXEC sp_rename {qualified_column}, {next_name}, N'COLUMN'",
    ))
}

fn compile_drop_column(operation: &DropColumn) -> Result<String, OrmError> {
    Ok(format!(
        "ALTER TABLE {} DROP COLUMN {}",
        quote_qualified_identifier(&operation.schema_name, &operation.table_name)?,
        crate::quote_identifier(&operation.column_name)?,
    ))
}

fn compile_alter_column(operation: &AlterColumn) -> Result<String, OrmError> {
    if operation.previous.name != operation.next.name {
        return Err(OrmError::new(
            "SQL Server alter column compilation does not support renaming columns",
        ));
    }

    if operation.previous.default_sql != operation.next.default_sql
        || operation.previous.computed_sql != operation.next.computed_sql
        || operation.previous.identity != operation.next.identity
        || operation.previous.primary_key != operation.next.primary_key
        || operation.previous.rowversion != operation.next.rowversion
    {
        return Err(OrmError::new(
            "SQL Server alter column compilation only supports type and nullability changes in this stage",
        ));
    }

    Ok(format!(
        "ALTER TABLE {} ALTER COLUMN {}",
        quote_qualified_identifier(&operation.schema_name, &operation.table_name)?,
        compile_alter_column_definition(&operation.next)?,
    ))
}

fn compile_create_index(operation: &CreateIndex) -> Result<String, OrmError> {
    if operation.index.columns.is_empty() {
        return Err(OrmError::new(
            "SQL Server index migration compilation requires at least one indexed column",
        ));
    }

    let index_name = quote_identifier(&operation.index.name)?;
    let table = quote_qualified_identifier(&operation.schema_name, &operation.table_name)?;
    let columns = operation
        .index
        .columns
        .iter()
        .map(compile_index_column)
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let unique = if operation.index.unique {
        "UNIQUE "
    } else {
        ""
    };

    Ok(format!(
        "CREATE {unique}INDEX {index_name} ON {table} ({columns})"
    ))
}

fn compile_drop_index(operation: &DropIndex) -> Result<String, OrmError> {
    Ok(format!(
        "DROP INDEX {} ON {}",
        quote_identifier(&operation.index_name)?,
        quote_qualified_identifier(&operation.schema_name, &operation.table_name)?,
    ))
}

fn compile_add_foreign_key(operation: &AddForeignKey) -> Result<String, OrmError> {
    if operation.foreign_key.columns.is_empty() {
        return Err(OrmError::new(
            "SQL Server foreign key migration compilation requires at least one local column",
        ));
    }

    if operation.foreign_key.referenced_columns.is_empty() {
        return Err(OrmError::new(
            "SQL Server foreign key migration compilation requires at least one referenced column",
        ));
    }

    if operation.foreign_key.columns.len() != operation.foreign_key.referenced_columns.len() {
        return Err(OrmError::new(
            "SQL Server foreign key migration compilation requires the same number of local and referenced columns",
        ));
    }

    let table = quote_qualified_identifier(&operation.schema_name, &operation.table_name)?;
    let constraint = quote_identifier(&operation.foreign_key.name)?;
    let columns = operation
        .foreign_key
        .columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let referenced_table = quote_qualified_identifier(
        &operation.foreign_key.referenced_schema,
        &operation.foreign_key.referenced_table,
    )?;
    let referenced_columns = operation
        .foreign_key
        .referenced_columns
        .iter()
        .map(|column| quote_identifier(column))
        .collect::<Result<Vec<_>, _>>()?
        .join(", ");
    let on_delete = render_foreign_key_action_clause("DELETE", operation.foreign_key.on_delete)?;
    let on_update = render_foreign_key_action_clause("UPDATE", operation.foreign_key.on_update)?;

    Ok(format!(
        "ALTER TABLE {table} ADD CONSTRAINT {constraint} FOREIGN KEY ({columns}) REFERENCES {referenced_table} ({referenced_columns}){on_delete}{on_update}"
    ))
}

fn compile_drop_foreign_key(operation: &DropForeignKey) -> Result<String, OrmError> {
    Ok(format!(
        "ALTER TABLE {} DROP CONSTRAINT {}",
        quote_qualified_identifier(&operation.schema_name, &operation.table_name)?,
        quote_identifier(&operation.foreign_key_name)?,
    ))
}

fn render_foreign_key_action_clause(
    action_kind: &str,
    action: ReferentialAction,
) -> Result<String, OrmError> {
    let action_sql = match action {
        ReferentialAction::NoAction => "NO ACTION",
        ReferentialAction::Cascade => "CASCADE",
        ReferentialAction::SetNull => "SET NULL",
        ReferentialAction::SetDefault => "SET DEFAULT",
    };

    Ok(format!(" ON {action_kind} {action_sql}"))
}

fn compile_index_column(column: &IndexColumnSnapshot) -> Result<String, OrmError> {
    Ok(format!(
        "{} {}",
        quote_identifier(&column.column_name)?,
        if column.descending { "DESC" } else { "ASC" }
    ))
}

fn compile_column_definition(column: &ColumnSnapshot) -> Result<String, OrmError> {
    if let Some(computed_sql) = &column.computed_sql {
        return Ok(format!(
            "{} AS ({computed_sql})",
            crate::quote_identifier(&column.name)?,
        ));
    }

    if column.rowversion || column.sql_type == SqlServerType::RowVersion {
        return Ok(format!(
            "{} rowversion",
            crate::quote_identifier(&column.name)?
        ));
    }

    let mut definition = format!(
        "{} {}",
        crate::quote_identifier(&column.name)?,
        render_sql_type(column),
    );

    if let Some(identity) = column.identity {
        definition.push_str(&format!(
            " IDENTITY({}, {})",
            identity.seed, identity.increment
        ));
    }

    definition.push_str(if column.nullable {
        " NULL"
    } else {
        " NOT NULL"
    });

    if let Some(default_sql) = &column.default_sql {
        definition.push_str(&format!(" DEFAULT {default_sql}"));
    }

    Ok(definition)
}

fn compile_alter_column_definition(column: &ColumnSnapshot) -> Result<String, OrmError> {
    if column.computed_sql.is_some()
        || column.rowversion
        || column.sql_type == SqlServerType::RowVersion
    {
        return Err(OrmError::new(
            "SQL Server alter column compilation does not support computed or rowversion columns in this stage",
        ));
    }

    Ok(format!(
        "{} {} {}",
        crate::quote_identifier(&column.name)?,
        render_sql_type(column),
        if column.nullable { "NULL" } else { "NOT NULL" }
    ))
}

fn render_sql_type(column: &ColumnSnapshot) -> String {
    match column.sql_type {
        SqlServerType::BigInt => "bigint".to_string(),
        SqlServerType::Int => "int".to_string(),
        SqlServerType::SmallInt => "smallint".to_string(),
        SqlServerType::TinyInt => "tinyint".to_string(),
        SqlServerType::Bit => "bit".to_string(),
        SqlServerType::UniqueIdentifier => "uniqueidentifier".to_string(),
        SqlServerType::Date => "date".to_string(),
        SqlServerType::DateTime2 => "datetime2".to_string(),
        SqlServerType::Decimal => format!(
            "decimal({}, {})",
            column.precision.unwrap_or(18),
            column.scale.unwrap_or(2)
        ),
        SqlServerType::Float => "float".to_string(),
        SqlServerType::Money => "money".to_string(),
        SqlServerType::NVarChar => format!("nvarchar({})", column.max_length.unwrap_or(255)),
        SqlServerType::VarBinary => match column.max_length {
            Some(length) => format!("varbinary({length})"),
            None => "varbinary(max)".to_string(),
        },
        SqlServerType::RowVersion => "rowversion".to_string(),
        SqlServerType::Custom(name) => name.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::super::SqlServerCompiler;
    use sql_orm_core::{IdentityMetadata, ReferentialAction, SqlServerType};
    use sql_orm_migrate::{
        AddColumn, AddForeignKey, AlterColumn, ColumnSnapshot, CreateIndex, CreateSchema,
        CreateTable, DropColumn, DropForeignKey, DropIndex, DropSchema, DropTable,
        ForeignKeySnapshot, IndexColumnSnapshot, IndexSnapshot, MigrationOperation, RenameColumn,
        RenameTable, TableSnapshot,
    };

    fn customer_table() -> TableSnapshot {
        TableSnapshot::new(
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
                    Some(180),
                    None,
                    None,
                ),
                ColumnSnapshot::new(
                    "created_at",
                    SqlServerType::DateTime2,
                    false,
                    false,
                    None,
                    Some("SYSUTCDATETIME()".to_string()),
                    None,
                    false,
                    true,
                    true,
                    None,
                    None,
                    None,
                ),
            ],
            Some("pk_customers".to_string()),
            vec!["id".to_string()],
            vec![],
            vec![],
        )
    }

    #[test]
    fn compiles_stage_seven_migration_operations_to_sql() {
        let operations = vec![
            MigrationOperation::CreateSchema(CreateSchema::new("sales")),
            MigrationOperation::CreateTable(CreateTable::new("sales", customer_table())),
            MigrationOperation::AddColumn(AddColumn::new(
                "sales",
                "customers",
                ColumnSnapshot::new(
                    "version",
                    SqlServerType::RowVersion,
                    false,
                    false,
                    None,
                    None,
                    None,
                    true,
                    false,
                    false,
                    None,
                    None,
                    None,
                ),
            )),
            MigrationOperation::DropColumn(DropColumn::new("sales", "customers", "phone")),
            MigrationOperation::DropTable(DropTable::new("sales", "customers_archive")),
            MigrationOperation::DropSchema(DropSchema::new("legacy")),
        ];

        let sql = SqlServerCompiler::compile_migration_operations(&operations).unwrap();

        assert_eq!(
            sql[0],
            "IF SCHEMA_ID(N'sales') IS NULL EXEC(N'CREATE SCHEMA [sales]')"
        );
        assert!(sql[1].contains("CREATE TABLE [sales].[customers]"));
        assert!(sql[1].contains("[id] bigint IDENTITY(1, 1) NOT NULL"));
        assert!(sql[1].contains("CONSTRAINT [pk_customers] PRIMARY KEY ([id])"));
        assert_eq!(
            sql[2],
            "ALTER TABLE [sales].[customers] ADD [version] rowversion"
        );
        assert_eq!(
            sql[3],
            "ALTER TABLE [sales].[customers] DROP COLUMN [phone]"
        );
        assert_eq!(sql[4], "DROP TABLE [sales].[customers_archive]");
        assert_eq!(sql[5], "DROP SCHEMA [legacy]");
    }

    #[test]
    fn compiles_migrations_history_table_sql() {
        let sql = SqlServerCompiler::compile_migrations_history_table().unwrap();

        assert!(sql.contains("IF OBJECT_ID(N'dbo.__sql_orm_migrations', N'U') IS NULL"));
        assert!(sql.contains("CREATE TABLE [dbo].[__sql_orm_migrations]"));
        assert!(sql.contains("[applied_at] datetime2 NOT NULL DEFAULT SYSUTCDATETIME()"));
        assert!(sql.contains("[orm_version] nvarchar(50) NOT NULL"));
    }

    #[test]
    fn rejects_unsupported_alter_column_default_changes() {
        let operation = MigrationOperation::AlterColumn(AlterColumn::new(
            "sales",
            "customers",
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
                Some(180),
                None,
                None,
            ),
            ColumnSnapshot::new(
                "email",
                SqlServerType::NVarChar,
                false,
                false,
                None,
                Some("'unknown'".to_string()),
                None,
                false,
                true,
                true,
                Some(180),
                None,
                None,
            ),
        ));

        let error = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server alter column compilation only supports type and nullability changes in this stage"
        );
    }

    #[test]
    fn compiles_basic_alter_column_type_and_nullability_change() {
        let operation = MigrationOperation::AlterColumn(AlterColumn::new(
            "sales",
            "customers",
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
                Some(180),
                None,
                None,
            ),
            ColumnSnapshot::new(
                "email",
                SqlServerType::NVarChar,
                true,
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

        let sql = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap();

        assert_eq!(
            sql[0],
            "ALTER TABLE [sales].[customers] ALTER COLUMN [email] nvarchar(255) NULL"
        );
    }

    #[test]
    fn compiles_rename_column_to_sp_rename() {
        let operation = MigrationOperation::RenameColumn(RenameColumn::new(
            "sales",
            "customers",
            "email",
            "email_address",
        ));

        let sql = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap();

        assert_eq!(
            sql[0],
            "EXEC sp_rename N'[sales].[customers].[email]', N'email_address', N'COLUMN'"
        );
    }

    #[test]
    fn compiles_rename_table_to_sp_rename() {
        let operation =
            MigrationOperation::RenameTable(RenameTable::new("sales", "customers", "clients"));

        let sql = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap();

        assert_eq!(
            sql[0],
            "EXEC sp_rename N'[sales].[customers]', N'clients', N'OBJECT'"
        );
    }

    #[test]
    fn migration_literals_escape_single_quotes() {
        let operations = vec![
            MigrationOperation::CreateSchema(CreateSchema::new("sales'ops")),
            MigrationOperation::RenameTable(RenameTable::new("sales", "customers", "client's")),
        ];

        let sql = SqlServerCompiler::compile_migration_operations(&operations).unwrap();

        assert_eq!(
            sql[0],
            "IF SCHEMA_ID(N'sales''ops') IS NULL EXEC(N'CREATE SCHEMA [sales''ops]')"
        );
        assert_eq!(
            sql[1],
            "EXEC sp_rename N'[sales].[customers]', N'client''s', N'OBJECT'"
        );
    }

    #[test]
    fn compiles_computed_column_in_create_and_add_column_definitions() {
        let operations = vec![
            MigrationOperation::CreateTable(CreateTable::new(
                "sales",
                TableSnapshot::new(
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
                    None,
                    vec![],
                    vec![],
                    vec![],
                ),
            )),
            MigrationOperation::AddColumn(AddColumn::new(
                "sales",
                "order_lines",
                ColumnSnapshot::new(
                    "discounted_total",
                    SqlServerType::Decimal,
                    false,
                    false,
                    None,
                    None,
                    Some("[line_total] * (1 - [discount])".to_string()),
                    false,
                    false,
                    false,
                    None,
                    Some(18),
                    Some(2),
                ),
            )),
        ];

        let sql = SqlServerCompiler::compile_migration_operations(&operations).unwrap();

        assert_eq!(
            sql[0],
            "CREATE TABLE [sales].[order_lines] (\n    [line_total] AS ([unit_price] * [quantity])\n)"
        );
        assert_eq!(
            sql[1],
            "ALTER TABLE [sales].[order_lines] ADD [discounted_total] AS ([line_total] * (1 - [discount]))"
        );
    }

    #[test]
    fn rejects_alter_column_for_computed_column_changes() {
        let operation = MigrationOperation::AlterColumn(AlterColumn::new(
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
        ));

        let error = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server alter column compilation only supports type and nullability changes in this stage"
        );
    }

    #[test]
    fn compiles_foreign_key_migration_operations_to_sql() {
        let operations = vec![
            MigrationOperation::AddForeignKey(AddForeignKey::new(
                "sales",
                "orders",
                ForeignKeySnapshot::new(
                    "fk_orders_customer_id_customers",
                    vec!["customer_id".to_string()],
                    "sales",
                    "customers",
                    vec!["id".to_string()],
                    ReferentialAction::Cascade,
                    ReferentialAction::SetNull,
                ),
            )),
            MigrationOperation::DropForeignKey(DropForeignKey::new(
                "sales",
                "orders",
                "fk_orders_customer_id_customers",
            )),
        ];

        let sql = SqlServerCompiler::compile_migration_operations(&operations).unwrap();

        assert_eq!(
            sql[0],
            "ALTER TABLE [sales].[orders] ADD CONSTRAINT [fk_orders_customer_id_customers] FOREIGN KEY ([customer_id]) REFERENCES [sales].[customers] ([id]) ON DELETE CASCADE ON UPDATE SET NULL"
        );
        assert_eq!(
            sql[1],
            "ALTER TABLE [sales].[orders] DROP CONSTRAINT [fk_orders_customer_id_customers]"
        );
    }

    #[test]
    fn compiles_foreign_key_no_action_clauses_explicitly() {
        let operation = MigrationOperation::AddForeignKey(AddForeignKey::new(
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

        let sql = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap();

        assert_eq!(
            sql[0],
            "ALTER TABLE [sales].[orders] ADD CONSTRAINT [fk_orders_customer_id_customers] FOREIGN KEY ([customer_id]) REFERENCES [sales].[customers] ([id]) ON DELETE NO ACTION ON UPDATE NO ACTION"
        );
    }

    #[test]
    fn compiles_foreign_key_set_default_and_composite_columns_to_sql() {
        let operation = MigrationOperation::AddForeignKey(AddForeignKey::new(
            "sales",
            "order_allocations",
            ForeignKeySnapshot::new(
                "fk_order_allocations_customer_branch_customers",
                vec!["customer_id".to_string(), "branch_id".to_string()],
                "sales",
                "customers",
                vec!["id".to_string(), "branch_id".to_string()],
                ReferentialAction::SetDefault,
                ReferentialAction::SetDefault,
            ),
        ));

        let sql = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap();

        assert_eq!(
            sql[0],
            "ALTER TABLE [sales].[order_allocations] ADD CONSTRAINT [fk_order_allocations_customer_branch_customers] FOREIGN KEY ([customer_id], [branch_id]) REFERENCES [sales].[customers] ([id], [branch_id]) ON DELETE SET DEFAULT ON UPDATE SET DEFAULT"
        );
    }

    #[test]
    fn rejects_foreign_key_with_mismatched_column_cardinality() {
        let operation = MigrationOperation::AddForeignKey(AddForeignKey::new(
            "sales",
            "orders",
            ForeignKeySnapshot::new(
                "fk_orders_customer_branch_customers",
                vec!["customer_id".to_string()],
                "sales",
                "customers",
                vec!["id".to_string(), "branch_id".to_string()],
                ReferentialAction::NoAction,
                ReferentialAction::NoAction,
            ),
        ));

        let error = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server foreign key migration compilation requires the same number of local and referenced columns"
        );
    }

    #[test]
    fn compiles_index_migration_operations_to_sql() {
        let operations = vec![
            MigrationOperation::CreateIndex(CreateIndex::new(
                "sales",
                "orders",
                IndexSnapshot::new(
                    "ix_orders_customer_id_total_cents",
                    vec![
                        IndexColumnSnapshot::asc("customer_id"),
                        IndexColumnSnapshot::desc("total_cents"),
                    ],
                    false,
                ),
            )),
            MigrationOperation::CreateIndex(CreateIndex::new(
                "sales",
                "orders",
                IndexSnapshot::new(
                    "ux_orders_external_id",
                    vec![IndexColumnSnapshot::asc("external_id")],
                    true,
                ),
            )),
            MigrationOperation::DropIndex(DropIndex::new(
                "sales",
                "orders",
                "ix_orders_customer_id_total_cents",
            )),
        ];

        let sql = SqlServerCompiler::compile_migration_operations(&operations).unwrap();

        assert_eq!(
            sql[0],
            "CREATE INDEX [ix_orders_customer_id_total_cents] ON [sales].[orders] ([customer_id] ASC, [total_cents] DESC)"
        );
        assert_eq!(
            sql[1],
            "CREATE UNIQUE INDEX [ux_orders_external_id] ON [sales].[orders] ([external_id] ASC)"
        );
        assert_eq!(
            sql[2],
            "DROP INDEX [ix_orders_customer_id_total_cents] ON [sales].[orders]"
        );
    }

    #[test]
    fn rejects_create_index_without_columns() {
        let operation = MigrationOperation::CreateIndex(CreateIndex::new(
            "sales",
            "orders",
            IndexSnapshot::new("ix_orders_empty", vec![], false),
        ));

        let error = SqlServerCompiler::compile_migration_operations(&[operation]).unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server index migration compilation requires at least one indexed column"
        );
    }
}
