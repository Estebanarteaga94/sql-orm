# Typed Raw SQL

Typed raw SQL is an explicit escape hatch for queries and commands that do not fit the public query builder.

This API does not change the architecture:

- `sql-orm-query` remains an AST crate and does not parse raw SQL.
- `sql-orm-sqlserver` remains the normal compiler for ORM queries.
- raw SQL execution still goes through the public root crate and the Tiberius adapter.

See also [Core concepts](core-concepts.md).

## When to Use It

Prefer `DbSetQuery` when you can express the operation with filters, ordering, joins, pagination, projections, and `count`.

Use raw SQL when you need:

- SQL Server-specific syntax not modeled in the AST;
- hand-written reporting queries;
- administrative commands;
- temporary compatibility while the query builder grows.

## Public API

```rust
let rows = db
    .raw::<UserDto>("SELECT id, email FROM dbo.users WHERE email LIKE @P1")
    .param("%@example.com")
    .all()
    .await?;
```

```rust
let result = db
    .raw_exec("UPDATE dbo.users SET active = @P1 WHERE id = @P2")
    .params((false, 7_i64))
    .execute()
    .await?;
```

`RawQuery<T>` materializes rows with `FromRow`. `RawCommand` returns `ExecuteResult`.

Raw SQL has an explicit execution classification for retry safety:

- `RawQuery<T>` defaults to `RawNoRetry`;
- `RawQuery<T>::read_only()` marks a raw query as read-only and eligible for the configured read retry policy;
- `RawQuery<T>::no_retry()` forces the default no-retry behavior;
- `RawCommand` defaults to `Write`;
- `RawCommand::migration()` marks command SQL as migration or schema-management SQL;
- `RawCommand::no_retry()` forces command SQL to bypass retry.

The ORM does not inspect the SQL text to decide retry safety. A string that starts with `SELECT` is still not retried unless the raw query is explicitly marked with `read_only()`.

Read-only raw query with retry opt-in:

```rust
let rows = db
    .raw::<UserDto>("SELECT id, email FROM dbo.users WHERE active = @P1")
    .param(true)
    .read_only()
    .all()
    .await?;
```

## Query Hints

`RawQuery<T>` supports SQL Server query hints through `query_hint(...)`.

The first supported hint is `QueryHint::Recompile`, which appends `OPTION (RECOMPILE)` to the raw SQL before execution:

```rust
let rows = db
    .raw::<UserDto>("SELECT id, email FROM dbo.users WHERE id = @P1")
    .param(7_i64)
    .query_hint(QueryHint::Recompile)
    .all()
    .await?;
```

Use this when a parametrized raw query gets a poor cached or generic SQL Server plan and recompiling per execution is an acceptable tradeoff. Query hints do not change retry classification: add `read_only()` separately when the SQL is safe to retry. The parameter rules remain unchanged: values are still bound through `@P1..@Pn`.

Do not write `OPTION (...)` manually in the SQL when using `query_hint(...)`. The ORM rejects that combination before execution to avoid duplicating or mixing API-managed hints with hand-written hints.

## DTOs

```rust
struct UserDto {
    id: i64,
    email: String,
}

impl FromRow for UserDto {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            id: row.get_required_typed("id")?,
            email: row.get_required_typed("email")?,
        })
    }
}
```

Column names used in `get_required_typed` must match the columns or aliases returned by the `SELECT`.

## Parameters

Raw SQL uses SQL Server placeholders `@P1`, `@P2`, ..., `@Pn`.

Rules:

- placeholders must be continuous from `@P1` to `@Pn`;
- the number of provided parameters must match the highest placeholder index;
- repeated placeholders reuse the same value;
- placeholders inside string literals, bracketed identifiers, double-quoted identifiers, `--` line comments, and `/* ... */` block comments are ignored by validation;
- values are bound as parameters, not interpolated strings.

Valid repeated placeholder:

```rust
db.raw::<UserDto>(
    "SELECT id, email FROM dbo.users WHERE id = @P1 OR manager_id = @P1",
)
.param(7_i64)
.all()
.await?;
```

Ignored placeholder-like text:

```rust
db.raw::<UserDto>(
    "SELECT '@P1 is text' AS note, id, email FROM dbo.users -- @P2 ignored
     WHERE id = @P1",
)
.param(7_i64)
.all()
.await?;
```

This query expects one parameter because only the `@P1` in `WHERE id = @P1` is in executable SQL.

## Retry Contract

Configured retry applies only to materialized read paths whose compiled execution classification is `ReadOnly`.

For normal ORM queries, `sql-orm-sqlserver` assigns that classification when it compiles read-only ASTs such as `SELECT`, `COUNT`, `EXISTS`, and aggregations. For raw SQL, the caller must classify the query explicitly with `RawQuery<T>::read_only()`.

Raw SQL retry rules:

- `RawQuery<T>` is `RawNoRetry` by default, even if the SQL starts with `SELECT`;
- `RawQuery<T>::read_only()` opts into retry for idempotent read-only SQL;
- `RawCommand` is `Write` by default and is not retried automatically;
- `RawCommand::migration()` documents schema-management intent and is not retried automatically;
- no raw SQL is retried inside `db.transaction(...)`, because transaction scopes disable retry.

Only mark raw SQL as read-only when rerunning the exact statement is safe. Do not use `read_only()` for statements with side effects, non-idempotent stored procedures, mutation hidden behind `SELECT`, temporary table setup, lock acquisition with side effects, or any operation that depends on running exactly once.

## Security

Do not interpolate user input into SQL strings:

```rust
// Do not do this.
let sql = format!("SELECT id FROM dbo.users WHERE email = '{email}'");
```

Use parameters instead:

```rust
db.raw::<UserDto>("SELECT id, email FROM dbo.users WHERE email = @P1")
    .param(email)
    .all()
    .await?;
```

Raw SQL does not automatically quote identifiers. If dynamic schema, table, or column names are required, use an application-level allowlist before building the SQL string.

## Tenant and Soft Delete

Raw SQL does not automatically apply ORM filters.

If an entity uses `#[orm(tenant = CurrentTenant)]`, public `DbSetQuery` and CRUD routes apply mandatory tenant filters. Raw SQL bypasses that path; you must write the tenant predicate yourself.

The same applies to `soft_delete`: raw SQL does not add `deleted_at IS NULL`, `is_deleted = 0`, or any equivalent predicate.

## Navigation Boundaries

Raw SQL is not integrated with navigation loading. It does not inspect
`EntityMetadata.navigations`, infer joins, apply include aliases, or attach
`Navigation<T>` / `Collection<T>` wrappers. If a raw query returns related data,
map it into a DTO with `FromRow` and explicit column aliases.

## Transactions

Raw SQL can run inside `db.transaction(...)` when using the transaction context passed to the closure. The same transaction limits documented in [transactions.md](transactions.md) apply.

## Limits

- No identifier builder or automatic identifier quoting.
- No safe format-string interpolation.
- No semantic validation of columns, tables, aliases, or DTOs before execution.
- No special support for multiple result sets.
- No public streaming API; `all()` materializes `Vec<T>`.
- No automatic application of `tenant`, `soft_delete`, or other policies.
- No automatic integration with migrations, `DbSetQuery`, or query-builder projections.
- Query hints are currently available only on `RawQuery<T>`, not on `RawCommand` or the public query builder.
- Retry for raw queries is opt-in via `read_only()`; it is never inferred from the SQL text.

## Validation

Coverage includes:

- raw parameter unit tests;
- repeated `@P1` behavior;
- continuous placeholder validation;
- ignored placeholder-like text in strings, delimited identifiers, and comments;
- explicit raw SQL execution classification for retry;
- public real SQL Server tests behind `SQL_ORM_TEST_CONNECTION_STRING`.
