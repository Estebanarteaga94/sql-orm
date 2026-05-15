# Contributing

This repository is a Rust workspace for a SQL Server-first, code-first ORM built on Tiberius. Contributions should preserve the project architecture and keep changes small, traceable, and validated.

## Architecture Rules

- `sql-orm-core` owns contracts, metadata, shared types, and errors.
- `sql-orm-macros` owns derives and generated metadata.
- `sql-orm-query` owns the query AST and must not generate SQL.
- `sql-orm-sqlserver` owns SQL Server query and DDL compilation.
- `sql-orm-tiberius` owns execution, connections, rows, transactions, and Tiberius adaptation.
- `sql-orm-migrate` owns migration snapshots, diffs, operations, and filesystem helpers.
- `sql-orm-cli` owns command-line workflows.
- `sql-orm` owns the public user-facing API and prelude.

Do not introduce multi-database abstractions in this phase. SQL Server is the only target.

## Before Changing Code

Read the operational documentation first:

- `docs/instructions.md`
- `docs/tasks.md`
- `docs/worklog.md`
- `docs/context.md`
- `docs/plan_orm_sqlserver_tiberius_code_first.md`

If there is a conflict about contracts, metadata shape, or crate responsibilities, the master plan wins.

## Workflow

1. Pick one concrete task from `docs/tasks.md`.
2. Move it to `En Progreso` before implementation when that file is part of the active tracked workflow.
3. Inspect the current code before editing.
4. Make the smallest change that completes the task.
5. Validate with the narrowest relevant checks, and broaden validation when the blast radius is larger.
6. Update operational documentation when behavior, state, or decisions change.
7. Commit completed and validated work.

When a file is intentionally ignored by `.gitignore`, do not modify it as part of a repository-wide documentation change unless the user explicitly asks for local-only notes.

## Validation

Use the checks that apply to the change:

```bash
cargo fmt --all --check
cargo check --workspace
cargo test --workspace
cargo clippy --workspace --all-targets --all-features
```

For targeted changes, run focused tests first. For public API, macros, migrations, or query compilation, prefer adding or updating focused unit tests, snapshot tests, and `trybuild` fixtures.

Real SQL Server tests require `SQL_ORM_TEST_CONNECTION_STRING`.

## Documentation

Documentation should describe implemented behavior accurately. If a claim is not verified in the current repository state, mark it as pending verification or keep it out of public docs.

Keep public docs aligned with:

- `README.md`
- `docs/core-concepts.md`
- `docs/repository-audit.md`
- feature guides under `docs/`
- examples under `examples/`

## Git Hygiene

- Do not revert unrelated user changes.
- Do not use destructive Git commands unless explicitly requested.
- Keep generated or local-only files out of Git.
- Commit only the files needed for the task.

## Pull Request Expectations

A useful PR should include:

- a clear problem statement;
- a small implementation scope;
- tests or a reason tests are not applicable;
- documentation updates when public behavior changes;
- notes about any remaining limits or risks.
