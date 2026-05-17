# Public API

This document is a compact inventory of the current public surface exposed by the root crate `sql-orm`.

For consumer code, the recommended entry point is:

```rust
use sql_orm::prelude::*;
```

The root crate concentrates the user API and reexports selected internals for tests, tooling, and advanced cases. Responsibilities remain separated by crate: `query` builds ASTs, `sqlserver` compiles SQL, `tiberius` executes, `migrate` manages snapshots/diffs/migrations, and `core` defines shared contracts.

See also [Core concepts](core-concepts.md) and
[Navigation properties](navigation.md).

## Public Derives

The following derives are available from the public crate:

- `#[derive(Entity)]`
- `#[derive(Insertable)]`
- `#[derive(Changeset)]`
- `#[derive(FromRow)]`
- `#[derive(DbContext)]`
- `#[derive(AuditFields)]`
- `#[derive(SoftDeleteFields)]`
- `#[derive(TenantContext)]`

Basic example:

```rust
use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 180)]
    email: String,
}

#[derive(Insertable)]
#[orm(entity = User)]
struct NewUser {
    email: String,
}

#[derive(Changeset)]
#[orm(entity = User)]
struct UpdateUser {
    email: Option<String>,
}

#[derive(FromRow)]
struct UserSummary {
    id: i64,
    #[orm(column = "email_address")]
    email: String,
    display_name: Option<String>,
}

#[derive(DbContext)]
struct AppDb {
    pub users: DbSet<User>,
}
```

`#[derive(FromRow)]` is available for DTOs used by typed projections and typed raw SQL. It supports structs with named fields, default aliases from field names, explicit field aliases with `#[orm(column = "...")]`, and nullable or missing projected columns through `Option<T>`.

## Model Contracts

The prelude exposes the main metadata and mapping contracts:

- `Entity`
- `EntityMetadata`
- `EntityColumn`
- `Navigation<T>`
- `Collection<T>`
- `LazyNavigation<T>`
- `LazyCollection<T>`
- `ColumnMetadata`
- `PrimaryKeyMetadata`
- `IdentityMetadata`
- `IndexMetadata`
- `IndexColumnMetadata`
- `ForeignKeyMetadata`
- `NavigationMetadata`
- `NavigationKind`
- `ReferentialAction`
- `EntityPolicy`
- `EntityPolicyMetadata`
- `SqlServerType`
- `SqlTypeMapping`
- `SqlValue`
- `ColumnValue`
- `Row`
- `FromRow`
- `Insertable`
- `Changeset`
- `OrmError`

Typical use:

```rust
let metadata = User::metadata();
let email_column = User::email;

assert_eq!(metadata.table, "users");
assert_eq!(email_column.column_name(), "email");
```

Navigation fields declared with `Navigation<T>`, `Collection<T>`,
`LazyNavigation<T>` or `LazyCollection<T>` are not persisted columns.
`#[derive(Entity)]` accepts `belongs_to`, `has_one` and `has_many` attributes,
excludes those fields from `ColumnMetadata`, emits `NavigationMetadata`, and
initializes eager wrappers empty and lazy wrappers unloaded when materializing
an entity without an explicit include/load operation.

The full navigation guide is [Navigation properties](navigation.md).

Direct `many_to_many` navigation attributes are rejected. Model many-to-many
relationships as explicit join entities with ordinary foreign keys and
supported `belongs_to` / `has_many` navigations until relationship-update
semantics are stable.

## DbContext and DbSet

The main data-access API is:

- `DbContext`
- `DbSet<T>`
- `DbSetQuery<T>`
- `DbContextEntitySet<T>`
- `SharedConnection`
- `connect_shared(...)`
- `connect_shared_with_options(...)`
- `connect_shared_with_config(...)`

`#[derive(DbContext)]` generates inherent methods on your context:

- `connect(...)`
- `connect_with_options(...)`
- `connect_with_config(...)`
- `from_connection(...)`
- `from_shared_connection(...)`
- `health_check().await`
- `transaction(|tx| async move { ... }).await`
- `save_changes().await`
- `from_pool(...)` when `pool-bb8` is enabled

`DbSet<T>` exposes CRUD and query operations:

- `find(key).await`
- `insert(model).await`
- `update(key, changeset).await`
- `delete(key).await`
- `query()`
- `query_with(select_query)`
- `entity_metadata()`
- `find_tracked(key).await`
- `add_tracked(entity)`
- `remove_tracked(&mut tracked)`

Relevant limits:

- `find`, `update`, `delete`, Active Record, and public tracking remain oriented around simple primary keys.
- `save_changes()` and `Tracked<T>` are stable for explicit tracking with
  simple primary keys.
- Pending `Added`, `Modified` and `Deleted` tracking work is held by the
  context registry after the wrapper is dropped or consumed. Explicit
  `detach_tracked(...)`, `Tracked::detach()` and `clear_tracker()` still remove
  work from the current unit of work.
- Wrapper lifetime is no longer required for those pending operations.
- A detached loaded identity can reattach to registry-owned original/current
  snapshots. A second live `Tracked<T>` handle for the same persisted identity
  in one context is rejected with `OrmError`.
- Registry-owned snapshots are also used when `save_changes()` accepts no-op
  modifications or synchronizes persisted rows, so those paths do not require
  the original wrapper to remain alive.
- Navigation wrapper mutations are not graph update commands. Persist
  relationship changes by updating, deleting or inserting the dependent entity
  or explicit join entity directly.
- With `pool-bb8`, `db.transaction(...)` is supported for contexts created
  from `from_pool(...)`. The runtime pins one physical pooled connection for
  the full closure, reuses it for `save_changes()`, tenant, audit and
  soft-delete paths, disables retry while the transaction is active, and clears
  the pinned slot after commit, rollback or transaction setup errors.

The current inventory of pending verification, deferred, and blocked public
surfaces is tracked in [Stability audit](stability-audit.md). The criteria used
to stabilize tracking are tracked in
[Tracking stability criteria](tracking-stability.md).

## Query Builder

The public query extensions include:

- `EntityColumnPredicateExt`
- `PredicateCompositionExt`
- `EntityColumnOrderExt`
- `PageRequest`
- `SelectProjections`

Common query methods:

- `filter(...)`
- `order_by(...)`
- `limit(...)`
- `take(...)`
- `paginate(...)`
- `inner_join::<T>(...)`
- `left_join::<T>(...)`
- `try_inner_join_navigation::<T>(...)`
- `try_left_join_navigation::<T>(...)`
- `try_inner_join_navigation_as::<T>(...)`
- `try_left_join_navigation_as::<T>(...)`
- `include::<T>(...)`
- `include_as::<T>(...)`
- `include_many::<T>(...)`
- `include_many_as::<T>(...)`
- `select(...)`
- `all().await`
- `first().await`
- `count().await`
- `all_as::<T>().await`
- `first_as::<T>().await`

The query builder produces AST values. SQL generation belongs to `sql-orm-sqlserver`.

`include::<T>(...)` and `include_as::<T>(...)` load one `belongs_to` /
`has_one` navigation. `include_many::<T>(...)` and `include_many_as::<T>(...)`
load one `has_many` navigation through a join grouped by root primary key,
default to a 10,000 joined-row safety limit, and reject pagination in this
first cut. Collection includes expose `max_joined_rows(...)`,
`unbounded_join()`, `join_strategy()`, and explicit `split_query()`; the
split-query execution path currently returns a clear not-implemented error.
Root policies are applied to the effective query predicate, while
included-entity `tenant` and default `soft_delete` policies are applied to the
include join predicate.

Explicit collection loading is available from `DbSet<E>`:

- `load_collection::<T>(&mut entity, "navigation").await`
- `load_collection_tracked::<T>(&mut tracked, "navigation").await`

This first explicit loading cut supports `has_many` navigations where the root
entity has a single-column primary key and the navigation local column is that
primary key. The tracked variant attaches the collection without marking the
tracked entity as modified. When a related entity is already present in the
tracking registry, `load_collection_tracked(...)` attaches the registry-owned
current snapshot for that related identity instead of the freshly materialized
row. Related rows that are not already tracked remain ordinary values and are
not registered automatically.
The ordinary `load_collection(...)` path uses the same replacement rule for
related rows that are already tracked in the same context, while still leaving
newly materialized related rows untracked.

Navigation graph tracking remains intentionally narrow. Includes return
ordinary entity values and do not register included graphs automatically.
`load_collection_tracked(...)` mutates only the tracked root wrapper without
tracking related entities. Relationship changes inside navigation wrappers are
not persisted by `save_changes()` in the stable explicit-tracking cut.

Many-to-many link changes follow the same rule: insert, update or delete the
explicit join entity rows directly. There is no direct collection navigation
whose mutations are translated into link-table updates.

The identity map has an initial tracking-focused cut: if `find_tracked(...)`
loads a persisted identity whose previous wrapper was dropped but whose entry
remains in the context registry, the returned wrapper reattaches to the
registry-owned snapshot instead of creating a duplicate. The first stable-cut
policy permits only one live `Tracked<T>` handle for the same persisted
identity in one context; attempting to load another live handle returns an
`OrmError` and asks the caller to detach or drop the existing handle first.
`load_collection_tracked(...)`
also consults those snapshots for already tracked related rows. `include(...)`,
`include_many(...)` and ordinary `load_collection(...)` now reuse those
registry-owned related snapshots when a related entity with the same simple
primary key is already tracked in the context. This is snapshot replacement,
not graph tracking: newly materialized related rows are not registered
automatically.

Navigation includes are not projection builders. After `include(...)` or
`include_many(...)`, the returned builder does not expose `select(...)`,
`all_as::<T>()` or `first_as::<T>()`; use plain `DbSetQuery` with explicit
joins for DTO projections, or raw SQL for fully manual result shapes.

Projection DTOs can derive `FromRow`:

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

The derive is intentionally limited to named-field DTOs. Tuple structs, unit structs, and field-level `#[orm(...)]` attributes other than `column = "..."` are rejected at compile time.

## Raw SQL

Raw SQL is exposed through:

- `DbContext::raw<T>(sql)`
- `DbContext::raw_exec(sql)`
- `RawQuery<T>`
- `RawCommand`
- `RawParam`
- `RawParams`
- `QueryHint`

Raw SQL uses `@P1..@Pn` parameters and materializes query rows through `FromRow`. It does not automatically apply tenant or soft-delete filters.
`RawQuery<T>::query_hint(QueryHint::Recompile)` can append SQL Server `OPTION (RECOMPILE)` for parametrized raw queries that need per-execution plan compilation.

## Entity Policies

Public policy-related contracts and derives include:

- `EntityPolicy`
- `EntityPolicyMetadata`
- `AuditFields`
- `SoftDeleteFields`
- `TenantContext`
- `AuditEntity`
- `SoftDeleteEntity`
- `TenantScopedEntity`
- `AuditProvider`
- `AuditContext`
- `AuditOperation`
- `AuditRequestValues`
- `AuditValues`
- `SoftDeleteProvider`
- `SoftDeleteContext`
- `SoftDeleteOperation`
- `SoftDeleteRequestValues`
- `SoftDeleteValues`
- `ActiveTenant`

Implemented behavior:

- `audit = Audit` contributes metadata/schema columns only.
- `#[derive(Entity)]` implements `AuditEntity`; `audit_policy()` returns the audit-owned columns for audited entities and `None` for entities without `audit`.
- The runtime `AuditProvider` contract exists in the public crate, including operation, request values, context, and precedence rules for resolving `ColumnValue`s.
- `#[derive(AuditFields)]` implements `AuditValues`, so the same audit policy struct can be passed as typed request values with `with_audit_values(Audit { ... })`.
- `SharedConnection` and derived `DbContext`s transport `AuditProvider`, `AuditRequestValues`, and typed `AuditValues`; transaction contexts inherit the same shared runtime.
- Insert/update paths consume that runtime: `DbSet::insert`, `DbSet::update`, Active Record `save`, and `save_changes()` for `Added`/`Modified` complete missing audit columns declared by `AuditEntity::audit_policy()`.
- `soft_delete = SoftDelete` changes delete behavior and read visibility for the root entity.
- `#[derive(SoftDeleteFields)]` implements `SoftDeleteValues`, so the same soft-delete policy struct can be passed as typed request values with `with_soft_delete_values(SoftDelete { ... })`.
- `SharedConnection` and derived `DbContext`s transport `SoftDeleteProvider`, `SoftDeleteRequestValues`, and typed `SoftDeleteValues`; typed values are converted into the existing request-values path.
- `tenant = CurrentTenant` adds fail-closed tenant filtering and tenant insert validation/fill for opt-in entities.

Deferred behavior:

- automatic policy filters over manually joined entities;
- global tenant conventions without a user-defined tenant type.

## Migrations

Migration-related public helpers include:

- `MigrationModelSource`
- `model_snapshot_from_source::<C>()`
- `model_snapshot_json_from_source::<C>()`

Advanced migration types are reexported through the `migrate` module for tooling.

## Operational Types

The public crate reexports Tiberius adapter configuration types such as:

- `MssqlConnectionConfig`
- `MssqlOperationalOptions`
- `MssqlTimeoutOptions`
- `MssqlRetryOptions`
- `MssqlTracingOptions`
- `MssqlSlowQueryOptions`
- `MssqlHealthCheckOptions`
- `MssqlHealthCheckQuery`
- `MssqlParameterLogMode`
- `MssqlPoolOptions`
- `MssqlPoolBackend`
- `MssqlPool`, `MssqlPoolBuilder`, and `MssqlPooledConnection` when `pool-bb8` is enabled

## Advanced Reexports

The root crate also reexports selected internal crates:

- `sql_orm::core`
- `sql_orm::query`
- `sql_orm::sqlserver`
- `sql_orm::tiberius`
- `sql_orm::migrate`
- `sql_orm::macros`

These are useful for tests, tooling, snapshots, and advanced diagnostics. Normal application code should prefer the prelude.

## Current Exclusions

- SQL Server is the only backend.
- Navigation properties currently expose metadata, explicit join inference, single-navigation eager loading for `belongs_to` / `has_one`, join-based `has_many` eager loading, and explicit `has_many` collection loading.
- Lazy wrappers are implemented as opt-in state containers, but they never query by themselves. There is no automatic single-navigation lazy loader yet.
- High-level typed aggregations are not available.
- Composite primary-key persistence is not complete across public CRUD and Active Record.
- `migration.rs` is not generated.
