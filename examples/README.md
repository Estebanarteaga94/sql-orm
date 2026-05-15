# Examples

This directory contains runnable examples for `sql-orm`.

## Available Examples

- [`todo-app`](todo-app/README.md): async web-style example with a relational domain, public query builder usage, HTTP handlers, optional pooling, health checks, and migration snapshot export.

## Running Examples

Examples that talk to SQL Server require a real connection string. Use local environment variables rather than committing credentials:

```bash
export DATABASE_URL='Server=localhost;Database=tempdb;User Id=sa;Password=Password123;TrustServerCertificate=True;Encrypt=False'
```

See each example README for its own setup and smoke-test instructions.

## Notes

- Examples are consumers of the public crate API.
- They should not rely on private crate internals.
- If an example documents a feature, keep it aligned with the corresponding guide under `docs/`.
