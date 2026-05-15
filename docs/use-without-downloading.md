# Use From Another Project

You can consume `sql-orm` from another Rust project through crates.io or directly from Git. You do not need to manually clone this repository into the consuming project.

## crates.io

```toml
[dependencies]
sql-orm = "0.1.0"
```

With optional pooling:

```toml
[dependencies]
sql-orm = { version = "0.1.0", features = ["pool-bb8"] }
```

## Git Dependency

Use this form when you need an unreleased branch or a specific commit:

```toml
[dependencies]
sql-orm = { git = "https://github.com/Estebanarteaga94/sql-orm.git", package = "sql-orm" }
```

To pin a branch, tag, or revision:

```toml
[dependencies]
sql-orm = { git = "https://github.com/Estebanarteaga94/sql-orm.git", package = "sql-orm", branch = "main" }
```

```toml
[dependencies]
sql-orm = { git = "https://github.com/Estebanarteaga94/sql-orm.git", package = "sql-orm", rev = "<commit-sha>" }
```

## Optional Pooling

```toml
[dependencies]
sql-orm = { git = "https://github.com/Estebanarteaga94/sql-orm.git", package = "sql-orm", features = ["pool-bb8"] }
```

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

- Cargo downloads Git dependencies into its own cache.
- Prefer pinning a revision for reproducible builds.
- Do not commit connection strings or credentials.
- The root public API is `sql_orm::prelude::*`.
