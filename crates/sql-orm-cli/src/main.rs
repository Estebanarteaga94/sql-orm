use sql_orm_migrate::{
    MigrationOperation, ModelSnapshot, build_database_update_script, create_migration_scaffold,
    create_migration_scaffold_with_snapshot, diff_column_operations, diff_relational_operations,
    diff_schema_and_table_operations, list_migrations, read_latest_model_snapshot,
    read_model_snapshot, write_migration_down_sql, write_migration_up_sql,
};
use sql_orm_query::CompiledQuery;
use sql_orm_sqlserver::SqlServerCompiler;
use sql_orm_tiberius::MssqlConnection;
use std::env;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    match run(env::args().collect(), Path::new(".")) {
        Ok(output) => println!("{output}"),
        Err(error) => {
            eprintln!("{error}");
            std::process::exit(1);
        }
    }
}

fn run(args: Vec<String>, root: &Path) -> Result<String, String> {
    match parse_command(&args)? {
        CliCommand::MigrationAdd { name, options } => {
            let current_snapshot = load_current_model_snapshot(root, &options)?;
            let previous_snapshot = match current_snapshot {
                Some(_) => load_previous_model_snapshot(root)?,
                None => None,
            };
            let migration_plan = build_migration_plan(
                previous_snapshot.as_ref().map(|(_, snapshot)| snapshot),
                current_snapshot.as_ref(),
            )
            .map_err(|error| format!("failed to build migration plan: {error}"))?;
            if let Some(plan) = migration_plan.as_ref()
                && !options.allow_destructive
                && let Some(operation) = first_destructive_migration_operation(&plan.operations)
            {
                return Err(format!(
                    "Error: destructive migration detected.\nOperation: {operation}\nUse --allow-destructive or edit migration manually."
                ));
            }
            let scaffold = match current_snapshot.as_ref() {
                Some(snapshot) => create_migration_scaffold_with_snapshot(root, &name, snapshot),
                None => create_migration_scaffold(root, &name),
            }
            .map_err(|error| error.to_string())?;
            if let Some(plan) = migration_plan.as_ref() {
                write_migration_up_sql(&scaffold.directory.join("up.sql"), &plan.sql_statements)
                    .map_err(|error| error.to_string())?;
                if let Some(down_sql_statements) = &plan.down_sql_statements {
                    write_migration_down_sql(
                        &scaffold.directory.join("down.sql"),
                        down_sql_statements,
                    )
                    .map_err(|error| error.to_string())?;
                }
            }

            let mut output = format!(
                "Created migration {}\nPath: {}",
                scaffold.id,
                scaffold.directory.display()
            );
            output.push_str(&format!(
                "\nArtifacts:\n  up.sql: {}\n  down.sql: {}\n  model_snapshot.json: {}\n  migration.rs: deferred for MVP",
                scaffold.up_sql_path().display(),
                scaffold.down_sql_path().display(),
                scaffold.snapshot_path().display()
            ));

            if let Some((migration, previous_snapshot)) = previous_snapshot {
                output.push_str(&format!(
                    "\nPrevious snapshot: {} (schemas: {})",
                    migration.id,
                    previous_snapshot.schemas.len()
                ));
            } else if current_snapshot.is_some() {
                output.push_str("\nPrevious snapshot: none");
            }

            if let Some(current_snapshot) = current_snapshot {
                output.push_str(&format!(
                    "\nCurrent snapshot: schemas={} tables={}",
                    current_snapshot.schemas.len(),
                    current_snapshot
                        .schemas
                        .iter()
                        .map(|schema| schema.tables.len())
                        .sum::<usize>()
                ));
            }

            if let Some(plan) = migration_plan {
                output.push_str(&format!("\nPlanned operations: {}", plan.operations.len()));
                output.push_str(&format!(
                    "\nCompiled SQL statements: {}",
                    plan.sql_statements.len()
                ));
                output.push_str("\nup.sql: generated");
                match &plan.down_sql_statements {
                    Some(statements) => output.push_str(&format!(
                        "\ndown.sql: generated ({} statements)",
                        statements.len()
                    )),
                    None => output.push_str(&format!(
                        "\ndown.sql: manual ({})",
                        plan.down_sql_reason
                            .as_deref()
                            .unwrap_or("migration includes non-reversible operations")
                    )),
                }
            }

            Ok(output)
        }
        CliCommand::MigrationList => {
            let migrations = list_migrations(root).map_err(|error| error.to_string())?;
            if migrations.is_empty() {
                return Ok("No migrations found.".to_string());
            }

            Ok(migrations
                .iter()
                .map(|migration| {
                    format!(
                        "{} | {} | {}",
                        migration.id,
                        migration.name,
                        migration.directory.display()
                    )
                })
                .collect::<Vec<_>>()
                .join("\n"))
        }
        CliCommand::DatabaseUpdate { options } => {
            let history_table_sql = SqlServerCompiler::compile_migrations_history_table()
                .map_err(|error| error.to_string())?;
            let script = build_database_update_script(root, &history_table_sql)
                .map_err(|error| error.to_string())?;

            if !options.execute {
                return Ok(script);
            }

            let connection_string = resolve_database_update_connection_string(&options)?;
            execute_database_update_script(&connection_string, script)?;
            Ok("Database update applied.".to_string())
        }
    }
}

fn load_current_model_snapshot(
    root: &Path,
    options: &MigrationAddOptions,
) -> Result<Option<ModelSnapshot>, String> {
    if let Some(path) = &options.model_snapshot {
        let snapshot_path = resolve_project_path(root, path);
        let snapshot = read_model_snapshot(&snapshot_path).map_err(|error| {
            format!(
                "failed to load current model snapshot from {}: {error}",
                snapshot_path.display()
            )
        })?;
        return Ok(Some(snapshot));
    }

    if let Some(snapshot_bin) = &options.snapshot_bin {
        let manifest_path = options
            .manifest_path
            .as_ref()
            .map(|path| resolve_project_path(root, path));
        let output = run_snapshot_exporter(snapshot_bin, manifest_path.as_deref())?;
        let snapshot = ModelSnapshot::from_json(&output)
            .map_err(|error| format!("failed to deserialize snapshot exporter output: {error}"))?;
        return Ok(Some(snapshot));
    }

    Ok(None)
}

fn load_previous_model_snapshot(
    root: &Path,
) -> Result<Option<(sql_orm_migrate::MigrationEntry, ModelSnapshot)>, String> {
    read_latest_model_snapshot(root)
        .map_err(|error| format!("failed to load previous model snapshot: {error}"))
}

fn build_migration_plan(
    previous: Option<&ModelSnapshot>,
    current: Option<&ModelSnapshot>,
) -> Result<Option<MigrationPlan>, String> {
    let Some(current) = current else {
        return Ok(None);
    };

    let previous = previous.cloned().unwrap_or_default();
    let mut operations = diff_schema_and_table_operations(&previous, current);
    operations.extend(diff_column_operations(&previous, current));
    operations.extend(diff_relational_operations(&previous, current));

    let sql_statements = SqlServerCompiler::compile_migration_operations(&operations)
        .map_err(|error| error.to_string())?;
    let (down_operations, down_sql_reason) = reverse_migration_operations(&operations);
    let down_sql_statements = match down_operations {
        Some(operations) => Some(
            SqlServerCompiler::compile_migration_operations(&operations)
                .map_err(|error| error.to_string())?,
        ),
        None => None,
    };

    Ok(Some(MigrationPlan {
        operations,
        sql_statements,
        down_sql_statements,
        down_sql_reason,
    }))
}

fn reverse_migration_operations(
    operations: &[MigrationOperation],
) -> (Option<Vec<MigrationOperation>>, Option<String>) {
    let mut reversed = Vec::with_capacity(operations.len());

    for operation in operations.iter().rev() {
        let Some(reverse_operation) = reverse_migration_operation(operation) else {
            return (
                None,
                Some(format!(
                    "{} cannot be reversed automatically because its operation payload is incomplete",
                    operation_label(operation)
                )),
            );
        };
        reversed.push(reverse_operation);
    }

    (Some(reversed), None)
}

fn reverse_migration_operation(operation: &MigrationOperation) -> Option<MigrationOperation> {
    match operation {
        MigrationOperation::CreateSchema(operation) => Some(MigrationOperation::DropSchema(
            sql_orm_migrate::DropSchema::new(operation.schema_name.clone()),
        )),
        MigrationOperation::DropSchema(operation) => Some(MigrationOperation::CreateSchema(
            sql_orm_migrate::CreateSchema::new(operation.schema_name.clone()),
        )),
        MigrationOperation::CreateTable(operation) => Some(MigrationOperation::DropTable(
            sql_orm_migrate::DropTable::new(
                operation.schema_name.clone(),
                operation.table.name.clone(),
            ),
        )),
        MigrationOperation::DropTable(_) => None,
        MigrationOperation::RenameTable(operation) => Some(MigrationOperation::RenameTable(
            sql_orm_migrate::RenameTable::new(
                operation.schema_name.clone(),
                operation.next_table_name.clone(),
                operation.previous_table_name.clone(),
            ),
        )),
        MigrationOperation::RenameColumn(operation) => Some(MigrationOperation::RenameColumn(
            sql_orm_migrate::RenameColumn::new(
                operation.schema_name.clone(),
                operation.table_name.clone(),
                operation.next_column_name.clone(),
                operation.previous_column_name.clone(),
            ),
        )),
        MigrationOperation::AddColumn(operation) => Some(MigrationOperation::DropColumn(
            sql_orm_migrate::DropColumn::new(
                operation.schema_name.clone(),
                operation.table_name.clone(),
                operation.column.name.clone(),
            ),
        )),
        MigrationOperation::DropColumn(_) => None,
        MigrationOperation::AlterColumn(operation) => Some(MigrationOperation::AlterColumn(
            sql_orm_migrate::AlterColumn::new(
                operation.schema_name.clone(),
                operation.table_name.clone(),
                operation.next.clone(),
                operation.previous.clone(),
            ),
        )),
        MigrationOperation::CreateIndex(operation) => Some(MigrationOperation::DropIndex(
            sql_orm_migrate::DropIndex::new(
                operation.schema_name.clone(),
                operation.table_name.clone(),
                operation.index.name.clone(),
            ),
        )),
        MigrationOperation::DropIndex(_) => None,
        MigrationOperation::AddForeignKey(operation) => Some(MigrationOperation::DropForeignKey(
            sql_orm_migrate::DropForeignKey::new(
                operation.schema_name.clone(),
                operation.table_name.clone(),
                operation.foreign_key.name.clone(),
            ),
        )),
        MigrationOperation::DropForeignKey(_) => None,
    }
}

fn first_destructive_migration_operation(operations: &[MigrationOperation]) -> Option<String> {
    operations.iter().find_map(destructive_operation_label)
}

fn destructive_operation_label(operation: &MigrationOperation) -> Option<String> {
    match operation {
        MigrationOperation::DropTable(operation) => Some(format!(
            "DropTable {}.{}",
            operation.schema_name, operation.table_name
        )),
        MigrationOperation::DropColumn(operation) => Some(format!(
            "DropColumn {}.{}.{}",
            operation.schema_name, operation.table_name, operation.column_name
        )),
        MigrationOperation::AlterColumn(operation) => {
            if operation.previous.sql_type != operation.next.sql_type {
                return Some(format!(
                    "AlterColumn {}.{}.{} changes type",
                    operation.schema_name, operation.table_name, operation.next.name
                ));
            }

            if let (Some(previous_length), Some(next_length)) =
                (operation.previous.max_length, operation.next.max_length)
                && next_length < previous_length
            {
                return Some(format!(
                    "AlterColumn {}.{}.{} reduces length",
                    operation.schema_name, operation.table_name, operation.next.name
                ));
            }

            if operation.previous.nullable
                && !operation.next.nullable
                && operation.next.default_sql.is_none()
            {
                return Some(format!(
                    "AlterColumn {}.{}.{} changes nullable to non-nullable without default",
                    operation.schema_name, operation.table_name, operation.next.name
                ));
            }

            None
        }
        MigrationOperation::CreateSchema(_)
        | MigrationOperation::DropSchema(_)
        | MigrationOperation::CreateTable(_)
        | MigrationOperation::RenameTable(_)
        | MigrationOperation::RenameColumn(_)
        | MigrationOperation::AddColumn(_)
        | MigrationOperation::CreateIndex(_)
        | MigrationOperation::DropIndex(_)
        | MigrationOperation::AddForeignKey(_)
        | MigrationOperation::DropForeignKey(_) => None,
    }
}

fn operation_label(operation: &MigrationOperation) -> String {
    match operation {
        MigrationOperation::CreateSchema(operation) => {
            format!("CreateSchema {}", operation.schema_name)
        }
        MigrationOperation::DropSchema(operation) => {
            format!("DropSchema {}", operation.schema_name)
        }
        MigrationOperation::CreateTable(operation) => {
            format!(
                "CreateTable {}.{}",
                operation.schema_name, operation.table.name
            )
        }
        MigrationOperation::DropTable(operation) => {
            format!(
                "DropTable {}.{}",
                operation.schema_name, operation.table_name
            )
        }
        MigrationOperation::RenameTable(operation) => format!(
            "RenameTable {}.{} -> {}",
            operation.schema_name, operation.previous_table_name, operation.next_table_name
        ),
        MigrationOperation::RenameColumn(operation) => format!(
            "RenameColumn {}.{}.{} -> {}",
            operation.schema_name,
            operation.table_name,
            operation.previous_column_name,
            operation.next_column_name
        ),
        MigrationOperation::AddColumn(operation) => format!(
            "AddColumn {}.{}.{}",
            operation.schema_name, operation.table_name, operation.column.name
        ),
        MigrationOperation::DropColumn(operation) => format!(
            "DropColumn {}.{}.{}",
            operation.schema_name, operation.table_name, operation.column_name
        ),
        MigrationOperation::AlterColumn(operation) => format!(
            "AlterColumn {}.{}.{}",
            operation.schema_name, operation.table_name, operation.next.name
        ),
        MigrationOperation::CreateIndex(operation) => format!(
            "CreateIndex {}.{}.{}",
            operation.schema_name, operation.table_name, operation.index.name
        ),
        MigrationOperation::DropIndex(operation) => format!(
            "DropIndex {}.{}.{}",
            operation.schema_name, operation.table_name, operation.index_name
        ),
        MigrationOperation::AddForeignKey(operation) => format!(
            "AddForeignKey {}.{}.{}",
            operation.schema_name, operation.table_name, operation.foreign_key.name
        ),
        MigrationOperation::DropForeignKey(operation) => format!(
            "DropForeignKey {}.{}.{}",
            operation.schema_name, operation.table_name, operation.foreign_key_name
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct MigrationPlan {
    operations: Vec<MigrationOperation>,
    sql_statements: Vec<String>,
    down_sql_statements: Option<Vec<String>>,
    down_sql_reason: Option<String>,
}

fn run_snapshot_exporter(
    snapshot_bin: &str,
    manifest_path: Option<&Path>,
) -> Result<String, String> {
    let mut command = Command::new("cargo");
    command.arg("run").arg("--quiet");

    if let Some(manifest_path) = manifest_path {
        command.arg("--manifest-path").arg(manifest_path);
    }

    command.arg("--bin").arg(snapshot_bin);

    let output = command
        .output()
        .map_err(|error| format!("failed to execute snapshot exporter binary: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let stderr = stderr.trim();
        return Err(if stderr.is_empty() {
            format!("snapshot exporter binary `{snapshot_bin}` failed")
        } else {
            format!("snapshot exporter binary `{snapshot_bin}` failed: {stderr}")
        });
    }

    String::from_utf8(output.stdout)
        .map_err(|_| "snapshot exporter emitted non-utf8 output".to_string())
}

fn resolve_project_path(root: &Path, path: &Path) -> PathBuf {
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        root.join(path)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum CliCommand {
    MigrationAdd {
        name: String,
        options: MigrationAddOptions,
    },
    MigrationList,
    DatabaseUpdate {
        options: DatabaseUpdateOptions,
    },
}

fn parse_command(args: &[String]) -> Result<CliCommand, String> {
    match args {
        [_bin, group, action, name, rest @ ..] if group == "migration" && action == "add" => {
            Ok(CliCommand::MigrationAdd {
                name: name.clone(),
                options: parse_migration_add_options(rest)?,
            })
        }
        [_bin, group, action] if group == "migration" && action == "list" => {
            Ok(CliCommand::MigrationList)
        }
        [_bin, group, action, rest @ ..] if group == "database" && action == "update" => {
            Ok(CliCommand::DatabaseUpdate {
                options: parse_database_update_options(rest)?,
            })
        }
        _ => Err(
            "Usage:\n  sql-orm-cli migration add <Name> [--model-snapshot <Path>] [--snapshot-bin <BinName> [--manifest-path <Path>]] [--allow-destructive]\n  sql-orm-cli migration list\n  sql-orm-cli database update [--execute [--connection-string <ConnectionString>]]".to_string(),
        ),
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct DatabaseUpdateOptions {
    execute: bool,
    connection_string: Option<String>,
}

fn parse_database_update_options(args: &[String]) -> Result<DatabaseUpdateOptions, String> {
    let mut options = DatabaseUpdateOptions::default();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--execute" => {
                options.execute = true;
                index += 1;
            }
            "--connection-string" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--connection-string requires a value".to_string())?;
                options.connection_string = Some(value.clone());
                index += 2;
            }
            unknown => {
                return Err(format!("unknown database update option: {unknown}"));
            }
        }
    }

    if options.connection_string.is_some() && !options.execute {
        return Err("--connection-string requires --execute".to_string());
    }

    Ok(options)
}

fn resolve_database_update_connection_string(
    options: &DatabaseUpdateOptions,
) -> Result<String, String> {
    if let Some(connection_string) = &options.connection_string {
        return Ok(connection_string.clone());
    }

    env::var("DATABASE_URL")
        .or_else(|_| env::var("SQL_ORM_TEST_CONNECTION_STRING"))
        .map_err(|_| {
            "database update --execute requires --connection-string, DATABASE_URL, or SQL_ORM_TEST_CONNECTION_STRING".to_string()
        })
}

fn execute_database_update_script(connection_string: &str, script: String) -> Result<(), String> {
    let runtime = tokio::runtime::Runtime::new()
        .map_err(|error| format!("failed to create async runtime: {error}"))?;

    runtime.block_on(async {
        let mut connection = MssqlConnection::connect(connection_string)
            .await
            .map_err(|error| format!("failed to connect to SQL Server: {error}"))?;
        connection
            .execute(CompiledQuery::new(script, vec![]))
            .await
            .map_err(|error| format!("failed to apply database update: {error}"))?;

        Ok(())
    })
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
struct MigrationAddOptions {
    model_snapshot: Option<PathBuf>,
    snapshot_bin: Option<String>,
    manifest_path: Option<PathBuf>,
    allow_destructive: bool,
}

fn parse_migration_add_options(args: &[String]) -> Result<MigrationAddOptions, String> {
    let mut options = MigrationAddOptions::default();
    let mut index = 0;

    while index < args.len() {
        match args[index].as_str() {
            "--model-snapshot" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--model-snapshot requires a path".to_string())?;
                options.model_snapshot = Some(PathBuf::from(value));
                index += 2;
            }
            "--snapshot-bin" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--snapshot-bin requires a binary name".to_string())?;
                options.snapshot_bin = Some(value.clone());
                index += 2;
            }
            "--manifest-path" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| "--manifest-path requires a path".to_string())?;
                options.manifest_path = Some(PathBuf::from(value));
                index += 2;
            }
            "--allow-destructive" => {
                options.allow_destructive = true;
                index += 1;
            }
            unknown => {
                return Err(format!("unknown migration add option: {unknown}"));
            }
        }
    }

    if options.model_snapshot.is_some() && options.snapshot_bin.is_some() {
        return Err("--model-snapshot and --snapshot-bin cannot be used together".to_string());
    }

    if options.snapshot_bin.is_none() && options.manifest_path.is_some() {
        return Err("--manifest-path requires --snapshot-bin".to_string());
    }

    Ok(options)
}

#[cfg(test)]
mod tests {
    use super::{
        CliCommand, DatabaseUpdateOptions, MigrationAddOptions, build_migration_plan,
        first_destructive_migration_operation, parse_command, run,
    };
    use sql_orm_core::SqlServerType;
    use sql_orm_migrate::{
        AlterColumn, ColumnSnapshot, DropColumn, DropTable, MigrationOperation, ModelSnapshot,
        SchemaSnapshot, TableSnapshot, read_model_snapshot,
    };
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project_root() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sql_orm_cli_{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    fn test_column(
        name: &str,
        sql_type: SqlServerType,
        nullable: bool,
        default_sql: Option<&str>,
        max_length: Option<u32>,
    ) -> ColumnSnapshot {
        ColumnSnapshot::new(
            name,
            sql_type,
            nullable,
            false,
            None,
            default_sql.map(str::to_string),
            None,
            false,
            true,
            true,
            max_length,
            None,
            None,
        )
    }

    fn customer_snapshot(include_phone: bool) -> ModelSnapshot {
        let mut columns = vec![ColumnSnapshot::new(
            "id",
            SqlServerType::BigInt,
            false,
            true,
            None,
            None,
            None,
            false,
            true,
            true,
            None,
            None,
            None,
        )];

        if include_phone {
            columns.push(test_column(
                "phone",
                SqlServerType::NVarChar,
                true,
                None,
                Some(30),
            ));
        }

        ModelSnapshot::new(vec![SchemaSnapshot::new(
            "sales",
            vec![TableSnapshot::new(
                "customers",
                columns,
                None,
                vec!["id".to_string()],
                Vec::new(),
                Vec::new(),
            )],
        )])
    }

    fn audited_entity_snapshot(include_audit: bool) -> ModelSnapshot {
        let mut columns = vec![
            ColumnSnapshot::new(
                "id",
                SqlServerType::BigInt,
                false,
                true,
                None,
                None,
                None,
                false,
                false,
                false,
                None,
                None,
                None,
            ),
            test_column("name", SqlServerType::NVarChar, false, None, Some(120)),
            test_column(
                "status",
                SqlServerType::NVarChar,
                true,
                Some("'new'"),
                Some(40),
            ),
        ];

        if include_audit {
            columns.push(ColumnSnapshot::new(
                "created_at",
                SqlServerType::DateTime2,
                false,
                false,
                None,
                Some("SYSUTCDATETIME()".to_string()),
                None,
                false,
                false,
                false,
                None,
                None,
                None,
            ));
            columns.push(test_column(
                "created_by_user_id",
                SqlServerType::BigInt,
                true,
                None,
                None,
            ));
            columns.push(ColumnSnapshot::new(
                "updated_at",
                SqlServerType::DateTime2,
                true,
                false,
                None,
                Some("SYSUTCDATETIME()".to_string()),
                None,
                false,
                false,
                true,
                None,
                None,
                None,
            ));
            columns.push(test_column(
                "updated_by",
                SqlServerType::NVarChar,
                true,
                None,
                Some(120),
            ));
        }

        ModelSnapshot::new(vec![SchemaSnapshot::new(
            "audit",
            vec![TableSnapshot::new(
                "audited_entities",
                columns,
                None,
                vec!["id".to_string()],
                Vec::new(),
                Vec::new(),
            )],
        )])
    }

    #[test]
    fn parses_minimal_cli_commands() {
        assert_eq!(
            parse_command(&[
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "CreateCustomers".to_string(),
            ])
            .unwrap(),
            CliCommand::MigrationAdd {
                name: "CreateCustomers".to_string(),
                options: MigrationAddOptions::default()
            }
        );
        assert_eq!(
            parse_command(&[
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "CreateCustomers".to_string(),
                "--model-snapshot".to_string(),
                "target/current_model_snapshot.json".to_string(),
            ])
            .unwrap(),
            CliCommand::MigrationAdd {
                name: "CreateCustomers".to_string(),
                options: MigrationAddOptions {
                    model_snapshot: Some(PathBuf::from("target/current_model_snapshot.json")),
                    snapshot_bin: None,
                    manifest_path: None,
                    allow_destructive: false,
                }
            }
        );
        assert_eq!(
            parse_command(&[
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "CreateCustomers".to_string(),
                "--snapshot-bin".to_string(),
                "app-model-snapshot".to_string(),
                "--manifest-path".to_string(),
                "examples/todo-app/Cargo.toml".to_string(),
            ])
            .unwrap(),
            CliCommand::MigrationAdd {
                name: "CreateCustomers".to_string(),
                options: MigrationAddOptions {
                    model_snapshot: None,
                    snapshot_bin: Some("app-model-snapshot".to_string()),
                    manifest_path: Some(PathBuf::from("examples/todo-app/Cargo.toml")),
                    allow_destructive: false,
                }
            }
        );
        assert_eq!(
            parse_command(&[
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "DropCustomerPhone".to_string(),
                "--model-snapshot".to_string(),
                "target/current_model_snapshot.json".to_string(),
                "--allow-destructive".to_string(),
            ])
            .unwrap(),
            CliCommand::MigrationAdd {
                name: "DropCustomerPhone".to_string(),
                options: MigrationAddOptions {
                    model_snapshot: Some(PathBuf::from("target/current_model_snapshot.json")),
                    snapshot_bin: None,
                    manifest_path: None,
                    allow_destructive: true,
                }
            }
        );
        assert_eq!(
            parse_command(&[
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "list".to_string(),
            ])
            .unwrap(),
            CliCommand::MigrationList
        );
        assert_eq!(
            parse_command(&[
                "sql-orm-cli".to_string(),
                "database".to_string(),
                "update".to_string(),
            ])
            .unwrap(),
            CliCommand::DatabaseUpdate {
                options: DatabaseUpdateOptions::default()
            }
        );
        assert_eq!(
            parse_command(&[
                "sql-orm-cli".to_string(),
                "database".to_string(),
                "update".to_string(),
                "--execute".to_string(),
                "--connection-string".to_string(),
                "Server=localhost;Database=tempdb;".to_string(),
            ])
            .unwrap(),
            CliCommand::DatabaseUpdate {
                options: DatabaseUpdateOptions {
                    execute: true,
                    connection_string: Some("Server=localhost;Database=tempdb;".to_string()),
                }
            }
        );
    }

    #[test]
    fn run_migration_add_creates_scaffold() {
        let root = temp_project_root();

        let output = run(
            vec![
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "CreateCustomers".to_string(),
            ],
            &root,
        )
        .unwrap();

        assert!(output.contains("Created migration"));
        assert!(output.contains("Artifacts:"));
        assert!(output.contains("up.sql:"));
        assert!(output.contains("down.sql:"));
        assert!(output.contains("model_snapshot.json:"));
        assert!(output.contains("migration.rs: deferred for MVP"));
        assert!(root.join("migrations").exists());
    }

    #[test]
    fn run_migration_add_uses_current_model_snapshot_when_provided() {
        let root = temp_project_root();
        let snapshot_path = root.join("current_model_snapshot.json");
        fs::write(
            &snapshot_path,
            "{\n  \"schemas\": [\n    {\n      \"name\": \"sales\",\n      \"tables\": []\n    }\n  ]\n}\n",
        )
        .unwrap();

        let output = run(
            vec![
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "CreateCustomers".to_string(),
                "--model-snapshot".to_string(),
                "current_model_snapshot.json".to_string(),
            ],
            &root,
        )
        .unwrap();

        assert!(output.contains("Previous snapshot: none"));
        assert!(output.contains("Current snapshot: schemas=1 tables=0"));
        assert!(output.contains("Planned operations: 1"));
        assert!(output.contains("Compiled SQL statements: 1"));
        assert!(output.contains("up.sql: generated"));
        assert!(output.contains("down.sql: generated (1 statements)"));

        let migration_path = output
            .lines()
            .find_map(|line| line.strip_prefix("Path: "))
            .map(PathBuf::from)
            .unwrap();
        let snapshot = read_model_snapshot(&migration_path.join("model_snapshot.json")).unwrap();
        let up_sql = fs::read_to_string(migration_path.join("up.sql")).unwrap();
        let down_sql = fs::read_to_string(migration_path.join("down.sql")).unwrap();

        assert!(snapshot.schema("sales").is_some());
        assert_eq!(
            up_sql,
            "IF SCHEMA_ID(N'sales') IS NULL EXEC(N'CREATE SCHEMA [sales]');\n"
        );
        assert_eq!(down_sql, "DROP SCHEMA [sales];\n");
    }

    #[test]
    fn run_migration_add_uses_snapshot_exporter_binary_when_provided() {
        let root = temp_project_root();
        let fixture = root.join("fixture_app");
        let fixture_src = fixture.join("src");
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .to_path_buf();
        let orm_crate_root = repo_root.join("crates/sql-orm");
        let escaped_repo_root = orm_crate_root.display().to_string().replace('\\', "\\\\");

        fs::create_dir_all(&fixture_src).unwrap();
        fs::write(
            fixture.join("Cargo.toml"),
            format!(
                "[package]\nname = \"fixture-app\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nsql-orm = {{ path = \"{}\" }}\n",
                escaped_repo_root
            ),
        )
        .unwrap();
        fs::write(
            fixture_src.join("main.rs"),
            "use sql_orm::prelude::*;\n\n#[derive(Entity, Debug, Clone)]\n#[orm(schema = \"sales\", table = \"customers\")]\nstruct Customer {\n    #[orm(primary_key)]\n    id: i64,\n}\n\n#[derive(DbContext, Debug, Clone)]\nstruct AppDbContext {\n    customers: DbSet<Customer>,\n}\n\nfn main() {\n    print!(\"{}\", sql_orm::model_snapshot_json_from_source::<AppDbContext>().unwrap());\n}\n",
        )
        .unwrap();

        let output = run(
            vec![
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "CreateCustomers".to_string(),
                "--snapshot-bin".to_string(),
                "fixture-app".to_string(),
                "--manifest-path".to_string(),
                fixture.join("Cargo.toml").display().to_string(),
            ],
            &root,
        );

        let output = output.unwrap();
        assert!(output.contains("Previous snapshot: none"));
        assert!(output.contains("Current snapshot: schemas=1 tables=1"));
        assert!(output.contains("Planned operations: 2"));
        assert!(output.contains("Compiled SQL statements: 2"));
        assert!(output.contains("up.sql: generated"));
        let migration_path = output
            .lines()
            .find_map(|line| line.strip_prefix("Path: "))
            .map(PathBuf::from)
            .unwrap();
        let snapshot = read_model_snapshot(&migration_path.join("model_snapshot.json")).unwrap();
        let up_sql = fs::read_to_string(migration_path.join("up.sql")).unwrap();

        assert_eq!(snapshot.schemas.len(), 1);
        assert!(
            snapshot
                .schema("sales")
                .unwrap()
                .table("customers")
                .is_some()
        );
        assert!(up_sql.contains("IF SCHEMA_ID(N'sales') IS NULL EXEC(N'CREATE SCHEMA [sales]')"));
        assert!(up_sql.contains("CREATE TABLE [sales].[customers]"));
    }

    #[test]
    fn run_migration_add_loads_previous_snapshot_from_latest_local_migration() {
        let root = temp_project_root();
        let previous_dir = root.join("migrations/100_create_customers");
        let current_snapshot_path = root.join("current_model_snapshot.json");

        fs::create_dir_all(&previous_dir).unwrap();
        fs::write(previous_dir.join("up.sql"), "-- noop").unwrap();
        fs::write(previous_dir.join("down.sql"), "-- noop").unwrap();
        fs::write(
            previous_dir.join("model_snapshot.json"),
            "{\n  \"schemas\": [\n    {\n      \"name\": \"dbo\",\n      \"tables\": []\n    }\n  ]\n}\n",
        )
        .unwrap();
        fs::write(
            &current_snapshot_path,
            "{\n  \"schemas\": [\n    {\n      \"name\": \"sales\",\n      \"tables\": []\n    }\n  ]\n}\n",
        )
        .unwrap();

        let output = run(
            vec![
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "CreateOrders".to_string(),
                "--model-snapshot".to_string(),
                "current_model_snapshot.json".to_string(),
            ],
            &root,
        )
        .unwrap();

        assert!(output.contains("Previous snapshot: 100_create_customers (schemas: 1)"));
        assert!(output.contains("Current snapshot: schemas=1 tables=0"));
        assert!(output.contains("Planned operations: 2"));
        assert!(output.contains("Compiled SQL statements: 2"));
        assert!(output.contains("up.sql: generated"));
    }

    #[test]
    fn run_migration_add_blocks_destructive_changes_by_default() {
        let root = temp_project_root();
        let previous_dir = root.join("migrations/100_create_customers");
        let current_snapshot_path = root.join("current_model_snapshot.json");

        fs::create_dir_all(&previous_dir).unwrap();
        fs::write(previous_dir.join("up.sql"), "-- noop").unwrap();
        fs::write(previous_dir.join("down.sql"), "-- noop").unwrap();
        fs::write(
            previous_dir.join("model_snapshot.json"),
            customer_snapshot(true).to_json_pretty().unwrap(),
        )
        .unwrap();
        fs::write(
            &current_snapshot_path,
            customer_snapshot(false).to_json_pretty().unwrap(),
        )
        .unwrap();

        let error = run(
            vec![
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "DropCustomerPhone".to_string(),
                "--model-snapshot".to_string(),
                "current_model_snapshot.json".to_string(),
            ],
            &root,
        )
        .unwrap_err();

        assert!(error.contains("Error: destructive migration detected."));
        assert!(error.contains("Operation: DropColumn sales.customers.phone"));
        assert!(error.contains("Use --allow-destructive or edit migration manually."));
        assert!(!root.join("migrations").read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("dropcustomerphone")
        }));
    }

    #[test]
    fn run_migration_add_blocks_removing_audit_policy_by_default() {
        let root = temp_project_root();
        let previous_dir = root.join("migrations/100_create_audited_entities");
        let current_snapshot_path = root.join("current_model_snapshot.json");

        fs::create_dir_all(&previous_dir).unwrap();
        fs::write(previous_dir.join("up.sql"), "-- noop").unwrap();
        fs::write(previous_dir.join("down.sql"), "-- noop").unwrap();
        fs::write(
            previous_dir.join("model_snapshot.json"),
            audited_entity_snapshot(true).to_json_pretty().unwrap(),
        )
        .unwrap();
        fs::write(
            &current_snapshot_path,
            audited_entity_snapshot(false).to_json_pretty().unwrap(),
        )
        .unwrap();

        let error = run(
            vec![
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "RemoveAudit".to_string(),
                "--model-snapshot".to_string(),
                "current_model_snapshot.json".to_string(),
            ],
            &root,
        )
        .unwrap_err();

        assert!(error.contains("Error: destructive migration detected."));
        assert!(error.contains("Operation: DropColumn audit.audited_entities.created_at"));
        assert!(error.contains("Use --allow-destructive or edit migration manually."));
        assert!(!root.join("migrations").read_dir().unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains("removeaudit")
        }));
    }

    #[test]
    fn run_migration_add_allows_destructive_changes_with_explicit_flag() {
        let root = temp_project_root();
        let previous_dir = root.join("migrations/100_create_customers");
        let current_snapshot_path = root.join("current_model_snapshot.json");

        fs::create_dir_all(&previous_dir).unwrap();
        fs::write(previous_dir.join("up.sql"), "-- noop").unwrap();
        fs::write(previous_dir.join("down.sql"), "-- noop").unwrap();
        fs::write(
            previous_dir.join("model_snapshot.json"),
            customer_snapshot(true).to_json_pretty().unwrap(),
        )
        .unwrap();
        fs::write(
            &current_snapshot_path,
            customer_snapshot(false).to_json_pretty().unwrap(),
        )
        .unwrap();

        let output = run(
            vec![
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "add".to_string(),
                "DropCustomerPhone".to_string(),
                "--model-snapshot".to_string(),
                "current_model_snapshot.json".to_string(),
                "--allow-destructive".to_string(),
            ],
            &root,
        )
        .unwrap();

        assert!(output.contains("Planned operations: 1"));
        assert!(output.contains("up.sql: generated"));
        assert!(output.contains("down.sql: manual (DropColumn sales.customers.phone cannot be reversed automatically because its operation payload is incomplete)"));

        let migration_path = output
            .lines()
            .find_map(|line| line.strip_prefix("Path: "))
            .map(PathBuf::from)
            .unwrap();
        let up_sql = fs::read_to_string(migration_path.join("up.sql")).unwrap();
        let down_sql = fs::read_to_string(migration_path.join("down.sql")).unwrap();
        assert!(up_sql.contains("ALTER TABLE [sales].[customers] DROP COLUMN [phone]"));
        assert!(down_sql.contains("Manual rollback SQL for this editable migration"));
    }

    #[test]
    fn destructive_operation_detection_covers_drop_and_unsafe_alter_column() {
        assert_eq!(
            first_destructive_migration_operation(&[MigrationOperation::DropTable(
                DropTable::new("sales", "customers")
            )]),
            Some("DropTable sales.customers".to_string())
        );
        assert_eq!(
            first_destructive_migration_operation(&[MigrationOperation::DropColumn(
                DropColumn::new("sales", "customers", "phone")
            )]),
            Some("DropColumn sales.customers.phone".to_string())
        );
        assert_eq!(
            first_destructive_migration_operation(&[MigrationOperation::AlterColumn(
                AlterColumn::new(
                    "sales",
                    "customers",
                    test_column("email", SqlServerType::NVarChar, false, None, Some(160)),
                    test_column("email", SqlServerType::NVarChar, false, None, Some(120)),
                )
            )]),
            Some("AlterColumn sales.customers.email reduces length".to_string())
        );
        assert_eq!(
            first_destructive_migration_operation(&[MigrationOperation::AlterColumn(
                AlterColumn::new(
                    "sales",
                    "customers",
                    test_column("email", SqlServerType::NVarChar, false, None, Some(160)),
                    test_column("email", SqlServerType::Int, false, None, None),
                )
            )]),
            Some("AlterColumn sales.customers.email changes type".to_string())
        );
        assert_eq!(
            first_destructive_migration_operation(&[MigrationOperation::AlterColumn(
                AlterColumn::new(
                    "sales",
                    "customers",
                    test_column("email", SqlServerType::NVarChar, true, None, Some(160)),
                    test_column("email", SqlServerType::NVarChar, false, None, Some(160)),
                )
            )]),
            Some(
                "AlterColumn sales.customers.email changes nullable to non-nullable without default"
                    .to_string()
            )
        );
        assert_eq!(
            first_destructive_migration_operation(&[MigrationOperation::AlterColumn(
                AlterColumn::new(
                    "sales",
                    "customers",
                    test_column("email", SqlServerType::NVarChar, true, None, Some(160)),
                    test_column(
                        "email",
                        SqlServerType::NVarChar,
                        false,
                        Some("''"),
                        Some(160)
                    ),
                )
            )]),
            None
        );
    }

    #[test]
    fn build_migration_plan_returns_none_without_current_snapshot() {
        assert_eq!(build_migration_plan(None, None).unwrap(), None);
    }

    #[test]
    fn build_migration_plan_compiles_schema_and_table_operations() {
        let current = ModelSnapshot::from_json(
            "{\n  \"schemas\": [\n    {\n      \"name\": \"sales\",\n      \"tables\": [\n        {\n          \"name\": \"customers\",\n          \"renamed_from\": null,\n          \"columns\": [\n            {\n              \"name\": \"id\",\n              \"renamed_from\": null,\n              \"sql_type\": \"bigint\",\n              \"nullable\": false,\n              \"primary_key\": true,\n              \"identity\": null,\n              \"default_sql\": null,\n              \"computed_sql\": null,\n              \"rowversion\": false,\n              \"insertable\": true,\n              \"updatable\": true,\n              \"max_length\": null,\n              \"precision\": null,\n              \"scale\": null\n            }\n          ],\n          \"primary_key_name\": null,\n          \"primary_key_columns\": [\"id\"],\n          \"indexes\": [],\n          \"foreign_keys\": []\n        }\n      ]\n    }\n  ]\n}\n",
        )
        .unwrap();

        let plan = build_migration_plan(None, Some(&current)).unwrap().unwrap();

        assert_eq!(plan.operations.len(), 2);
        assert!(plan.sql_statements[0].contains("CREATE SCHEMA"));
        assert!(plan.sql_statements[1].contains("CREATE TABLE"));
        assert_eq!(
            plan.down_sql_statements.unwrap(),
            vec![
                "DROP TABLE [sales].[customers]".to_string(),
                "DROP SCHEMA [sales]".to_string(),
            ]
        );
    }

    #[test]
    fn run_migration_list_prints_existing_migrations() {
        let root = temp_project_root();
        fs::create_dir_all(root.join("migrations/100_create_customers")).unwrap();

        let output = run(
            vec![
                "sql-orm-cli".to_string(),
                "migration".to_string(),
                "list".to_string(),
            ],
            &root,
        )
        .unwrap();

        assert!(output.contains("100_create_customers"));
    }

    #[test]
    fn run_database_update_outputs_sql_script() {
        let root = temp_project_root();
        let migration_dir = root.join("migrations/100_create_customers");
        fs::create_dir_all(&migration_dir).unwrap();
        fs::write(
            migration_dir.join("up.sql"),
            "CREATE TABLE [sales].[customers] ([id] bigint NOT NULL);",
        )
        .unwrap();
        fs::write(migration_dir.join("down.sql"), "-- noop").unwrap();
        fs::write(
            migration_dir.join("model_snapshot.json"),
            "{ \"schemas\": [] }",
        )
        .unwrap();

        let output = run(
            vec![
                "sql-orm-cli".to_string(),
                "database".to_string(),
                "update".to_string(),
            ],
            &root,
        )
        .unwrap();

        assert!(output.contains("CREATE TABLE [dbo].[__sql_orm_migrations]"));
        assert!(output.contains("SET QUOTED_IDENTIFIER ON;"));
        assert!(output.contains("CREATE TABLE [sales].[customers]"));
        assert!(output.contains("INSERT INTO [dbo].[__sql_orm_migrations]"));
        assert!(output.contains("THROW 50001, N'sql-orm migration checksum mismatch"));
        assert!(output.contains("BEGIN TRANSACTION;"));
        assert!(output.contains("ROLLBACK TRANSACTION;"));
    }
}
