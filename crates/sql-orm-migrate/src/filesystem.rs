use crate::ModelSnapshot;
use sql_orm_core::OrmError;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MIGRATIONS_DIR: &str = "migrations";
const ORM_VERSION: &str = env!("CARGO_PKG_VERSION");

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationScaffold {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
}

impl MigrationScaffold {
    pub fn up_sql_path(&self) -> PathBuf {
        self.directory.join("up.sql")
    }

    pub fn down_sql_path(&self) -> PathBuf {
        self.directory.join("down.sql")
    }

    pub fn snapshot_path(&self) -> PathBuf {
        self.directory.join("model_snapshot.json")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MigrationEntry {
    pub id: String,
    pub name: String,
    pub directory: PathBuf,
    pub up_sql_path: PathBuf,
    pub down_sql_path: PathBuf,
    pub snapshot_path: PathBuf,
}

pub fn create_migration_scaffold(root: &Path, name: &str) -> Result<MigrationScaffold, OrmError> {
    create_migration_scaffold_with_snapshot(root, name, &ModelSnapshot::default())
}

pub fn create_migration_scaffold_with_snapshot(
    root: &Path,
    name: &str,
    snapshot: &ModelSnapshot,
) -> Result<MigrationScaffold, OrmError> {
    if name.trim().is_empty() {
        return Err(OrmError::new("migration name cannot be empty"));
    }

    let slug = slugify(name);
    let timestamp = migration_timestamp()?;
    let id = format!("{timestamp}_{slug}");
    let migrations_dir = root.join(MIGRATIONS_DIR);
    let directory = migrations_dir.join(&id);

    fs::create_dir_all(&directory)
        .map_err(|_| OrmError::new("failed to create migration directory"))?;
    fs::write(directory.join("up.sql"), initial_up_sql_template(&id))
        .map_err(|_| OrmError::new("failed to write migration up.sql"))?;
    fs::write(directory.join("down.sql"), initial_down_sql_template(&id))
        .map_err(|_| OrmError::new("failed to write migration down.sql"))?;
    write_model_snapshot(&directory.join("model_snapshot.json"), snapshot)?;

    Ok(MigrationScaffold {
        id,
        name: name.to_string(),
        directory,
    })
}

fn initial_up_sql_template(id: &str) -> String {
    format!("-- Migration: {id}\n-- SQL Server DDL for this migration.\n")
}

fn initial_down_sql_template(id: &str) -> String {
    format!(
        "-- Migration: {id}\n-- Manual rollback SQL for this editable migration.\n-- The current MVP does not execute down.sql automatically.\n"
    )
}

pub fn write_model_snapshot(path: &Path, snapshot: &ModelSnapshot) -> Result<(), OrmError> {
    fs::write(path, snapshot.to_json_pretty()?)
        .map_err(|_| OrmError::new("failed to write migration model snapshot"))
}

pub fn write_migration_up_sql(path: &Path, sql_statements: &[String]) -> Result<(), OrmError> {
    let sql = if sql_statements.is_empty() {
        String::from("-- No schema changes detected.\n")
    } else {
        let mut sql = sql_statements.join(";\n\n");
        sql.push_str(";\n");
        sql
    };

    fs::write(path, sql).map_err(|_| OrmError::new("failed to write migration up.sql"))
}

pub fn write_migration_down_sql(path: &Path, sql_statements: &[String]) -> Result<(), OrmError> {
    let sql = if sql_statements.is_empty() {
        String::from("-- No reversible schema changes detected.\n")
    } else {
        let mut sql = sql_statements.join(";\n\n");
        sql.push_str(";\n");
        sql
    };

    fs::write(path, sql).map_err(|_| OrmError::new("failed to write migration down.sql"))
}

pub fn read_model_snapshot(path: &Path) -> Result<ModelSnapshot, OrmError> {
    let json = fs::read_to_string(path)
        .map_err(|_| OrmError::new("failed to read migration model snapshot"))?;
    ModelSnapshot::from_json(&json)
}

pub fn list_migrations(root: &Path) -> Result<Vec<MigrationEntry>, OrmError> {
    let migrations_dir = root.join(MIGRATIONS_DIR);
    if !migrations_dir.exists() {
        return Ok(Vec::new());
    }

    let mut entries = fs::read_dir(&migrations_dir)
        .map_err(|_| OrmError::new("failed to read migrations directory"))?
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().map(|kind| kind.is_dir()).unwrap_or(false))
        .filter_map(|entry| parse_migration_entry(entry.path()))
        .collect::<Vec<_>>();

    entries.sort_by(|left, right| left.id.cmp(&right.id));
    Ok(entries)
}

pub fn latest_migration(root: &Path) -> Result<Option<MigrationEntry>, OrmError> {
    Ok(list_migrations(root)?.into_iter().last())
}

pub fn read_latest_model_snapshot(
    root: &Path,
) -> Result<Option<(MigrationEntry, ModelSnapshot)>, OrmError> {
    let Some(migration) = latest_migration(root)? else {
        return Ok(None);
    };

    let snapshot = read_model_snapshot(&migration.snapshot_path)?;
    Ok(Some((migration, snapshot)))
}

pub fn build_database_update_script(
    root: &Path,
    history_table_sql: &str,
) -> Result<String, OrmError> {
    let migrations = list_migrations(root)?;
    let mut script = vec![
        "-- sql-orm database update".to_string(),
        "SET ANSI_NULLS ON;".to_string(),
        "SET ANSI_PADDING ON;".to_string(),
        "SET ANSI_WARNINGS ON;".to_string(),
        "SET ARITHABORT ON;".to_string(),
        "SET CONCAT_NULL_YIELDS_NULL ON;".to_string(),
        "SET QUOTED_IDENTIFIER ON;".to_string(),
        "SET NUMERIC_ROUNDABORT OFF;".to_string(),
        history_table_sql.to_string(),
    ];

    for migration in migrations {
        let up_sql = fs::read_to_string(&migration.up_sql_path)
            .map_err(|_| OrmError::new("failed to read migration up.sql"))?;
        let checksum = checksum_hex(up_sql.as_bytes());
        let statements = split_sql_statements(&up_sql);
        let body = if statements.is_empty() {
            String::new()
        } else {
            statements
                .iter()
                .map(|statement| format!("    EXEC(N'{}');", escape_sql_literal(statement)))
                .collect::<Vec<_>>()
                .join("\n")
                + "\n"
        };
        script.push(render_idempotent_migration_block(
            &migration.id,
            &migration.name,
            &checksum,
            &body,
        ));
    }

    Ok(script.join("\n\n"))
}

fn render_idempotent_migration_block(id: &str, name: &str, checksum: &str, body: &str) -> String {
    format!(
        "IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'{id}' AND [checksum] <> N'{checksum}')\nBEGIN\n    THROW 50001, N'sql-orm migration checksum mismatch for {id}', 1;\nEND\n\nIF NOT EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'{id}')\nBEGIN\n    BEGIN TRY\n        BEGIN TRANSACTION;\n{body}        INSERT INTO [dbo].[__sql_orm_migrations] ([id], [name], [checksum], [orm_version]) VALUES (N'{id}', N'{name}', N'{checksum}', N'{version}');\n        COMMIT TRANSACTION;\n    END TRY\n    BEGIN CATCH\n        IF XACT_STATE() <> 0\n            ROLLBACK TRANSACTION;\n        THROW;\n    END CATCH\nEND",
        id = id,
        name = name,
        checksum = checksum,
        version = ORM_VERSION,
        body = body,
    )
}

fn parse_migration_entry(path: PathBuf) -> Option<MigrationEntry> {
    let file_name = path.file_name()?.to_str()?;
    let (timestamp, slug) = file_name.split_once('_')?;
    if timestamp.is_empty() || slug.is_empty() {
        return None;
    }

    Some(MigrationEntry {
        id: file_name.to_string(),
        name: slug.replace('_', " "),
        up_sql_path: path.join("up.sql"),
        down_sql_path: path.join("down.sql"),
        snapshot_path: path.join("model_snapshot.json"),
        directory: path,
    })
}

fn migration_timestamp() -> Result<String, OrmError> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|_| OrmError::new("system clock is before UNIX_EPOCH"))?;
    Ok(duration.as_nanos().to_string())
}

fn slugify(name: &str) -> String {
    let mut slug = String::new();
    let mut previous_was_separator = false;

    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            previous_was_separator = false;
        } else if !previous_was_separator {
            slug.push('_');
            previous_was_separator = true;
        }
    }

    slug.trim_matches('_').to_string()
}

fn checksum_hex(bytes: &[u8]) -> String {
    let mut hash = 0xcbf29ce484222325u64;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(0x100000001b3);
    }

    format!("{hash:016x}")
}

fn escape_sql_literal(sql: &str) -> String {
    sql.replace('\'', "''")
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    sql.split(';')
        .map(str::trim)
        .filter(|statement| !statement.is_empty())
        .filter(|statement| {
            statement.lines().any(|line| {
                let trimmed = line.trim();
                !trimmed.is_empty() && !trimmed.starts_with("--")
            })
        })
        .map(|statement| format!("{statement};"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::{
        build_database_update_script, create_migration_scaffold,
        create_migration_scaffold_with_snapshot, latest_migration, list_migrations,
        read_latest_model_snapshot, read_model_snapshot, write_migration_down_sql,
        write_migration_up_sql, write_model_snapshot,
    };
    use crate::{ModelSnapshot, SchemaSnapshot};
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_project_root() -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = std::env::temp_dir().join(format!("sql_orm_migrate_{unique}"));
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn creates_scaffolded_migration_files() {
        let root = temp_project_root();

        let scaffold = create_migration_scaffold(&root, "Create Customers").unwrap();

        assert!(scaffold.id.contains("create_customers"));
        assert!(scaffold.up_sql_path().exists());
        assert!(scaffold.down_sql_path().exists());
        assert!(scaffold.snapshot_path().exists());
        assert!(!scaffold.directory.join("migration.rs").exists());

        assert_eq!(
            fs::read_to_string(scaffold.up_sql_path()).unwrap(),
            format!(
                "-- Migration: {}\n-- SQL Server DDL for this migration.\n",
                scaffold.id
            )
        );
        assert_eq!(
            fs::read_to_string(scaffold.down_sql_path()).unwrap(),
            format!(
                "-- Migration: {}\n-- Manual rollback SQL for this editable migration.\n-- The current MVP does not execute down.sql automatically.\n",
                scaffold.id
            )
        );

        let snapshot = read_model_snapshot(&scaffold.snapshot_path()).unwrap();
        assert_eq!(snapshot, ModelSnapshot::default());
    }

    #[test]
    fn writes_and_reads_model_snapshot_artifact() {
        let root = temp_project_root();
        let snapshot_path = root.join("model_snapshot.json");
        let snapshot = ModelSnapshot::new(vec![SchemaSnapshot::new("sales", Vec::new())]);

        write_model_snapshot(&snapshot_path, &snapshot).unwrap();

        assert_eq!(read_model_snapshot(&snapshot_path).unwrap(), snapshot);
    }

    #[test]
    fn writes_generated_down_sql_artifact() {
        let root = temp_project_root();
        let down_sql_path = root.join("down.sql");

        write_migration_down_sql(
            &down_sql_path,
            &[
                "DROP TABLE [sales].[customers]".to_string(),
                "DROP SCHEMA [sales]".to_string(),
            ],
        )
        .unwrap();

        assert_eq!(
            fs::read_to_string(down_sql_path).unwrap(),
            "DROP TABLE [sales].[customers];\n\nDROP SCHEMA [sales];\n"
        );
    }

    #[test]
    fn creates_scaffold_with_provided_model_snapshot() {
        let root = temp_project_root();
        let snapshot = ModelSnapshot::new(vec![SchemaSnapshot::new("sales", Vec::new())]);

        let scaffold =
            create_migration_scaffold_with_snapshot(&root, "Create Sales", &snapshot).unwrap();

        assert_eq!(
            read_model_snapshot(&scaffold.snapshot_path()).unwrap(),
            snapshot
        );
    }

    #[test]
    fn lists_migrations_in_sorted_order() {
        let root = temp_project_root();
        let migrations_dir = root.join("migrations");
        fs::create_dir_all(migrations_dir.join("200_create_orders")).unwrap();
        fs::create_dir_all(migrations_dir.join("100_create_customers")).unwrap();

        let migrations = list_migrations(&root).unwrap();

        assert_eq!(migrations.len(), 2);
        assert_eq!(migrations[0].id, "100_create_customers");
        assert_eq!(migrations[1].id, "200_create_orders");
    }

    #[test]
    fn returns_latest_migration_in_lexical_order() {
        let root = temp_project_root();
        let migrations_dir = root.join("migrations");
        fs::create_dir_all(migrations_dir.join("100_create_customers")).unwrap();
        fs::create_dir_all(migrations_dir.join("200_create_orders")).unwrap();

        let latest = latest_migration(&root).unwrap().unwrap();

        assert_eq!(latest.id, "200_create_orders");
    }

    #[test]
    fn reads_latest_model_snapshot_from_last_local_migration() {
        let root = temp_project_root();
        let older_dir = root.join("migrations/100_create_customers");
        let newer_dir = root.join("migrations/200_create_orders");
        fs::create_dir_all(&older_dir).unwrap();
        fs::create_dir_all(&newer_dir).unwrap();
        fs::write(older_dir.join("up.sql"), "-- noop").unwrap();
        fs::write(older_dir.join("down.sql"), "-- noop").unwrap();
        fs::write(
            older_dir.join("model_snapshot.json"),
            "{\n  \"schemas\": []\n}\n",
        )
        .unwrap();
        fs::write(newer_dir.join("up.sql"), "-- noop").unwrap();
        fs::write(newer_dir.join("down.sql"), "-- noop").unwrap();
        fs::write(
            newer_dir.join("model_snapshot.json"),
            "{\n  \"schemas\": [\n    {\n      \"name\": \"sales\",\n      \"tables\": []\n    }\n  ]\n}\n",
        )
        .unwrap();

        let (migration, snapshot) = read_latest_model_snapshot(&root).unwrap().unwrap();

        assert_eq!(migration.id, "200_create_orders");
        assert!(snapshot.schema("sales").is_some());
    }

    #[test]
    fn builds_database_update_script_with_history_inserts() {
        let root = temp_project_root();
        let scaffold = create_migration_scaffold(&root, "Create Customers").unwrap();
        fs::write(
            scaffold.directory.join("up.sql"),
            "CREATE SCHEMA [sales];\nCREATE TABLE [sales].[customers] ([id] bigint NOT NULL);",
        )
        .unwrap();

        let script =
            build_database_update_script(&root, "CREATE TABLE [dbo].[__sql_orm_migrations] (...);")
                .unwrap();

        assert!(script.contains("CREATE TABLE [dbo].[__sql_orm_migrations]"));
        assert!(script.contains("SET ANSI_NULLS ON;"));
        assert!(script.contains("SET QUOTED_IDENTIFIER ON;"));
        assert!(script.contains("SET NUMERIC_ROUNDABORT OFF;"));
        assert!(script.contains("IF NOT EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations]"));
        assert!(script.contains("IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations]"));
        assert!(script.contains("THROW 50001, N'sql-orm migration checksum mismatch"));
        assert!(script.contains("BEGIN TRY"));
        assert!(script.contains("BEGIN TRANSACTION;"));
        assert!(script.contains("EXEC(N'CREATE SCHEMA [sales];');"));
        assert!(
            script.contains("EXEC(N'CREATE TABLE [sales].[customers] ([id] bigint NOT NULL);');")
        );
        assert!(script.contains("INSERT INTO [dbo].[__sql_orm_migrations]"));
        assert!(script.contains("COMMIT TRANSACTION;"));
        assert!(script.contains("ROLLBACK TRANSACTION;"));
    }

    #[test]
    fn builds_database_update_script_without_empty_exec_blocks() {
        let root = temp_project_root();
        let scaffold = create_migration_scaffold(&root, "Noop").unwrap();
        fs::write(
            scaffold.directory.join("up.sql"),
            "-- comment only migration\n\n-- still intentionally empty\n",
        )
        .unwrap();

        let script =
            build_database_update_script(&root, "CREATE TABLE [dbo].[__sql_orm_migrations] (...);")
                .unwrap();

        assert!(!script.contains("EXEC(N'');"));
        assert!(script.contains("INSERT INTO [dbo].[__sql_orm_migrations]"));
    }

    #[test]
    fn writes_up_sql_from_compiled_statements() {
        let root = temp_project_root();
        let up_sql_path = root.join("up.sql");

        write_migration_up_sql(
            &up_sql_path,
            &[
                "CREATE SCHEMA [sales]".to_string(),
                "CREATE TABLE [sales].[customers] ([id] bigint NOT NULL)".to_string(),
            ],
        )
        .unwrap();

        let sql = fs::read_to_string(up_sql_path).unwrap();

        assert_eq!(
            sql,
            "CREATE SCHEMA [sales];\n\nCREATE TABLE [sales].[customers] ([id] bigint NOT NULL);\n"
        );
    }

    #[test]
    fn writes_noop_up_sql_when_no_statements_exist() {
        let root = temp_project_root();
        let up_sql_path = root.join("up.sql");

        write_migration_up_sql(&up_sql_path, &[]).unwrap();

        assert_eq!(
            fs::read_to_string(up_sql_path).unwrap(),
            "-- No schema changes detected.\n"
        );
    }
}
