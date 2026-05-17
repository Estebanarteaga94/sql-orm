# Stability Audit

This audit records the public surfaces still marked as experimental, pending
verification, deferred, planned, or explicitly unavailable as of 2026-05-17.
It is based on `README.md`, `CHANGELOG.md`, `docs/context.md`,
`docs/core-concepts.md`, `docs/api.md`, and public rustdoc in the root crate.

The master plan requested as `plan_orm_sqlserver_tiberius_code_first.md` is not
present at the repository root. The active plan file is
`docs/plan_orm_sqlserver_tiberius_code_first.md`.

## Audit Result

No root public surface is currently documented as experimental. `Tracked<T>`,
`EntityState`, `find_tracked(...)`, `add_tracked(...)`, `remove_tracked(...)`
and `save_changes()` were stabilized for explicit single-primary-key tracking
after Etapa 21 validation. Relationship graph persistence remains outside that
stable cut.

## Deferred Or Blocked Surfaces

These surfaces are documented as unavailable or blocked, not experimental:

| Surface | Current status | Next backlog stage |
| --- | --- | --- |
| `include_many(...).split_query()` | Public builder method exists, but execution returns a clear not-implemented error. Join-based collection include is the available path. | Future navigation follow-up |
| Direct many-to-many navigation | Rejected; use an explicit join entity with ordinary foreign keys and supported navigation edges. | Future relationship-update design |
| Automatic lazy loading | Not available; lazy wrappers are state containers and never issue SQL by themselves. | Future explicit async loader design |
| Navigation graph tracking and relationship persistence | Not stable; includes and explicit loads do not register related entities automatically, and `save_changes()` does not persist relationship mutations. | Etapa 21 |
| Typed aggregations advanced SQL shapes | Scalar and grouped aggregate APIs are implemented. Window functions, rollups, cubes and distinct aggregates remain outside the current cut. | Future aggregation follow-up |
| `database downgrade` | Implemented in Etapa 23 as explicit-target script generation plus opt-in `--execute`; requires local `up.sql` checksum validation and executable `down.sql`. | Available with limits |
| `migration.rs` | Deferred from the migration MVP; current artifacts are `up.sql`, `down.sql`, and `model_snapshot.json`. | Future migrations stage |
| Composite primary key persistence | Metadata supports composite PKs, but public CRUD, Active Record, and tracking persistence remain centered on simple primary keys. | Future persistence hardening |
| Automatic policy filters over manually joined entities | Not available; `soft_delete` and `tenant` automatic filters apply to root entities and selected include predicates, not arbitrary manual joins. | Future policy/query design |

## SQL Server Example Verification

`examples/todo-app` was revalidated against real SQL Server on 2026-05-17 using
local `tempdb`. The current recorded evidence covers fixture setup with
`sqlcmd`, the ignored smoke test using `DATABASE_URL`, HTTP read endpoints, and
the migration script apply path. Future release candidates should rerun the
same flow before claiming fresh validation.

## Rustdoc Findings

Public rustdoc already marks the same stability boundaries:

- `crates/sql-orm/src/tracking.rs` documents the stable explicit tracking
  guarantees and remaining graph-persistence limits.
- `DbContext::transaction(...)` supports direct connections and, with
  `pool-bb8`, contexts created from `from_pool(...)` by pinning one physical
  pooled connection for the closure.
- `DbSetQueryIncludeMany::split_query()` documents that execution is not
  implemented.

No additional public rustdoc surface was found that presents the deferred
items above as stable.

## Documentation Consistency

The audited documents are consistent with the current implementation:

- `README.md` lists the current limits and records the latest real SQL Server
  validation date for `todo-app`.
- `CHANGELOG.md` records `0.2.0-rc.1` as the current pre-release candidate
  and keeps `0.1.0` as historical release context.
- `docs/api.md` and `docs/core-concepts.md` document the stabilized explicit
  tracking surface and deferred limits.
- `docs/context.md` records that Etapa 21 through Etapa 24 have been validated
  and that remaining items are roadmap limits.

The stability criteria used to graduate explicit tracking are recorded in
[Tracking stability criteria](tracking-stability.md).
