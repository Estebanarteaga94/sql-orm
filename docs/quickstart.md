# Quickstart

This guide shows the minimum path for connecting `sql-orm`, defining a model, using `DbContext`, running basic CRUD, and executing a public query-builder query.

It is written against the current repository state:

- SQL Server is the only database target.
- The normal user API is `sql_orm::prelude::*`.
- `#[derive(Entity)]`, `#[derive(Insertable)]`, `#[derive(Changeset)]`, and `#[derive(DbContext)]` are available.
- SQL is compiled by `sql-orm-sqlserver` and executed by the Tiberius adapter.

See also [Core concepts](core-concepts.md).

## 1. Add the Dependency

From an external project, use the root crate:

```toml
[dependencies]
sql-orm = "0.2.0-rc.1"
```

If you want optional pooling:

```toml
[dependencies]
sql-orm = { version = "0.2.0-rc.1", features = ["pool-bb8"] }
```

## 2. Import the Prelude

```rust
use sql_orm::prelude::*;
```

The prelude contains the public derives, `DbContext`, `DbSet`, query extensions, errors, metadata contracts, and common SQL values.

## 3. Define an Entity, Write Models, and Context

```rust
use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "customers", schema = "sales")]
pub struct Customer {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(length = 160)]
    #[orm(unique)]
    pub email: String,

    #[orm(length = 120)]
    pub full_name: String,

    #[orm(nullable)]
    #[orm(length = 30)]
    pub phone: Option<String>,
}

#[derive(Insertable)]
#[orm(entity = Customer)]
pub struct NewCustomer {
    pub email: String,
    pub full_name: String,
    pub phone: Option<String>,
}

#[derive(Changeset)]
#[orm(entity = Customer)]
pub struct UpdateCustomer {
    pub full_name: Option<String>,
    pub phone: Option<Option<String>>,
}

#[derive(DbContext)]
pub struct AppDb {
    pub customers: DbSet<Customer>,
}
```

## 4. Connect

Use a connection string from your own environment:

```rust
let db = AppDb::connect(
    "Server=localhost;Database=tempdb;User Id=sa;Password=Password123;\
     TrustServerCertificate=True;Encrypt=False"
).await?;
```

For examples and integration tests, prefer environment variables such as `DATABASE_URL` or `SQL_ORM_TEST_CONNECTION_STRING`.

## 5. Insert and Find

```rust
let saved = db
    .customers
    .insert(NewCustomer {
        email: "ana@example.com".to_string(),
        full_name: "Ana Perez".to_string(),
        phone: None,
    })
    .await?;

let found = db.customers.find(saved.id).await?;
```

## 6. Update and Delete

```rust
let updated = db
    .customers
    .update(
        saved.id,
        UpdateCustomer {
            full_name: Some("Ana Maria Perez".to_string()),
            phone: Some(Some("+57 300 000 0000".to_string())),
        },
    )
    .await?;

let deleted = db.customers.delete(saved.id).await?;
```

`update` returns `Ok(None)` when no row matches in the simple non-concurrency case. Entities with `rowversion` can return `OrmError::ConcurrencyConflict` when the primary key still exists but the token is stale.

## 7. Query

```rust
let customers = db
    .customers
    .query()
    .filter(Customer::email.contains("@example.com"))
    .order_by(Customer::email.asc())
    .take(20)
    .all()
    .await?;
```

The public query builder produces an AST. SQL Server SQL is generated only by `sql-orm-sqlserver`.

## 8. Next Reading

- Public API inventory: [api.md](api.md)
- Code-first guide: [code-first.md](code-first.md)
- Query builder guide: [query-builder.md](query-builder.md)
- Migrations guide: [migrations.md](migrations.md)
- Raw SQL guide: [raw-sql.md](raw-sql.md)
