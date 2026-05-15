# Release Checklist

This workspace is published as an experimental `0.1.0` multi-crate release.

## Preflight

Run these checks from the workspace root:

```bash
cargo fmt --all --check
cargo test --workspace --all-features
cargo package -p sql-orm-core
cargo package -p sql-orm-macros
```

Before the first dependent crate is published, `cargo package` / `cargo publish
--dry-run` for crates that depend on unpublished internal crates will fail with
`no matching package named ... found`. Publish in dependency order below and
rerun `cargo publish --dry-run` for each crate immediately before publishing it.

## Publish Order

```bash
cargo publish -p sql-orm-core
cargo publish -p sql-orm-macros
cargo publish -p sql-orm-query
cargo publish -p sql-orm-migrate
cargo publish -p sql-orm-sqlserver
cargo publish -p sql-orm-tiberius
cargo publish -p sql-orm
```

The CLI can be published after the library crates:

```bash
cargo publish -p sql-orm-cli
```

If crates.io index propagation is slow, wait briefly and retry the next crate.

## Release Notes

- `0.1.0` is experimental.
- SQL Server is the only supported backend.
- `Tracked<T>` and `save_changes()` remain experimental.
- Pooled transactions and `database downgrade` are not yet supported.
