# ORM Architecture

## Goal

The project builds a code-first ORM for SQL Server. The architecture is split into layers so metadata, query composition, SQL compilation, and execution do not collapse into one module.

## Expected Flow

1. The user defines entities and a context in Rust.
2. `sql-orm-macros` generates static metadata and auxiliary contracts.
3. `sql-orm-query` builds a typed SQL-free AST.
4. `sql-orm-sqlserver` compiles that AST to parameterized SQL Server SQL.
5. `sql-orm-tiberius` executes the query and adapts rows and errors.
6. `sql-orm-migrate` uses metadata for snapshots and migrations.
7. `sql-orm` exposes the supported public surface.

## Crate Boundaries

### `sql-orm-core`

- Defines stable shared contracts.
- Owns metadata, common types, and errors.
- Does not know Tiberius or execution details.

### `sql-orm-macros`

- Implements derives and `#[orm(...)]` parsing.
- Generates metadata and compile-time auxiliary code.
- Must not assume SQL generation or network access.

### `sql-orm-query`

- Represents the AST and typed query builder.
- Models selection, filtering, ordering, pagination, and composition.
- Does not emit SQL directly.

### `sql-orm-sqlserver`

- Converts the AST into parameterized SQL Server SQL.
- Centralizes identifier quoting, `@P1..@Pn` placeholders, and dialect decisions.
- Does not open connections or execute queries.

### `sql-orm-tiberius`

- Encapsulates connections, execution, rows, and transactions.
- Translates driver errors into ORM errors.
- Does not define metadata or compile ASTs into SQL.

### `sql-orm-migrate`

- Computes model snapshots and diffs.
- Produces migration operations and SQL Server migration SQL.
- Depends on model metadata and the SQL compiler, not on the public query builder.

### `sql-orm-cli`

- Orchestrates migration commands and operational tasks.
- Must rely on internal crates rather than duplicating domain logic.

### `sql-orm`

- Provides the consolidated public surface.
- Reexports supported types, derives, and modules for consumers.

## Current Decisions

- Only SQL Server is supported in this phase.
- Crate separation is structural and must not be collapsed for convenience.
- The MVP prioritizes metadata, derives, basic CRUD, and initial migrations before advanced features.

## Current State

- The architecture is implemented as a workspace with separate crates for the public API, core contracts, macros, query AST, SQL Server compilation, Tiberius execution, migrations, and CLI.
- `docs/repository-audit.md` keeps the verified inventory of real APIs, implemented features, limits, and deferred features.
- Pending verification: any functional-state claim not covered by `docs/repository-audit.md`, versioned tests, or `docs/worklog.md` must be checked against the code before being repeated in public documentation.
