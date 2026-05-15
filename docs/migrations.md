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

## Recreating the Todo App Migration Flow

```bash
examples/todo-app/scripts/migration_e2e.sh
```

The script generates an initial migration from the current example model, a no-op incremental migration, and a `database_update.sql` script in a temporary directory.

## Limits

- `down.sql` is not executed automatically.
- `database downgrade` does not exist.
- `migration.rs` is outside the current MVP.
- Composite foreign-key derivation from public attributes is not part of the current derive surface.
- Review SQL carefully for computed columns, foreign keys, indexes, and explicit renames.
