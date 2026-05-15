# Security Policy

`sql-orm` is an early-stage ORM for Microsoft SQL Server. Security reports should focus on issues that can affect data isolation, SQL execution, migration safety, connection handling, or runtime behavior.

## Supported Versions

The project has not published a stable release yet. Until a stable release exists, security fixes target the current `main` branch.

## Reporting a Vulnerability

If you find a security issue, do not open a public issue with exploit details. Contact the maintainers through the private channel configured for the repository or provide a minimal private report with:

- affected commit or version;
- reproduction steps;
- expected and actual behavior;
- impact assessment;
- any known workaround.

If no private channel is configured, open a public issue with a high-level description only and ask for a private disclosure path.

## Sensitive Areas

Pay special attention to:

- SQL injection risk in raw SQL or identifier construction;
- parameter binding and placeholder ordering;
- tenant filters and fail-closed behavior;
- soft-delete behavior and accidental physical deletes;
- migration destructive-change detection;
- transaction boundaries and rollback behavior;
- connection-string handling;
- logging of sensitive query parameters;
- concurrency handling with `rowversion`.

## Current Limits

- SQL Server is the only supported database backend.
- Raw SQL is an explicit escape hatch and does not automatically apply tenant or soft-delete filters.
- Runtime audit-field auto-fill is implemented for audited insert/update paths, but raw SQL and semantic soft-delete deletes remain explicit escape hatches.
- Pooled transactions remain blocked until one physical connection can be pinned for the full transaction closure.

## Guidance for Contributors

- Keep secrets out of tests, docs, examples, and logs.
- Prefer parameterized values over string interpolation.
- Use allowlists for dynamic identifiers.
- Preserve fail-closed tenant behavior.
- Do not weaken migration destructive-change blocking without an explicit design.
- Add regression tests for security fixes whenever possible.
