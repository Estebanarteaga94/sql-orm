# Typed Projections

Typed projections are the Stage 18 public query-builder feature. They let a query select specific columns or expressions from SQL Server and materialize them into DTOs that implement `FromRow`, without breaking the existing full-entity materialization path through `all()` and `first()`.

See also [Core concepts](core-concepts.md).

## Public Surface

The projection surface is available on `DbSetQuery<E>`:

- `select(...)`
- `all_as::<T>()`
- `first_as::<T>()`

`all()` and `first()` still mean “materialize the full entity `E`”. Partial selections must use the `_as` methods.

## Basic Example

```rust
use sql_orm::prelude::*;

#[derive(Debug, PartialEq, FromRow)]
struct UserSummary {
    id: i64,
    email: String,
}

let users = db
    .users
    .query()
    .select((User::id, User::email))
    .all_as::<UserSummary>()
    .await?;
```

`#[derive(FromRow)]` is the recommended DTO mapping path for projections. The derive reads each DTO field from a row column with the same name as the field, unless the field has an explicit alias with `#[orm(column = "...")]`.

## Aliases and Nullable Fields

Projection aliases are the contract between SQL Server results and DTO fields.

```rust
use sql_orm::prelude::*;

#[derive(Debug, PartialEq, FromRow)]
struct UserCard {
    id: i64,
    #[orm(column = "email_address")]
    email: String,
    display_name: Option<String>,
}

let cards = db
    .users
    .query()
    .select((
        User::id,
        SelectProjection::expr_as(sql_orm::query::Expr::from(User::email), "email_address"),
        SelectProjection::expr_as(sql_orm::query::Expr::from(User::email), "display_name"),
    ))
    .all_as::<UserCard>()
    .await?;
```

Rules:

- field names are used as aliases by default;
- `#[orm(column = "...")]` lets a DTO field read from a different projection alias;
- `Option<T>` fields materialize `NULL` as `None`;
- `Option<T>` fields also materialize a missing column as `None`, which is useful for DTOs shared across compatible projections;
- non-optional fields require the alias to exist and contain a compatible non-null value.

## AST Shape

The AST stores projections as `SelectProjection { expr, alias }`.

Rules:

- an empty projection keeps full-entity semantics and compiles as `SELECT *`;
- generated `EntityColumn<E>` projections receive a default alias equal to `column_name`;
- expression projections require an explicit alias;
- aliases must be stable, non-empty, and unique.

## SQL Compilation

`sql-orm-sqlserver` renders projected values with explicit aliases:

```sql
SELECT [dbo].[users].[id] AS [id], [dbo].[users].[email] AS [email]
FROM [dbo].[users]
```

The alias is part of the contract with `FromRow`: the DTO reads `"id"` and `"email"` rather than relying on driver-specific expression names.

## Expressions

Expressions need explicit aliases:

```rust
use sql_orm::prelude::*;
use sql_orm::query::SelectProjection;

#[derive(Debug, PartialEq, FromRow)]
struct UserEmailProjection {
    id: i64,
    #[orm(column = "email_lower")]
    email: String,
}

let rows = db
    .users
    .query()
    .select((
        User::id,
        SelectProjection::expr_as(User::email.lower(), "email_lower"),
    ))
    .all_as::<UserEmailProjection>()
    .await?;
```

## Joins and Aliases

Projections can select columns from explicitly joined tables. Use
`column.aliased("alias")` together with aliased joins when a query references
the same table more than once, uses a self-join, or needs stable DTO aliases.

Current limits:

- aliases are explicit; the query builder does not assign table aliases automatically;
- if two projected columns share the same `column_name`, assign an explicit
  projection alias to one of them with `SelectProjection::expr_as(...)`.

This avoids ambiguous DTOs for common names such as `id`, `created_at`, or `name`.

## SQL Projections vs. In-Memory `map`

This is an in-memory transformation:

```rust
let summaries = db
    .users
    .query()
    .all()
    .await?
    .into_iter()
    .map(|user| UserSummary {
        id: user.id,
        email: user.email,
    })
    .collect::<Vec<_>>();
```

It is valid when the business flow needs full entities, but it is not a SQL projection. SQL Server still returns all selected entity columns.

Use SQL projections when you want to reduce row width, avoid materializing unused fields, or map directly into read DTOs.

## Runtime Filters

Projections reuse the effective `DbSetQuery` path. Mandatory tenant filters and soft-delete visibility for the root entity still apply before SQL compilation and execution.

Raw SQL remains different: `raw<T>()` does not apply ORM runtime filters automatically.

## Navigation Boundaries

Projection DTOs and navigation loading are intentionally separate. A query that
uses `include(...)` or `include_many(...)` materializes root entities and
attaches `Navigation<T>` or `Collection<T>` values. It does not expose
`select(...)`, `all_as::<T>()` or `first_as::<T>()`.

Use `select(...).all_as::<T>()` for flat SQL DTOs. Use `include(...)` /
`include_many(...)` for entity graphs. If a DTO needs data from related tables,
build an explicit join and projection with aliases instead of expecting
navigation wrappers to be populated.

## FromRow Derive Limits

`#[derive(FromRow)]` for projection DTOs is intentionally small:

- only structs with named fields are supported;
- tuple structs and unit structs are rejected at compile time;
- the only supported field attribute is `#[orm(column = "...")]`;
- it does not infer SQL expressions, joins, or aggregate aliases;
- it does not generate query projections from the DTO shape.

Manual `impl FromRow` is still available for DTOs that need custom decoding logic.

## Not in This Cut

- High-level typed aggregation DSL.
- Automatic table aliases.
- Self-join support.
- Navigation-property projection.
- Automatic query projection generation from DTO definitions.

## Validation

Coverage lives in:

- `crates/sql-orm/tests/stage18_public_projections.rs`
- `crates/sql-orm/tests/stage18_from_row_derive.rs`
- `crates/sql-orm/tests/ui/query_projection_public_valid.rs`
- `crates/sql-orm/tests/ui/from_row_projection_public_valid.rs`
- `crates/sql-orm/tests/ui/from_row_tuple_struct.rs`
- `crates/sql-orm/tests/ui/from_row_unit_struct.rs`
- `crates/sql-orm/tests/ui/from_row_unsupported_attr.rs`
- SQL compiler snapshot tests in `crates/sql-orm-sqlserver`
