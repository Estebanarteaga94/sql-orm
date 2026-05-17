# Code-First Guide

`sql-orm` treats Rust code as the source of truth for the model. Entities, policies, relationships, and contexts are declared in Rust and expanded into static metadata by proc macros.

See also [Core concepts](core-concepts.md).

## Entity

```rust
use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
pub struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(length = 180)]
    #[orm(unique)]
    pub email: String,

    #[orm(length = 120)]
    pub name: String,
}
```

`#[derive(Entity)]` generates:

- `Entity` metadata;
- `FromRow` materialization for real Rust fields;
- static column symbols such as `User::email`;
- internal persistence contracts used by the public crate.

## Supported Column Attributes

Common field attributes include:

- `#[orm(primary_key)]`
- `#[orm(identity)]`
- `#[orm(column = "db_column")]`
- `#[orm(length = 120)]`
- `#[orm(nullable)]`
- `#[orm(unsafe_default_sql = "...")]`
- `#[orm(sql_type = "datetime2")]`
- `#[orm(unsafe_sql_type = "vendor_specific_type")]`
- `#[orm(precision = 18)]`
- `#[orm(scale = 2)]`
- `#[orm(unsafe_computed_sql = "...")]`
- `#[orm(rowversion)]`
- `#[orm(index(name = "..."))]`
- `#[orm(unique)]`
- `#[orm(foreign_key(entity = User, column = id))]`
- `#[orm(on_delete = "cascade" | "set null" | "no action")]`
- `#[orm(renamed_from = "...")]`

Entity-level attributes include `table`, `schema`, explicit table `renamed_from`, composite indexes, and entity policies such as `audit`, `soft_delete`, and `tenant`.

## Insertable and Changeset

Use separate write models to avoid mixing database-generated fields with user-provided payloads.

```rust
#[derive(Insertable)]
#[orm(entity = User)]
pub struct NewUser {
    pub email: String,
    pub name: String,
}

#[derive(Changeset)]
#[orm(entity = User)]
pub struct UpdateUser {
    pub email: Option<String>,
    pub name: Option<String>,
}
```

`Insertable` extracts insert values. `Changeset` extracts update values and can carry a `rowversion` concurrency token when the target entity has one.

## DbContext

```rust
#[derive(DbContext)]
pub struct AppDb {
    pub users: DbSet<User>,
}
```

The derive generates:

- connection constructors;
- typed `DbSet<T>` fields;
- public helpers for runtime state such as audit, tenant and soft-delete values;
- `MigrationModelSource` for snapshot export.

## Entity Policies

Entity Policies add reusable model concerns without creating a second schema pipeline.

### Audit Metadata

```rust
#[derive(AuditFields)]
pub struct Audit {
    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    #[orm(updatable = false)]
    pub created_at: String,

    #[orm(nullable)]
    #[orm(length = 120)]
    pub updated_by: Option<String>,
}

#[derive(Entity)]
#[orm(table = "todos", schema = "todo", audit = Audit)]
pub struct Todo {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    pub title: String,
}
```

Audit columns are metadata/schema columns. They do not become visible Rust
fields on `Todo` and do not generate `Todo::created_at`. Audited write paths
can fill missing audit-owned values at runtime through `AuditProvider`,
`AuditRequestValues` or typed `with_audit_values(...)` values.

### Soft Delete

`#[derive(SoftDeleteFields)]` plus `#[orm(soft_delete = SoftDelete)]` enables runtime soft-delete behavior for the root entity. Deletes become updates, default reads hide logically deleted rows, and explicit APIs can include or select deleted rows.

### Tenant

`#[derive(TenantContext)]` plus `#[orm(tenant = CurrentTenant)]` enables explicit tenant scoping. Tenant-scoped entities fail closed when no compatible active tenant exists. Root-entity reads and writes apply mandatory tenant predicates.

## Migrations

`ModelSnapshot::from_entities(...)` consumes generated metadata. Policy columns enter snapshots as normal columns, so migrations do not need a separate schema path for policies.

## Limits

- SQL Server is the only backend.
- Public CRUD and Active Record are oriented around simple primary keys.
- Audit policy columns are not Rust fields and do not generate entity column symbols.
- Audit runtime auto-fill covers the main `DbSet`, Active Record and
  `save_changes()` insert/update paths when audited entities have missing
  audit-owned values.
- `migration.rs` is outside the current MVP.
