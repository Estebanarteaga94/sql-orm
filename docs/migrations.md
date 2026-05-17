# Migrations

This guide describes the migration surface currently available in the repository.

See also [Core concepts](core-concepts.md).

## Current Surface

- `ModelSnapshot` can be generated from entity metadata.
- Snapshots serialize to and from JSON.
- Diffing supports schemas, tables, columns, indexes, foreign keys, and explicit renames.
- `migration add` can consume a current snapshot from a file or from a consumer snapshot binary.
- `up.sql` is generated from SQL Server migration operations.
- `down.sql` is generated when all operations are reversible with the available payload.
- Destructive changes are blocked by default unless explicitly allowed.
- `database update` generates an idempotent script and can execute it with `--execute`.
- `database downgrade --target <MigrationId|0>` generates an idempotent rollback script from local `down.sql` artifacts and can execute it with `--execute`.

## Recommended Flow

1. Change Rust entities and metadata first.
2. Export the current model snapshot.
3. Run `migration add`.
4. Review `up.sql`, `down.sql`, and `model_snapshot.json`.
5. Commit the model change and migration together.
6. Run `database update` to generate or apply the script.

## Snapshot Binary

A consumer project can expose a binary that prints the current snapshot:

```rust
use sql_orm::prelude::*;

fn main() {
    print!(
        "{}",
        sql_orm::model_snapshot_json_from_source::<AppDbContext>()
            .expect("snapshot should serialize")
    );
}
```

Then create a migration:

```bash
sql-orm-cli migration add CreateCustomers \
  --manifest-path path/to/Cargo.toml \
  --snapshot-bin model_snapshot
```

## Migration Artifact

The current editable migration artifact is:

- `up.sql`
- `down.sql`
- `model_snapshot.json`

The master plan mentions `migration.rs`, but it is explicitly deferred until a Rust migration API can be designed without duplicating or contradicting the current snapshot/diff/DDL pipeline.

## Destructive Changes

`migration add` blocks destructive plans by default. The current detection includes:

- dropping a table;
- dropping a column;
- reducing column length;
- changing a column type;
- changing nullable to non-nullable without a default.

Use `--allow-destructive` only after reviewing the generated plan.

## `down.sql`

`down.sql` is generated only when the full plan can be reversed with the available payload.

Examples of reversible operations include:

- `CreateSchema`
- `CreateTable`
- `AddColumn`
- `AlterColumn`
- `RenameTable`
- `RenameColumn`
- `CreateIndex`
- `AddForeignKey`

Operations such as `DropTable`, `DropColumn`, `DropIndex`, and `DropForeignKey` remain manual because the current payload does not reconstruct the removed object.

## Listing Migrations

```bash
sql-orm-cli migration list
```

The output lists each local migration directory and its artifacts.

## Database Update

By default, `database update` prints the accumulated SQL script:

```bash
sql-orm-cli database update
```

You can archive or inspect the script before applying it:

```bash
sql-orm-cli database update > database_update.sql
```

To execute directly:

```bash
sql-orm-cli database update --execute \
  --connection-string "$DATABASE_URL"
```

Connection-string resolution order for `--execute`:

1. `--connection-string`
2. `DATABASE_URL`
3. `SQL_ORM_TEST_CONNECTION_STRING`

## Migration History

The generated script uses a `__sql_orm_migrations` history table. It records migration identity and checksum. Editing an already-applied migration is treated as drift and should fail intentionally.

## Database Downgrade

`database downgrade` rolls back applied migrations using the artifacts that already exist today:

- local migration directories under `migrations/`;
- `down.sql` for rollback SQL;
- `up.sql` checksums;
- `model_snapshot.json` for local ordering and identity;
- the existing `[dbo].[__sql_orm_migrations]` history table.

The command is script-first, matching `database update`:

```bash
sql-orm-cli database downgrade --target <MigrationId>
```

By default the command prints a SQL script:

```bash
sql-orm-cli database downgrade --target <MigrationId> > database_downgrade.sql
```

To execute directly:

```bash
sql-orm-cli database downgrade --target <MigrationId> --execute \
  --connection-string "$DATABASE_URL"
```

The target is the last migration that should remain applied after the downgrade. Rolling back everything requires the explicit sentinel target `0`; there is no implicit "previous migration" default.

The script generation flow is:

1. Read local migrations in ascending order.
2. Generate SQL guards that query `[dbo].[__sql_orm_migrations]`.
3. Validate that the requested target is explicit and known, unless it is the empty-database sentinel.
4. Select applied migrations after the target.
5. Process those migrations in reverse order.
6. For each migration, verify that the stored checksum matches the checksum of local `up.sql`.
7. Execute the local `down.sql` inside a transaction.
8. Delete the migration row from `[dbo].[__sql_orm_migrations]` only after `down.sql` succeeds.

This implementation does not introduce `migration.rs`. Hand-authored rollback logic belongs in `down.sql` for this phase.

Connection-string resolution order for `--execute` matches `database update`:

1. `--connection-string`
2. `DATABASE_URL`
3. `SQL_ORM_TEST_CONNECTION_STRING`

Safety requirements:

- reject missing target;
- reject unknown target;
- reject an applied migration that does not exist locally;
- reject missing or comment-only `down.sql`;
- reject unresolved scaffold/template `down.sql` that still contains only manual rollback guidance;
- reject checksum mismatch before running rollback SQL;
- run each migration rollback in its own transaction;
- fail clearly when rollback cannot be proven reversible from the local artifacts.

### Downgrade Safety Rules

`database downgrade` treats rollback as a potentially destructive operation and fails closed. These rules are implementation requirements, not optional documentation guidance.

Target rules:

- `--target` is required for both script generation and `--execute`.
- The target value must be either a known local migration id or the explicit empty-database sentinel `0`.
- The target is inclusive: after downgrade completes, that migration should still be present in `[dbo].[__sql_orm_migrations]`.
- A target newer than or equal to the latest applied migration is a no-op script, not an error.
- A target that exists locally but is not present in the applied history is ambiguous and must fail instead of guessing how far to rollback.
- A target that is not local and is not `0` must fail before reading or executing `down.sql`.
- The command does not support an implicit "previous" target.

History and checksum rules:

- Every applied migration selected for rollback must exist as a local migration directory.
- The local `up.sql` checksum must match the checksum stored in `[dbo].[__sql_orm_migrations]` before any corresponding `down.sql` is executed.
- Checksum mismatch must abort the entire downgrade script for that migration and all older migrations.
- A missing local migration or missing checksum must produce a deterministic error naming the migration id.
- Local migrations that are newer than the database history but are not applied must be ignored for rollback selection.

`down.sql` rules:

- `down.sql` must exist for every migration selected for rollback.
- `down.sql` must contain at least one executable statement after comments and blank lines are ignored.
- Template-only or comment-only `down.sql` must be treated as non-reversible.
- Missing `up.sql` must fail before rollback because the local checksum cannot be compared to migration history.
- The implementation must not infer reverse SQL from `model_snapshot.json` at downgrade time.
- The implementation must not read or require `migration.rs`.

Transaction rules:

- Each migration rollback must run in its own SQL Server transaction.
- The history row for that migration must be deleted in the same transaction as its `down.sql`.
- If any statement in `down.sql` fails, the transaction must roll back and the history row must remain.
- The script must stop at the first failed rollback; it must not continue to older migrations after an error.
- Nested user transactions are outside the first implementation. Generated scripts should manage their own `BEGIN TRY` / `BEGIN CATCH` / `BEGIN TRANSACTION` block per migration.

Script and execution rules:

- Script generation is the default mode and must be usable for review before execution.
- `--execute` must use the same script body generated for review.
- `--connection-string` is valid only with `--execute`, matching `database update`.
- Connection-string resolution for execution should match `database update`: `--connection-string`, then `DATABASE_URL`, then `SQL_ORM_TEST_CONNECTION_STRING`.
- The generated script should be idempotent with respect to migrations that are no longer applied, but it must still fail checksum mismatches for applied migrations before rollback.

Ambiguity rules:

- Downgrade across a migration that was applied in the database but is missing locally must fail.
- Downgrade across a migration whose local `down.sql` is not executable must fail.
- Downgrade from a database history that is not a prefix of the local migration list must fail. This catches edited, deleted, reordered, or branch-diverged migration histories.
- The CLI should prefer explicit error messages over partial progress whenever it can detect ambiguity before execution.

## Recreating the Todo App Migration Flow

```bash
examples/todo-app/scripts/migration_e2e.sh
```

The script generates an initial migration from the current example model, a no-op incremental migration, and a `database_update.sql` script in a temporary directory.

## Limits

- `database downgrade --execute` is available and has optional real SQL Server coverage through `sql-orm-cli` tests when `SQL_ORM_TEST_CONNECTION_STRING` is configured.
- `migration.rs` is outside the current MVP.
- Composite foreign-key derivation from public attributes is not part of the current derive surface.
- Review SQL carefully for computed columns, foreign keys, indexes, and explicit renames.
