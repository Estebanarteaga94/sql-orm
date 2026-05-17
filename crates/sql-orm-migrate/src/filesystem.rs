use crate::ModelSnapshot;
use sql_orm_core::{OrmError, quote_sql_string_literal};
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
                .map(|statement| format!("    EXEC({});", quote_sql_string_literal(statement)))
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

pub fn build_database_downgrade_script(
    root: &Path,
    history_table_sql: &str,
    target: &str,
) -> Result<String, OrmError> {
    let target = target.trim();
    if target.is_empty() {
        return Err(OrmError::new(
            "database downgrade requires an explicit target",
        ));
    }

    let migrations = list_migrations(root)?;
    if target != "0" && !migrations.iter().any(|migration| migration.id == target) {
        return Err(OrmError::new(format!(
            "database downgrade target `{target}` is not a known local migration"
        )));
    }

    let rollback_migrations = migrations
        .iter()
        .filter(|migration| target == "0" || migration.id.as_str() > target)
        .rev()
        .map(|migration| {
            let up_sql = fs::read_to_string(&migration.up_sql_path).map_err(|_| {
                OrmError::new(format!(
                    "database downgrade migration `{}` is missing local up.sql for checksum validation",
                    migration.id
                ))
            })?;
            let down_sql = fs::read_to_string(&migration.down_sql_path).map_err(|_| {
                OrmError::new(format!(
                    "database downgrade migration `{}` is missing local down.sql",
                    migration.id
                ))
            })?;
            if is_unresolved_down_sql_template(&down_sql) {
                return Err(OrmError::new(format!(
                    "database downgrade migration `{}` has no reversible payload in down.sql; edit down.sql with executable rollback SQL",
                    migration.id
                )));
            }
            let down_statements = split_sql_statements(&down_sql);
            if down_statements.is_empty() {
                return Err(OrmError::new(format!(
                    "database downgrade migration `{}` has no executable down.sql statements",
                    migration.id
                )));
            }

            Ok(DowngradeMigrationBlock {
                id: migration.id.clone(),
                checksum: checksum_hex(up_sql.as_bytes()),
                down_statements,
            })
        })
        .collect::<Result<Vec<_>, OrmError>>()?;

    let mut script = vec![
        "-- sql-orm database downgrade".to_string(),
        "SET ANSI_NULLS ON;".to_string(),
        "SET ANSI_PADDING ON;".to_string(),
        "SET ANSI_WARNINGS ON;".to_string(),
        "SET ARITHABORT ON;".to_string(),
        "SET CONCAT_NULL_YIELDS_NULL ON;".to_string(),
        "SET QUOTED_IDENTIFIER ON;".to_string(),
        "SET NUMERIC_ROUNDABORT OFF;".to_string(),
        history_table_sql.to_string(),
        render_downgrade_history_guard(&migrations, target),
    ];

    for migration in rollback_migrations {
        script.push(render_idempotent_downgrade_block(&migration));
    }

    Ok(script.join("\n\n"))
}

fn render_idempotent_migration_block(id: &str, name: &str, checksum: &str, body: &str) -> String {
    let id_literal = quote_sql_string_literal(id);
    let name_literal = quote_sql_string_literal(name);
    let checksum_literal = quote_sql_string_literal(checksum);
    let version_literal = quote_sql_string_literal(ORM_VERSION);
    let checksum_mismatch_message =
        quote_sql_string_literal(&format!("sql-orm migration checksum mismatch for {id}"));

    format!(
        "IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = {id_literal} AND [checksum] <> {checksum_literal})\nBEGIN\n    THROW 50001, {checksum_mismatch_message}, 1;\nEND\n\nIF NOT EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = {id_literal})\nBEGIN\n    BEGIN TRY\n        BEGIN TRANSACTION;\n{body}        INSERT INTO [dbo].[__sql_orm_migrations] ([id], [name], [checksum], [orm_version]) VALUES ({id_literal}, {name_literal}, {checksum_literal}, {version_literal});\n        COMMIT TRANSACTION;\n    END TRY\n    BEGIN CATCH\n        IF XACT_STATE() <> 0\n            ROLLBACK TRANSACTION;\n        THROW;\n    END CATCH\nEND",
        body = body,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct DowngradeMigrationBlock {
    id: String,
    checksum: String,
    down_statements: Vec<String>,
}

fn render_downgrade_history_guard(migrations: &[MigrationEntry], target: &str) -> String {
    let local_history_guard = if migrations.is_empty() {
        "IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations])\nBEGIN\n    THROW 50002, N'sql-orm migration history contains entries missing from local migrations', 1;\nEND".to_string()
    } else {
        let known_ids = migrations
            .iter()
            .map(|migration| quote_sql_string_literal(&migration.id))
            .collect::<Vec<_>>()
            .join(", ");
        format!(
            "IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] NOT IN ({known_ids}))\nBEGIN\n    THROW 50002, N'sql-orm migration history contains entries missing from local migrations', 1;\nEND",
            known_ids = known_ids,
        )
    };
    let target_literal = quote_sql_string_literal(target);
    let target_guard = if target == "0" {
        String::new()
    } else {
        let target_error = quote_sql_string_literal(&format!(
            "sql-orm downgrade target {target} is not applied in migration history"
        ));
        format!(
            "\n\nIF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] > {target})\n   AND NOT EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = {target})\nBEGIN\n    THROW 50003, {target_error}, 1;\nEND",
            target = target_literal,
        )
    };

    format!(
        "{local_history_guard}{target_guard}",
        local_history_guard = local_history_guard,
        target_guard = target_guard,
    )
}

fn render_idempotent_downgrade_block(migration: &DowngradeMigrationBlock) -> String {
    let id = quote_sql_string_literal(&migration.id);
    let checksum = quote_sql_string_literal(&migration.checksum);
    let checksum_mismatch_message = quote_sql_string_literal(&format!(
        "sql-orm migration checksum mismatch for {}",
        migration.id
    ));
    let body = migration
        .down_statements
        .iter()
        .map(|statement| format!("        EXEC({});", quote_sql_string_literal(statement)))
        .collect::<Vec<_>>()
        .join("\n");

    format!(
        "IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = {id} AND [checksum] <> {checksum})\nBEGIN\n    THROW 50001, {checksum_mismatch_message}, 1;\nEND\n\nIF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] = {id})\nBEGIN\n    BEGIN TRY\n        BEGIN TRANSACTION;\n{body}\n        DELETE FROM [dbo].[__sql_orm_migrations] WHERE [id] = {id};\n        COMMIT TRANSACTION;\n    END TRY\n    BEGIN CATCH\n        IF XACT_STATE() <> 0\n            ROLLBACK TRANSACTION;\n        THROW;\n    END CATCH\nEND",
        id = id,
        checksum = checksum,
        checksum_mismatch_message = checksum_mismatch_message,
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

fn is_unresolved_down_sql_template(sql: &str) -> bool {
    let mut saw_executable_statement = false;
    let mut saw_unresolved_marker = false;

    for line in sql.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }

        if trimmed.starts_with("--") {
            let lower = trimmed.to_ascii_lowercase();
            if lower.contains("manual rollback sql")
                || lower.contains("does not execute down.sql automatically")
                || lower.contains("no reversible schema changes detected")
            {
                saw_unresolved_marker = true;
            }
            continue;
        }

        saw_executable_statement = true;
    }

    saw_unresolved_marker && !saw_executable_statement
}

fn split_sql_statements(sql: &str) -> Vec<String> {
    let mut statements = Vec::new();
    let mut current = String::new();
    let mut line = String::new();
    let mut chars = sql.chars().peekable();
    let mut state = SqlScriptState::Default;

    while let Some(ch) = chars.next() {
        match state {
            SqlScriptState::Default => {
                if ch != '\r' && ch != '\n' {
                    line.push(ch);
                }

                match ch {
                    '\'' => {
                        current.push(ch);
                        state = SqlScriptState::StringLiteral;
                    }
                    '[' => {
                        current.push(ch);
                        state = SqlScriptState::BracketIdentifier;
                    }
                    '"' => {
                        current.push(ch);
                        state = SqlScriptState::DoubleQuotedIdentifier;
                    }
                    '-' if chars.peek() == Some(&'-') => {
                        current.push(ch);
                        current.push(chars.next().expect("peeked dash"));
                        state = SqlScriptState::LineComment;
                    }
                    '/' if chars.peek() == Some(&'*') => {
                        current.push(ch);
                        current.push(chars.next().expect("peeked star"));
                        state = SqlScriptState::BlockComment;
                    }
                    ';' => {
                        current.push(ch);
                        push_sql_statement(&mut statements, &mut current);
                    }
                    '\n' => {
                        if is_go_batch_separator(&line) {
                            remove_current_line(&mut current);
                            push_sql_statement(&mut statements, &mut current);
                        } else {
                            current.push(ch);
                        }
                        line.clear();
                    }
                    '\r' => {
                        current.push(ch);
                    }
                    _ => {
                        current.push(ch);
                    }
                }
            }
            SqlScriptState::StringLiteral => {
                current.push(ch);
                if ch == '\'' {
                    if chars.peek() == Some(&'\'') {
                        current.push(chars.next().expect("peeked quote"));
                    } else {
                        state = SqlScriptState::Default;
                    }
                }
                if ch == '\n' {
                    line.clear();
                }
            }
            SqlScriptState::BracketIdentifier => {
                current.push(ch);
                if ch == ']' {
                    if chars.peek() == Some(&']') {
                        current.push(chars.next().expect("peeked bracket"));
                    } else {
                        state = SqlScriptState::Default;
                    }
                }
                if ch == '\n' {
                    line.clear();
                }
            }
            SqlScriptState::DoubleQuotedIdentifier => {
                current.push(ch);
                if ch == '"' {
                    if chars.peek() == Some(&'"') {
                        current.push(chars.next().expect("peeked double quote"));
                    } else {
                        state = SqlScriptState::Default;
                    }
                }
                if ch == '\n' {
                    line.clear();
                }
            }
            SqlScriptState::LineComment => {
                current.push(ch);
                if ch == '\n' {
                    state = SqlScriptState::Default;
                    line.clear();
                }
            }
            SqlScriptState::BlockComment => {
                current.push(ch);
                if ch == '*' && chars.peek() == Some(&'/') {
                    current.push(chars.next().expect("peeked slash"));
                    state = SqlScriptState::Default;
                }
                if ch == '\n' {
                    line.clear();
                }
            }
        }
    }

    if is_go_batch_separator(&line) {
        remove_current_line(&mut current);
    }
    push_sql_statement(&mut statements, &mut current);

    statements
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SqlScriptState {
    Default,
    StringLiteral,
    BracketIdentifier,
    DoubleQuotedIdentifier,
    LineComment,
    BlockComment,
}

fn push_sql_statement(statements: &mut Vec<String>, current: &mut String) {
    let statement = current.trim();
    if !statement.is_empty() && has_executable_sql(statement) {
        statements.push(statement.to_string());
    }
    current.clear();
}

fn remove_current_line(current: &mut String) {
    match current.rfind('\n') {
        Some(index) => current.truncate(index + 1),
        None => current.clear(),
    }
}

fn is_go_batch_separator(line: &str) -> bool {
    line.trim().eq_ignore_ascii_case("GO")
}

fn has_executable_sql(statement: &str) -> bool {
    let mut chars = statement.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '-' if chars.peek() == Some(&'-') => {
                chars.next();
                for comment_ch in chars.by_ref() {
                    if comment_ch == '\n' {
                        break;
                    }
                }
            }
            '/' if chars.peek() == Some(&'*') => {
                chars.next();
                let mut previous = '\0';
                for comment_ch in chars.by_ref() {
                    if previous == '*' && comment_ch == '/' {
                        break;
                    }
                    previous = comment_ch;
                }
            }
            _ if ch.is_whitespace() || ch == ';' => {}
            _ => return true,
        }
    }

    false
}

#[cfg(test)]
mod tests {
    use super::{
        build_database_downgrade_script, build_database_update_script, checksum_hex,
        create_migration_scaffold, create_migration_scaffold_with_snapshot, latest_migration,
        list_migrations, read_latest_model_snapshot, read_model_snapshot, split_sql_statements,
        write_migration_down_sql, write_migration_up_sql, write_model_snapshot,
    };
    use crate::{ModelSnapshot, SchemaSnapshot};
    use std::fs;
    use std::path::{Path, PathBuf};
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

    fn write_local_migration(root: &Path, id: &str, up_sql: &str, down_sql: &str) {
        let migration_dir = root.join("migrations").join(id);
        fs::create_dir_all(&migration_dir).unwrap();
        fs::write(migration_dir.join("up.sql"), up_sql).unwrap();
        fs::write(migration_dir.join("down.sql"), down_sql).unwrap();
        fs::write(
            migration_dir.join("model_snapshot.json"),
            "{ \"schemas\": [] }",
        )
        .unwrap();
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
    fn database_update_script_escapes_single_quotes_inside_exec_blocks() {
        let root = temp_project_root();
        let scaffold = create_migration_scaffold(&root, "Quoted Literal").unwrap();
        fs::write(
            scaffold.directory.join("up.sql"),
            "INSERT INTO [dbo].[messages] ([body]) VALUES (N'O''Brien');",
        )
        .unwrap();

        let script =
            build_database_update_script(&root, "CREATE TABLE [dbo].[__sql_orm_migrations] (...);")
                .unwrap();

        assert!(
            script.contains(
                "EXEC(N'INSERT INTO [dbo].[messages] ([body]) VALUES (N''O''''Brien'');');"
            )
        );
    }

    #[test]
    fn split_sql_statements_respects_literals_comments_and_go_batches() {
        let statements = split_sql_statements(
            "CREATE TABLE [dbo].[semi;colon] ([body] nvarchar(200));\n\
             INSERT INTO [dbo].[semi;colon] ([body]) VALUES (N'one;two -- not comment');\n\
             -- GO is ignored inside a line comment\n\
             /* semicolon ; and GO are ignored inside block comments */\n\
             GO\n\
             SELECT N'GO; still literal';\n",
        );

        assert_eq!(
            statements,
            vec![
                "CREATE TABLE [dbo].[semi;colon] ([body] nvarchar(200));",
                "INSERT INTO [dbo].[semi;colon] ([body]) VALUES (N'one;two -- not comment');",
                "SELECT N'GO; still literal';",
            ]
        );
    }

    #[test]
    fn split_sql_statements_discards_comment_only_batches() {
        let statements = split_sql_statements(
            "-- comment only\n\
             GO\n\
             /* block comment ; only */\n\
             GO\n\
             CREATE SCHEMA [sales]\n\
             GO\n",
        );

        assert_eq!(statements, vec!["CREATE SCHEMA [sales]"]);
    }

    #[test]
    fn database_update_script_splits_go_batches_without_splitting_literals() {
        let root = temp_project_root();
        let scaffold = create_migration_scaffold(&root, "Go Batch").unwrap();
        fs::write(
            scaffold.directory.join("up.sql"),
            "CREATE SCHEMA [sales]\nGO\nINSERT INTO [dbo].[messages] ([body]) VALUES (N'a;b');",
        )
        .unwrap();

        let script =
            build_database_update_script(&root, "CREATE TABLE [dbo].[__sql_orm_migrations] (...);")
                .unwrap();

        assert!(script.contains("EXEC(N'CREATE SCHEMA [sales]');"));
        assert!(
            script.contains("EXEC(N'INSERT INTO [dbo].[messages] ([body]) VALUES (N''a;b'');');")
        );
        assert!(!script.contains("EXEC(N'GO');"));
    }

    #[test]
    fn builds_database_downgrade_script_in_reverse_order() {
        let root = temp_project_root();
        write_local_migration(
            &root,
            "100_create_customers",
            "CREATE TABLE [sales].[customers] ([id] bigint NOT NULL);",
            "DROP TABLE [sales].[customers];",
        );
        write_local_migration(
            &root,
            "200_create_orders",
            "CREATE TABLE [sales].[orders] ([id] bigint NOT NULL);",
            "DROP TABLE [sales].[orders];",
        );
        write_local_migration(
            &root,
            "300_create_lines",
            "CREATE TABLE [sales].[order_lines] ([id] bigint NOT NULL);",
            "DROP TABLE [sales].[order_lines];",
        );

        let script = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "100_create_customers",
        )
        .unwrap();

        let lines_pos = script.find("DROP TABLE [sales].[order_lines]").unwrap();
        let orders_pos = script.find("DROP TABLE [sales].[orders]").unwrap();
        assert!(lines_pos < orders_pos);
        assert!(!script.contains("DROP TABLE [sales].[customers]"));
        assert!(script.contains("CREATE TABLE [dbo].[__sql_orm_migrations]"));
        assert!(
            script.contains(
                "IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations] WHERE [id] NOT IN"
            )
        );
        assert!(script.contains("sql-orm downgrade target 100_create_customers is not applied"));
        assert!(
            script.contains(
                "THROW 50001, N'sql-orm migration checksum mismatch for 300_create_lines'"
            )
        );
        assert!(script.contains("BEGIN TRANSACTION;"));
        assert!(script.contains(
            "DELETE FROM [dbo].[__sql_orm_migrations] WHERE [id] = N'300_create_lines';"
        ));
        assert!(script.contains("ROLLBACK TRANSACTION;"));
    }

    #[test]
    fn builds_database_downgrade_script_to_empty_database_sentinel() {
        let root = temp_project_root();
        write_local_migration(
            &root,
            "100_create_customers",
            "CREATE TABLE [sales].[customers] ([id] bigint NOT NULL);",
            "DROP TABLE [sales].[customers];",
        );

        let script = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "0",
        )
        .unwrap();

        assert!(script.contains("DROP TABLE [sales].[customers]"));
        assert!(!script.contains("downgrade target 0 is not applied"));
    }

    #[test]
    fn database_downgrade_script_with_no_local_migrations_rejects_any_history_rows() {
        let root = temp_project_root();

        let script = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "0",
        )
        .unwrap();

        assert!(script.contains("IF EXISTS (SELECT 1 FROM [dbo].[__sql_orm_migrations])"));
        assert!(script.contains("history contains entries missing from local migrations"));
        assert!(!script.contains("NOT IN (NULL)"));
    }

    #[test]
    fn database_downgrade_requires_explicit_target_and_renders_checksum_guards() {
        let root = temp_project_root();
        let orders_up = "CREATE TABLE [sales].[orders] ([id] bigint NOT NULL);";
        write_local_migration(
            &root,
            "100_create_customers",
            "CREATE TABLE [sales].[customers] ([id] bigint NOT NULL);",
            "DROP TABLE [sales].[customers];",
        );
        write_local_migration(
            &root,
            "200_create_orders",
            orders_up,
            "DROP TABLE [sales].[orders];",
        );

        let error = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "   ",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("database downgrade requires an explicit target")
        );

        let script = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "100_create_customers",
        )
        .unwrap();
        let orders_checksum = checksum_hex(orders_up.as_bytes());
        let checksum_guard =
            format!("WHERE [id] = N'200_create_orders' AND [checksum] <> N'{orders_checksum}'");

        assert!(script.contains("N'100_create_customers', N'200_create_orders'"));
        assert!(script.contains(&checksum_guard));
        assert!(
            script
                .find("sql-orm migration checksum mismatch for 200_create_orders")
                .unwrap()
                < script.find("DROP TABLE [sales].[orders]").unwrap()
        );
        assert_eq!(script.matches("DROP TABLE [sales].[orders]").count(), 1);
        assert!(!script.contains("DROP TABLE [sales].[customers]"));
    }

    #[test]
    fn database_downgrade_rejects_unknown_target_and_empty_down_sql() {
        let root = temp_project_root();
        write_local_migration(
            &root,
            "100_create_customers",
            "CREATE TABLE [sales].[customers] ([id] bigint NOT NULL);",
            "DROP TABLE [sales].[customers];",
        );

        let error = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "999_missing",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("target `999_missing` is not a known local migration")
        );

        write_local_migration(
            &root,
            "200_create_orders",
            "CREATE TABLE [sales].[orders] ([id] bigint NOT NULL);",
            "-- manual rollback pending\n",
        );

        let error = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "100_create_customers",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("migration `200_create_orders` has no executable down.sql statements")
        );
    }

    #[test]
    fn database_downgrade_reports_missing_artifacts_and_unresolved_templates() {
        let root = temp_project_root();
        write_local_migration(
            &root,
            "100_create_customers",
            "CREATE TABLE [sales].[customers] ([id] bigint NOT NULL);",
            "DROP TABLE [sales].[customers];",
        );
        write_local_migration(
            &root,
            "200_create_orders",
            "CREATE TABLE [sales].[orders] ([id] bigint NOT NULL);",
            "DROP TABLE [sales].[orders];",
        );
        fs::remove_file(root.join("migrations/200_create_orders/down.sql")).unwrap();

        let error = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "100_create_customers",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("migration `200_create_orders` is missing local down.sql")
        );

        fs::write(
            root.join("migrations/200_create_orders/down.sql"),
            "-- Migration: 200_create_orders\n-- Manual rollback SQL for this editable migration.\n-- The current MVP does not execute down.sql automatically.\n",
        )
        .unwrap();

        let error = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "100_create_customers",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("migration `200_create_orders` has no reversible payload in down.sql")
        );

        fs::remove_file(root.join("migrations/200_create_orders/up.sql")).unwrap();
        fs::write(
            root.join("migrations/200_create_orders/down.sql"),
            "DROP TABLE [sales].[orders];",
        )
        .unwrap();

        let error = build_database_downgrade_script(
            &root,
            "CREATE TABLE [dbo].[__sql_orm_migrations] (...);",
            "100_create_customers",
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("migration `200_create_orders` is missing local up.sql")
        );
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
