# Use From Another Project

You can consume `sql-orm` from another Rust project through crates.io. You do not need to manually clone this repository into the consuming project.

## crates.io

```toml
[dependencies]
sql-orm = "0.2.0-rc.2"
```

With optional pooling:

```toml
[dependencies]
sql-orm = { version = "0.2.0-rc.2", features = ["pool-bb8"] }
```

Published package: <https://crates.io/crates/sql-orm>

API documentation: <https://docs.rs/sql-orm>

## CLI

Install the migration CLI from crates.io:

```bash
cargo install sql-orm-cli
```

Published package: <https://crates.io/crates/sql-orm-cli>

API documentation: <https://docs.rs/sql-orm-cli>

## Basic Consumer Code

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

#[derive(DbContext)]
struct AppDb {
    users: DbSet<User>,
}
```

## Migration Snapshot Export

If your project wants to use `migration add --snapshot-bin`, add a small binary that prints the model snapshot:

```rust
use sql_orm::prelude::*;

fn main() {
    print!(
        "{}",
        sql_orm::model_snapshot_json_from_source::<AppDb>()
            .expect("snapshot should serialize")
    );
}
```

Then call the CLI with the consumer manifest:

```bash
sql-orm-cli migration add CreateSchema \
  --manifest-path path/to/consumer/Cargo.toml \
  --snapshot-bin model_snapshot
```

## Notes

- Cargo downloads crates.io dependencies into its own cache.
- Use `Cargo.lock` in applications for reproducible builds.
- Do not commit connection strings or credentials.
- The root public API is `sql_orm::prelude::*`.
