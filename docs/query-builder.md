# Query Builder

The public query builder does not build SQL directly from the root crate. It produces a `sql-orm-query` AST. SQL Server parameterized SQL is compiled by `sql-orm-sqlserver`, and execution happens in the Tiberius adapter.

See also [Core concepts](core-concepts.md) and
[Navigation properties](navigation.md).

## Entry Point

The normal entry point is `DbSet<T>::query()` from a derived `DbContext`.

```rust
let users = db
    .users
    .query()
    .filter(User::email.contains("@company.com"))
    .order_by(User::email.asc())
    .take(20)
    .all()
    .await?;
```

## Column Symbols

`#[derive(Entity)]` generates column symbols such as `User::email` and `User::active`. These are typed references to model columns, not reads of Rust field values.

## Predicates

Public column predicates include:

- `eq`, `ne`
- `gt`, `gte`, `lt`, `lte`
- `is_null`, `is_not_null`
- `contains`, `starts_with`, `ends_with` for strings

Predicates can be composed with `and`, `or`, and `not`.

```rust
let predicate = User::email
    .contains("@company.com")
    .and(User::active.eq(true));
```

Values compile to SQL Server parameters (`@P1`, `@P2`, ...), not string interpolation.

## Ordering

Use `asc()` or `desc()` on generated columns:

```rust
db.users
    .query()
    .order_by(User::created_at.desc())
    .all()
    .await?;
```

Ordering is preserved in the AST and then in compiled SQL.

## Pagination and Limits

Use `take(...)` or `limit(...)` for simple row limits:

```rust
db.users.query().take(10).all().await?;
```

Use `PageRequest::new(page, page_size)` for explicit pages. Pages are 1-based:

```rust
db.users
    .query()
    .order_by(User::id.asc())
    .paginate(PageRequest::new(1, 25))
    .all()
    .await?;
```

SQL Server pagination requires deterministic ordering.

## Joins And Navigation Loading

For complete navigation-property documentation, including model syntax,
include behavior, explicit loading and current limits, see
[Navigation properties](navigation.md).

Joins are explicit. You can either provide the `ON` predicate manually or ask
`DbSetQuery` to build it from navigation metadata.

```rust
let rows = db
    .orders
    .query()
    .inner_join::<Customer>(Order::customer_id.eq(Customer::id))
    .filter(Customer::email.contains("@company.com"))
    .all()
    .await?;
```

The public API exposes `inner_join::<T>(...)` and `left_join::<T>(...)` for
manual joins.

When a relationship is declared as a navigation property, use the fallible
helpers to infer the join predicate from `EntityMetadata.navigations`:

```rust
let rows = db
    .customers
    .query()
    .try_inner_join_navigation_as::<Order>("orders", "orders")?
    .filter(Order::total_cents.aliased("orders").gte(1000_i64))
    .all()
    .await?;
```

The `_as` variants bind the joined table to a SQL alias and can be used with
aliased public columns such as `Order::total_cents.aliased("orders")`.
Navigation joins only build SQL joins; they do not materialize related entity
graphs.

For single navigations, `include::<T>(...)` performs eager loading through a
left join and attaches the related row to `Navigation<T>`:

```rust
let orders = db
    .orders
    .query()
    .include::<Customer>("customer")?
    .all()
    .await?;

let customer = orders[0].customer.as_ref();
```

After `include`, the returned query still supports filters, explicit joins,
ordering, limits and pagination before `all()` / `first()`:

```rust
let orders = db
    .orders
    .query()
    .include_as::<Customer>("customer", "customer")?
    .filter(Customer::email.aliased("customer").contains("@company.com"))
    .order_by(Customer::email.aliased("customer").asc())
    .take(20)
    .all()
    .await?;
```

`include` remains part of entity materialization. The include builders expose
entity-loading methods such as `all()` / `first()` and chaining for filters,
ordering and joins, but they intentionally do not expose `select(...)`,
`all_as::<T>()` or `first_as::<T>()`. DTO projections stay on plain
`DbSetQuery` before any include is configured.

Runtime policies are applied to both sides safely. Root `tenant` and
`soft_delete` filters are added to the effective query predicate; included
entity `tenant` and default `soft_delete` filters are added to the include
`JOIN ... ON` predicate so a missing or filtered related row does not discard
the root row. Tenant-scoped included entities fail closed when the active
tenant is missing, has a different column, or has an incompatible value.

The current include cut supports `belongs_to` and `has_one`. `has_many` remains
available through `include_many::<T>(...)` / `include_many_as::<T>(...)`:

```rust
let customers = db
    .customers
    .query()
    .include_many_as::<Order>("orders", "orders")?
    .max_joined_rows(2_000)
    .filter(Order::total_cents.aliased("orders").gte(1000_i64))
    .all()
    .await?;

let orders = customers[0].orders.as_slice();
```

The collection include path uses a left join in this first cut, then groups
joined rows by the root entity primary key before assigning `Collection<T>`.
It rejects pagination because `OFFSET` / `FETCH` over joined rows would not
produce a stable page of root entities.

The default join strategy has a safety limit of 10,000 joined rows before
grouping. Call `max_joined_rows(...)` when the expected cardinality is known,
or `unbounded_join()` only when the caller intentionally accepts the full joined
result. `split_query()` is already part of the public builder to make the
strategy explicit, but execution returns a clear error until the split-query
loader is implemented.

When the root entity is already materialized, use explicit loading instead of
rewriting the root query:

```rust
let mut customer = db.customers.find(7_i64).await?.expect("customer");
db.customers
    .load_collection::<Order>(&mut customer, "orders")
    .await?;
```

This remains an explicit async call and does not add lazy loading to field
access.

Direct many-to-many navigation is not part of the executable query surface.
Represent many-to-many relationships as a normal join entity with two
`belongs_to` edges and query that entity with explicit joins or supported
includes. The ORM does not infer hidden join tables and does not persist link
adds/removes from direct collection mutations.

Lazy loading remains opt-in only. `LazyNavigation<T>` and `LazyCollection<T>`
can be used as navigation fields when the model wants loaded/unloaded state,
but the wrappers do not own a context and never issue queries from accessors,
`Debug`, `Clone` or comparison. Current loading still happens through explicit
query APIs: `include(...)`, `include_many(...)` and `load_collection(...)`.
The default guidance is still to choose includes, explicit loading, joins or
projections according to the query shape instead of relying on hidden per-row
queries.

## Count

`count()` preserves the base `from` and filters. In the current state it does not carry joins, ordering, or pagination into the internal `CountQuery`; use it for base-entity counts with filters that do not depend on joined tables.

## Projections

The public API supports two separate materialization paths:

- `all()` and `first()` materialize full entities.
- `select(...).all_as::<T>()` and `select(...).first_as::<T>()` materialize DTOs with `FromRow`.

Includes and DTO projections are mutually separate routes. Use `include(...)`
or `include_many(...)` when the result must be an entity graph with
`Navigation<T>` / `Collection<T>` populated. Use `select(...).all_as::<T>()`
when the result must be a flat SQL projection DTO. Raw SQL remains a third
escape hatch and does not infer navigation metadata or attach navigation
wrappers.

```rust
use sql_orm::prelude::*;

#[derive(Debug, FromRow)]
struct UserSummary {
    id: i64,
    #[orm(column = "email_address")]
    email: String,
}

let summaries = db
    .users
    .query()
    .select((
        User::id,
        SelectProjection::expr_as(sql_orm::query::Expr::from(User::email), "email_address"),
    ))
    .all_as::<UserSummary>()
    .await?;
```

Projected columns receive default aliases equal to their column names. Expressions require explicit aliases. Empty or duplicate aliases are rejected before execution.
Projection DTOs can use `#[derive(FromRow)]`; fields read aliases by field name unless overridden with `#[orm(column = "...")]`.

## Runtime Filters

`DbSetQuery<T>` does not publicly expose its internal `SelectQuery`. The effective query can add mandatory runtime filters before compilation or execution, such as soft-delete visibility and tenant security filters.

## Limits

- The public query builder does not accept arbitrary SQL fragments.
- Navigation joins are explicit and fallible.
- `include::<T>(...)` supports `belongs_to` and `has_one`; `include_many::<T>(...)` supports `has_many` without pagination.
- `include_many::<T>(...)` defaults to join loading with a 10,000 joined-row safety limit. Split-query loading is explicit but not implemented yet.
- Direct `many_to_many` navigation is rejected; use an explicit join entity until relationship-update semantics are stable.
- Included `tenant` and `soft_delete` policies use the default visibility only; there is no include-specific visibility override yet.
- Initial public projections exist, but high-level typed aggregations are not available.

## Related

- Navigation properties: [navigation.md](navigation.md)
- Projections: [projections.md](projections.md)
- Raw SQL escape hatch: [raw-sql.md](raw-sql.md)
- Real example queries: [examples/todo-app/src/queries.rs](../examples/todo-app/src/queries.rs)
