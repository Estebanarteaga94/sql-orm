# Typed Aggregations Design

Status: Etapa 24 design. The first neutral AST cut exists in
`sql-orm-query`; SQL Server compilation and public `DbSetQuery` APIs are not
yet implemented.

This document defines the public aggregation surface that Etapa 24 should
implement on top of `DbSetQuery`. It is intentionally a design contract for the
next implementation tasks, not a user guide for shipped behavior.

## Goals

- Keep the public API on the root `sql-orm` crate.
- Keep aggregate intent in `sql-orm-query` as neutral AST.
- Keep SQL Server syntax generation in `sql-orm-sqlserver`.
- Keep execution in `sql-orm-tiberius` through existing compiled-query paths.
- Preserve mandatory root-entity `tenant` and `soft_delete` filters before
  aggregate compilation.
- Support scalar aggregates first, then grouped aggregates materialized through
  `FromRow` DTOs.

## Non Goals

- Do not add multi-database aggregation abstractions.
- Do not generate SQL directly from `sql-orm` or `sql-orm-query`.
- Do not attach navigation wrappers or materialize entity graphs from grouped
  aggregate queries.
- Do not infer hidden joins from navigation properties for aggregations. Joins
  remain explicit or use the existing fallible navigation join helpers.
- Do not implement window functions, rollups, cubes, distinct aggregates, or
  provider-specific advanced aggregate syntax in the first cut.

## Scalar API

`DbSetQuery<E>` keeps `count()` and adds scalar aggregate methods:

```rust
let total_users = db.users.query().count().await?;
let has_users = db.users.query().exists().await?;
let has_users_alias = db.users.query().any().await?;

let total_cents = db
    .orders
    .query()
    .filter(Order::status.eq("paid"))
    .sum::<i64>(Order::total_cents)
    .await?;

let average_cents = db
    .orders
    .query()
    .avg::<f64>(Order::total_cents)
    .await?;

let first_created_at = db
    .orders
    .query()
    .min::<chrono::NaiveDateTime>(Order::created_at)
    .await?;

let last_created_at = db
    .orders
    .query()
    .max::<chrono::NaiveDateTime>(Order::created_at)
    .await?;
```

Planned signatures:

```rust
impl<E: Entity> DbSetQuery<E> {
    pub async fn count(self) -> Result<i64, OrmError>;
    pub async fn exists(self) -> Result<bool, OrmError>;
    pub async fn any(self) -> Result<bool, OrmError>;

    pub async fn sum<T, C>(self, column: C) -> Result<Option<T>, OrmError>
    where
        T: SqlTypeMapping + Send,
        C: Into<Expr>;

    pub async fn avg<T, C>(self, column: C) -> Result<Option<T>, OrmError>
    where
        T: SqlTypeMapping + Send,
        C: Into<Expr>;

    pub async fn min<T, C>(self, column: C) -> Result<Option<T>, OrmError>
    where
        T: SqlTypeMapping + Send,
        C: Into<Expr>;

    pub async fn max<T, C>(self, column: C) -> Result<Option<T>, OrmError>
    where
        T: SqlTypeMapping + Send,
        C: Into<Expr>;
}
```

Return rules:

- `count()` returns `i64` and never `Option<i64>`.
- `exists()` and `any()` return `bool`; `any()` is an alias for `exists()`.
- `sum`, `avg`, `min` and `max` return `Option<T>` because SQL Server can
  return `NULL` for empty filtered sets and for nullable expressions.
- The caller chooses `T` explicitly because current `EntityColumn<E>` values do
  not carry the Rust field type at the type level.

`count()` should be reimplemented on top of the aggregate query path so it
preserves joins and effective filters. It should ignore ordering and
pagination, because it answers "how many rows match this query shape before
page slicing". If a caller needs a count over a limited subquery, that remains
outside the first cut.

## Grouped API

Grouped aggregates use a separate builder returned from `group_by(...)`.
The builder must not expose `all()` or `first()` for full entities.

```rust
#[derive(Debug, FromRow)]
struct OrderTotals {
    customer_id: i64,
    order_count: i64,
    total_cents: Option<i64>,
}

let rows = db
    .orders
    .query()
    .filter(Order::status.eq("paid"))
    .group_by(Order::customer_id)
    .select_aggregate((
        AggregateProjection::group_key(Order::customer_id),
        AggregateProjection::count_as("order_count"),
        AggregateProjection::sum_as(Order::total_cents, "total_cents"),
    ))
    .all_as::<OrderTotals>()
    .await?;
```

Planned public types:

```rust
pub struct DbSetGroupedQuery<E: Entity> { /* opaque */ }

pub trait GroupByExpressions {
    fn into_group_by_expressions(self) -> Vec<Expr>;
}

pub trait AggregateProjections {
    fn into_aggregate_projections(self) -> Vec<AggregateProjection>;
}

pub struct AggregateProjection {
    pub expr: AggregateExpr,
    pub alias: &'static str,
}

pub enum AggregateExpr {
    GroupKey(Expr),
    CountAll,
    Count(Expr),
    Sum(Expr),
    Avg(Expr),
    Min(Expr),
    Max(Expr),
}
```

Planned builder shape:

```rust
impl<E: Entity> DbSetQuery<E> {
    pub fn group_by<G>(self, group_by: G) -> Result<DbSetGroupedQuery<E>, OrmError>
    where
        G: GroupByExpressions;
}

impl<E: Entity> DbSetGroupedQuery<E> {
    pub fn select_aggregate<P>(self, projection: P) -> Self
    where
        P: AggregateProjections;

    pub fn having(self, predicate: AggregatePredicate) -> Self;
    pub fn order_by(self, order: AggregateOrderBy) -> Self;
    pub fn limit(self, limit: u64) -> Self;
    pub fn take(self, limit: u64) -> Self;
    pub fn paginate(self, request: PageRequest) -> Self;

    pub async fn all_as<T>(self) -> Result<Vec<T>, OrmError>
    where
        T: FromRow + Send;

    pub async fn first_as<T>(self) -> Result<Option<T>, OrmError>
    where
        T: FromRow + Send;
}
```

`GroupByExpressions` should mirror the existing projection ergonomics:
single expression, tuple, array, and `Vec`. Empty groups are rejected early.

`AggregateProjections` should mirror `SelectProjections`: single projection,
tuple, array, and `Vec`.

## Projection Rules

- Every aggregate expression must have an explicit non-empty alias.
- Group key projections may use their column name as the default alias when the
  expression is an entity column.
- Non-column group key expressions require an explicit alias.
- Duplicate aliases are rejected before execution.
- Grouped queries require at least one group key and at least one projection.
- Projected non-aggregate expressions must appear in `group_by(...)`.
- Aggregate projection aliases are the contract consumed by `FromRow` DTOs.

The first cut should prefer runtime validation with clear `OrmError` messages
inside the builder/compiler. Compile-time rejection can be added only where the
type system already has enough information without creating a proc-macro DSL.

## Having

`HAVING` support should be deliberately small:

```rust
let rows = db
    .orders
    .query()
    .group_by(Order::customer_id)
    .select_aggregate((
        AggregateProjection::group_key(Order::customer_id),
        AggregateProjection::count_as("order_count"),
    ))
    .having(AggregateExpr::count_all().gt(1_i64))
    .all_as::<OrderTotals>()
    .await?;
```

The predicate shape can mirror existing `Predicate`, but it must operate over
aggregate expressions and group keys. It should compile only to `HAVING`, not
to `WHERE`.

## Policies, Joins, and Includes

- Root `tenant` and `soft_delete` filters are applied before aggregation.
- Joins configured before `count`, `exists`, scalar aggregates or `group_by`
  are preserved in the aggregate query AST.
- Manual joins do not receive automatic `tenant` or `soft_delete` filters for
  joined entities in this first cut, matching the current query-builder limit.
- `include(...)` and `include_many(...)` do not expose aggregation methods.
  Entity graph loading and aggregate DTO materialization remain separate.
- Projection `select(...)` and grouped aggregate `select_aggregate(...)` are
  separate routes.

## SQL Server Compilation Expectations

The SQL Server compiler should own all SQL rendering:

- `COUNT(*) AS [count]`
- `CASE WHEN EXISTS (...) THEN CAST(1 AS bit) ELSE CAST(0 AS bit) END AS [exists]`
- `SUM(...) AS [alias]`
- `AVG(...) AS [alias]`
- `MIN(...) AS [alias]`
- `MAX(...) AS [alias]`
- `GROUP BY ...`
- basic `HAVING ...`

Parameter order must follow the same rule as existing queries: predicates and
expressions are compiled deterministically into `@P1`, `@P2`, ...

## Validation Plan

The implementation tasks that follow this design should add:

- SQL compiler snapshot tests for `COUNT`, `EXISTS`, `SUM`, `AVG`, `MIN`,
  `MAX`, `GROUP BY`, `HAVING`, joins and parameter ordering.
- Public `trybuild` fixtures for valid and invalid aggregation APIs from
  `sql_orm::prelude`.
- Optional SQL Server runtime tests for scalar aggregates and grouped DTOs.
- Documentation updates to `docs/query-builder.md`, `docs/projections.md`,
  `docs/api.md`, `README.md` and `CHANGELOG.md` only after executable behavior
  lands.
