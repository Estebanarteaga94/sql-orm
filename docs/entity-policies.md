# Entity Policies

This document describes the public concept, architectural boundaries, implemented behavior, and deferred work for `Entity Policies` in `sql-orm`.

The initial MVP implemented audit as metadata/schema through `#[derive(AuditFields)]` and `#[orm(audit = Audit)]`. Later cuts added runtime behavior for `soft_delete`, mandatory tenant filters for `tenant`, and the public runtime contract for `AuditProvider`. Audit metadata ownership, context transport, and insert/update auto-fill are implemented on the main persistence paths.

See also [Core concepts](core-concepts.md).

## Goal

An `Entity Policy` is a reusable code-first model component that an entity can declare to add cross-cutting columns and, when explicitly designed, related runtime behavior.

The feature avoids repeating the same structural fields in many entities, for example audit columns, soft-delete columns, or tenant columns. A policy does not replace the entity model; it extends it declaratively.

```rust
use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    #[orm(default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    created_at: String,

    #[orm(nullable)]
    #[orm(sql_type = "datetime2")]
    updated_at: Option<String>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todos", schema = "todo", audit = Audit)]
struct Todo {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 200)]
    title: String,
}
```

## Core Rule

Columns contributed by a policy must become normal `ColumnMetadata` entries inside `EntityMetadata.columns`.

This prevents a second schema pipeline. The rest of the system continues to use the same pieces:

- `ModelSnapshot::from_entities(...)` reads columns from `EntityMetadata`.
- The diff engine compares `ColumnSnapshot` values without caring whether a column came from an entity field or from a policy.
- `sql-orm-sqlserver` compiles DDL from normal snapshots and operations.
- `DbContext` and the migration CLI consume normal entity metadata.

Policy-specific metadata can exist for validation and ergonomics, but it must not become a parallel path for snapshots, diffs, or DDL.

## Metadata Contract

The neutral contract lives in `sql-orm-core` and does not know Tiberius, executable SQL, or migrations:

```rust
pub struct EntityPolicyMetadata {
    pub name: &'static str,
    pub columns: &'static [ColumnMetadata],
}

pub trait EntityPolicy: Sized + Send + Sync + 'static {
    const POLICY_NAME: &'static str;
    const COLUMN_NAMES: &'static [&'static str] = &[];

    fn columns() -> &'static [ColumnMetadata];

    fn metadata() -> EntityPolicyMetadata {
        EntityPolicyMetadata::new(Self::POLICY_NAME, Self::columns())
    }
}
```

The contract exposes a stable name, a static column-name slice for compile-time validation, and a static `ColumnMetadata` slice. Expansion into an entity remains the responsibility of `sql-orm-macros`.

`EntityMetadata` does not currently keep a separate list of policies. The data that must flow through snapshots, diffs, and DDL is the resulting column.

## Current Policy Matrix

| Policy | Status | Scope |
| --- | --- | --- |
| `audit = Audit` | Implemented | Metadata/schema columns, typed `AuditValues`, and insert/update auto-fill through typed request values or `AuditProvider`. |
| `soft_delete = SoftDelete` | Implemented | Runtime logical delete, default read visibility, and schema columns through the normal column pipeline. |
| `tenant = CurrentTenant` | Implemented | Opt-in tenant scope, active tenant runtime state, fail-closed filters on the root entity, and insert fill/validation. |
| `AuditProvider` | Implemented | Public runtime contract, audit-owned entity metadata, context transport, typed value helper, and insert/update auto-fill on the main persistence paths exist. |
| `timestamps` | Deferred | Not implemented as a separate policy. |

## Audit Fields

`#[derive(AuditFields)]` implements `EntityPolicy` for a user-defined struct. Its fields become reusable audit columns.

Audit fields do not have to use the predefined semantic markers. The markers
`created_at`, `created_by`, `updated_at`, and `updated_by` are optional metadata
for common audit roles, not column-name inference rules. Applications can add
custom audit columns by declaring normal fields and, when needed, mapping them
to physical names with `#[orm(column = "...")]`.

Supported field attributes include:

- `column`
- `created_at`
- `created_by`
- `updated_at`
- `updated_by`
- `length`
- `nullable`
- `default_sql`
- `sql_type`
- `precision`
- `scale`
- `renamed_from`
- `insertable`
- `updatable`

Unsupported audit-field attributes include:

- `primary_key`
- `identity`
- `computed_sql`
- `rowversion`
- `index`
- `unique`
- `foreign_key`
- `on_delete`

The derive validates:

- only structs with named fields are accepted;
- field types must implement `SqlTypeMapping`;
- column names must not be empty;
- duplicate columns are rejected;
- unsupported attributes produce compile-time errors.

Example with application-specific audit columns:

```rust
#[derive(AuditFields)]
struct Audit {
    #[orm(column = "audit_created_stamp")]
    #[orm(updatable = false)]
    created_stamp: DateTime<Utc>,

    #[orm(column = "audit_created_actor")]
    #[orm(updatable = false)]
    created_actor: String,

    #[orm(column = "audit_modified_stamp")]
    #[orm(nullable)]
    modified_stamp: Option<DateTime<Utc>>,

    #[orm(column = "audit_modified_actor")]
    #[orm(nullable)]
    modified_actor: Option<String>,

    #[orm(column = "audit_source")]
    #[orm(length = 80)]
    source: String,
}
```

`insertable` and `updatable` decide whether a runtime value is valid for
`AuditOperation::Insert` or `AuditOperation::Update`. The ORM does not decide
that from field names such as `created_*` or `modified_*`.

## `#[orm(audit = Audit)]`

The entity attribute references a Rust type visible from the derive site:

```rust
#[derive(Entity)]
#[orm(table = "orders", schema = "sales", audit = Audit)]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}
```

The macro:

- requires `Audit` to implement `EntityPolicy`;
- rejects duplicate `audit` declarations;
- expands audit columns after the entity's own fields in stable order;
- rejects collisions between entity fields and audit columns;
- exposes the final columns through `EntityMetadata.columns`.

Audit columns are metadata/schema columns in the current release. They do not become visible Rust fields and do not generate symbols such as `Todo::created_at`. `FromRow` for the entity materializes only real Rust fields.

`#[derive(Entity)]` also implements `AuditEntity` for every entity. For entities declaring `#[orm(audit = Audit)]`, `audit_policy()` returns `Some(EntityPolicyMetadata)` with only the audit-owned columns. Entities without `audit` return `None`. This hook is runtime metadata over the already-expanded audit columns; it does not add a second snapshot, diff, DDL, row-mapping, or `EntityMetadata.columns` pipeline.

## Soft Delete

`soft_delete` is not a metadata-only feature. It changes runtime semantics for delete and read visibility.

Public shape:

```rust
#[derive(SoftDeleteFields)]
struct SoftDelete {
    #[orm(sql_type = "datetime2")]
    deleted_at: Option<String>,

    #[orm(nullable)]
    #[orm(length = 120)]
    deleted_by: Option<String>,
}

#[derive(Entity)]
#[orm(table = "todos", schema = "todo", soft_delete = SoftDelete)]
struct Todo {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    title: String,
}
```

Implemented behavior:

- `#[derive(SoftDeleteFields)]` implements an `EntityPolicy` named `soft_delete`.
- By default, soft-delete columns are `insertable = false` and `updatable = true`.
- `#[derive(Entity)]` accepts `#[orm(soft_delete = SoftDelete)]`.
- Soft-delete columns enter metadata and snapshots as normal columns.
- `DbSet::delete(...)`, Active Record `delete`, and deleted tracked entities use `UpdateQuery` instead of physical `DELETE`.
- Normal entities still use physical `DELETE`.
- `rowversion` and `OrmError::ConcurrencyConflict` remain respected.
- Public reads default to active-only visibility for the root entity.
- `with_deleted()` includes deleted rows.
- `only_deleted()` returns only logically deleted rows.

Visibility convention:

- the first soft-delete policy column controls visibility;
- nullable columns use `IS NULL` / `IS NOT NULL`;
- `BIT` columns use `false` / `true`.

Current limit: automatic soft-delete filtering applies only to the root entity of `DbSetQuery<E>`, not to every manually joined entity.

## Ergonomic Soft Delete Values

The low-level `SoftDeleteProvider` / `SoftDeleteRequestValues` contract remains the foundation for runtime delete values. It is flexible, but it forces users to build `ColumnValue`s manually. The ergonomic layer should be typed and should reuse the same `#[derive(SoftDeleteFields)]` struct that already defines the policy columns.

Implemented shape:

```rust
use chrono::{DateTime, Utc};
use sql_orm::prelude::*;

#[derive(SoftDeleteFields)]
struct SoftDelete {
    #[orm(deleted_at)]
    deleted_at: DateTime<Utc>,

    #[orm(deleted_by)]
    #[orm(nullable)]
    deleted_by: Option<String>,
}

let db = db.with_soft_delete_values(SoftDelete {
    deleted_at: Utc::now(),
    deleted_by: Some(current_user_id),
});
```

This avoids a separate predefined `SoftDeleteValues` struct. The user's policy struct is the runtime value shape. If an application only wants one column, it declares only one column:

```rust
#[derive(SoftDeleteFields)]
struct SoftDelete {
    #[orm(is_deleted)]
    #[orm(column = "deleted")]
    deleted: bool,
}

let db = db.with_soft_delete_values(SoftDelete { deleted: true });
```

The semantic markers are explicit metadata, not name inference:

- `#[orm(deleted_at)]` marks the timestamp column used when a row is logically deleted.
- `#[orm(deleted_by)]` marks the actor column.
- `#[orm(is_deleted)]` marks a boolean flag column.
- `#[orm(column = "...")]` keeps working when the physical column name differs from the Rust field.

The derive generates a runtime conversion trait:

```rust
pub trait SoftDeleteValues {
    fn soft_delete_values(self) -> Vec<ColumnValue>;
}
```

`DbContext` / `SharedConnection` can then expose a typed helper:

```rust
fn with_soft_delete_values<V: SoftDeleteValues>(&self, values: V) -> Self;
```

Internally this helper converts the typed struct into the existing low-level `SoftDeleteRequestValues` path. It does not replace `SoftDeleteProvider`, and it does not move delete semantics into `core`, `query`, `sqlserver`, or `tiberius`.

Design rules:

- The user selects columns by declaring fields in the `SoftDeleteFields` struct.
- The user selects semantic roles through explicit attributes, not by column-name guessing.
- No role is mandatory: `deleted_at`, `deleted_by`, and `is_deleted` are all optional.
- Runtime values are supplied only for the fields present in the value struct.
- The low-level `SoftDeleteProvider` and `SoftDeleteRequestValues` remain available as escape hatches.
- Existing precedence and duplicate validation rules must remain deterministic.

## Tenant

Tenant is a security feature, not just a schema convenience. It is opt-in per entity and fail-closed.

Public shape:

```rust
#[derive(TenantContext)]
struct CurrentTenant {
    #[orm(column = "tenant_id")]
    id: i64,
}

#[derive(Entity)]
#[orm(table = "orders", schema = "sales", tenant = CurrentTenant)]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    amount: i64,
}
```

Implemented behavior:

- `#[derive(TenantContext)]` accepts a struct with exactly one non-optional tenant field.
- The tenant context implements `EntityPolicy` with `POLICY_NAME = "tenant"`.
- `#[derive(Entity)]` accepts `#[orm(tenant = CurrentTenant)]`.
- Tenant columns enter metadata as normal columns.
- Entities without `tenant` are cross-tenant even when the context has an active tenant.
- `SharedConnection` transports `ActiveTenant { column_name, value }`.
- Derived contexts expose `with_tenant(...)` and `clear_tenant()`.
- Reads on tenant-scoped root entities add mandatory tenant predicates.
- Writes add mandatory tenant predicates.
- Inserts auto-fill the tenant column when missing, accept matching explicit values, and reject mismatched values.
- Internal existence checks for concurrency preserve tenant filtering.

Current limit: automatic tenant filtering applies to the root entity only. Filters for tenant-scoped manually joined entities must be written explicitly until the AST has a stronger alias and per-join metadata design.

## Runtime Audit Provider Design

Runtime audit auto-fill is implemented for audited inserts and semantic
updates through the main persistence paths. Soft-delete deletes compile as
`UPDATE` internally, but they remain delete semantics and do not consume the
audit update path in this cut.

## Ergonomic Audit Values

The low-level audit contract remains `AuditProvider` / `AuditRequestValues` over `ColumnValue`s. That is the escape hatch for advanced cases, but the ergonomic layer should be typed and should reuse the same `#[derive(AuditFields)]` struct that declares audit columns.

Implemented shape:

```rust
use chrono::{DateTime, Utc};
use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    #[orm(created_at)]
    created_at: DateTime<Utc>,

    #[orm(created_by)]
    created_by: String,

    #[orm(updated_at)]
    updated_at: DateTime<Utc>,

    #[orm(updated_by)]
    updated_by: String,
}

let db = db.with_audit_values(Audit {
    created_at: Utc::now(),
    created_by: current_user_id.clone(),
    updated_at: Utc::now(),
    updated_by: current_user_id,
});
```

This avoids a separate predefined value struct. The user's audit policy struct is also the runtime value shape. `#[derive(AuditFields)]` generates `AuditValues` for the struct:

```rust
pub trait AuditValues {
    fn audit_values(self) -> Vec<ColumnValue>;
}
```

`DbContext` / `SharedConnection` expose a typed helper:

```rust
fn with_audit_values<V: AuditValues>(&self, values: V) -> Self;
```

Internally this helper converts the typed struct into the existing `AuditRequestValues` path. It does not replace `AuditProvider`, and it does not move request context into `core`, `query`, `sqlserver`, or `tiberius`.

`with_audit_values(...)` always converts the whole `AuditFields` value into
column values. It does not inspect the current operation and omit fields for
you. If the struct contains creation-only columns marked `updatable = false`,
passing the same full struct to an update path can fail validation because those
columns are not updatable. For operation-specific audit values, prefer
`AuditProvider` or low-level `AuditRequestValues`, where the caller can return
only the columns that apply to the current operation.

The semantic markers are explicit metadata, not name inference:

- `#[orm(created_at)]` marks the creation timestamp value.
- `#[orm(created_by)]` marks the creation actor value.
- `#[orm(updated_at)]` marks the update timestamp value.
- `#[orm(updated_by)]` marks the update actor value.
- `#[orm(column = "...")]` keeps working when the physical column name differs from the Rust field.

Rules:

- The user selects columns by declaring fields in the `AuditFields` struct.
- The user supplies concrete values by passing an instance of that same struct to `with_audit_values(...)`.
- No audit role is mandatory; a policy may contain only the fields required by the application.
- Insert and update still filter by `AuditOperation` plus `insertable`/`updatable` metadata.
- `with_audit_values(...)` is best suited when all provided fields are valid for the write path being executed.
- Explicit mutation values keep highest precedence; typed audit values map to request values and therefore precede provider values.
- The low-level `AuditProvider` and `AuditRequestValues` remain available.

For example, this is valid for an insert when the creation columns are
insertable:

```rust
let db = db.with_audit_values(Audit {
    created_stamp: Utc::now(),
    created_actor: current_user_id.clone(),
    modified_stamp: None,
    modified_actor: None,
    source: "api".to_string(),
});
```

For updates with separate modified columns, use an operation-aware provider:

```rust
struct AppAuditProvider {
    user_id: String,
}

impl AuditProvider for AppAuditProvider {
    fn values(&self, context: AuditContext<'_>) -> Result<Vec<ColumnValue>, OrmError> {
        match context.operation {
            AuditOperation::Insert => Ok(vec![
                ColumnValue::new("audit_created_actor", SqlValue::String(self.user_id.clone())),
                ColumnValue::new("audit_source", SqlValue::String("api".to_string())),
            ]),
            AuditOperation::Update => Ok(vec![
                ColumnValue::new("audit_modified_actor", SqlValue::String(self.user_id.clone())),
            ]),
        }
    }
}
```

The implemented runtime contracts include:

- `AuditOperation::{Insert, Update}`;
- `AuditRequestValues`, an explicit per-request list of `ColumnValue`s;
- `AuditContext`, carrying entity metadata, operation, and optional request values;
- `AuditProvider`, a `Send + Sync` trait that returns provider values for a context;
- `resolve_audit_values(...)`, the internal/public-crate resolver used to codify precedence;
- `AuditEntity`, generated by `#[derive(Entity)]`, exposing audit-owned columns for audited entities.
- `SharedConnection::with_audit_provider(...)`, `with_audit_request_values(...)`, and `clear_audit_request_values()`;
- equivalent helpers generated by `#[derive(DbContext)]`.

The precedence rule is deterministic and non-overwriting:

1. values already produced by the mutation path win first;
2. `AuditRequestValues` fill columns that are still missing;
3. `AuditProvider` values fill columns that are still missing after request values.

Duplicates inside a single source are rejected. A lower-precedence source cannot overwrite a higher-precedence value; its value for an already-present column is ignored by the resolver. Write integration applies this rule only to columns declared by `AuditEntity::audit_policy()`; non-audit values returned by request/provider sources are ignored.

The implemented direction for writes is:

- `audit = Audit` remains the compile-time source of columns;
- `AuditProvider` supplies runtime values such as `now`, user id, or request values;
- mutation happens in the public `sql-orm` crate over normalized `Vec<ColumnValue>`;
- `core`, `query`, `sqlserver`, and `tiberius` do not learn request context;
- values are not inferred globally from column names.
- `DbSet::insert`, Active Record insert, and `save_changes()` for `Added` converge through the same normalized insert path.
- runtime values for `AuditOperation::Insert` must target insertable audit columns.
- `DbSet::update`, Active Record save over existing entities, and `save_changes()` for `Modified` converge through the same normalized update path.
- runtime values for `AuditOperation::Update` must target updatable audit columns.

Update integration preserves:

- explicit opt-in through `#[orm(audit = Audit)]`;
- no silent overwrite of user-provided values without a clear rule;
- deterministic handling inside transactions;
- compatibility with `Changeset`, Active Record, `save_changes()`, tenant filters, rowversion predicates, and soft-delete delete semantics.

Transaction contexts are created from the same `SharedConnection`, so configured audit provider and request values are inherited by the transaction context. Insert and update writes already consume that inherited runtime state.

Runtime audit must not auto-fill by matching names such as `created_at` or `updated_by`. Write integration must use `AuditEntity::audit_policy()` to know which flattened `EntityMetadata.columns` entries are audit-owned.

## Interactions

Policies can coexist, but collisions are rejected:

- entity field vs. audit column;
- entity field vs. soft-delete column;
- entity field vs. tenant column;
- audit vs. soft-delete;
- audit vs. tenant;
- soft-delete vs. tenant.

Runtime behavior is layered:

- audit contributes schema and insert/update runtime values when configured;
- soft delete decides whether delete compiles as `DELETE` or `UPDATE`;
- tenant decides the mandatory security boundary;
- rowversion still controls optimistic concurrency.

## Validation

Coverage includes:

- public `trybuild` fixtures for valid policy usage;
- compile-fail fixtures for invalid audit, soft-delete, and tenant shapes;
- metadata tests for column order, defaults, nullability, insertable/updatable flags, and collisions;
- migration tests proving policy columns enter snapshots, diffs, and DDL as normal columns;
- runtime tests for soft-delete behavior and audit write normalization, including insert/update compiled SQL coverage for `AuditProvider` and `AuditRequestValues`;
- public `trybuild` coverage for `AuditValues`, `with_audit_values(...)`, the low-level `AuditProvider`, `AuditRequestValues`, `AuditContext`, derived `DbContext` helpers, and `SharedConnection` helper surface;
- runtime and compiled-SQL tests for tenant filters and insert validation.

## Deferred Work

- `timestamps` as a separate policy or alias.
- Visible Rust fields for generated audit columns.
- Generated entity column symbols for policy-only columns.
- Automatic policy filters over all manually joined entities.
- Predefined global tenant conventions without a user-defined tenant context.
