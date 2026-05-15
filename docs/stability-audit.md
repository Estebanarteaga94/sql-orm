# Stability Audit

This audit records the public surfaces still marked as experimental, pending
verification, deferred, planned, or explicitly unavailable as of 2026-04-30.
It is based on `README.md`, `CHANGELOG.md`, `docs/context.md`,
`docs/core-concepts.md`, `docs/api.md`, and public rustdoc in the root crate.

The master plan requested as `plan_orm_sqlserver_tiberius_code_first.md` is not
present at the repository root. The active plan file is
`docs/plan_orm_sqlserver_tiberius_code_first.md`.

## Audit Result

The only public surface still explicitly marked as experimental is change
tracking:

- `Tracked<T>`
- `EntityState`
- `DbSet::find_tracked(...)`
- `DbSet::add_tracked(...)`
- `DbSet::remove_tracked(...)`
- `DbContext::save_changes()`

These APIs are implemented, tested as an experimental cut, and exported from
the public crate, but they are not yet stable. The current implementation keeps
registrations tied to live wrappers, marks `Modified` on mutable access instead
of structural diffing, supports persistence through the existing simple-PK
CRUD routes, and does not track navigation graphs or relationship changes.

## Deferred Or Blocked Surfaces

These surfaces are documented as unavailable or blocked, not experimental:

| Surface | Current status | Next backlog stage |
| --- | --- | --- |
| Transactions from `DbContext::from_pool(...)` | Blocked by design until one physical pooled connection is pinned for the full transaction closure. Direct-connection transactions remain available. | Etapa 22 |
| `include_many(...).split_query()` | Public builder method exists, but execution returns a clear not-implemented error. Join-based collection include is the available path. | Future navigation follow-up |
| Direct many-to-many navigation | Rejected; use an explicit join entity with ordinary foreign keys and supported navigation edges. | Future relationship-update design |
| Automatic lazy loading | Not available; lazy wrappers are state containers and never issue SQL by themselves. | Future explicit async loader design |
| Navigation graph tracking and relationship persistence | Not stable; includes and explicit loads do not register related entities automatically, and `save_changes()` does not persist relationship mutations. | Etapa 21 |
| High-level typed aggregations and `group_by` | Not implemented as public query builder APIs. | Etapa 24 |
| `database downgrade` | CLI command does not exist yet; `down.sql` artifacts exist when reversible but are not executed automatically. | Etapa 23 |
| `migration.rs` | Deferred from the migration MVP; current artifacts are `up.sql`, `down.sql`, and `model_snapshot.json`. | Future migrations stage |
| Composite primary key persistence | Metadata supports composite PKs, but public CRUD, Active Record, and tracking persistence remain centered on simple primary keys. | Future persistence hardening |
| Automatic policy filters over manually joined entities | Not available; `soft_delete` and `tenant` automatic filters apply to root entities and selected include predicates, not arbitrary manual joins. | Future policy/query design |

## Pending Verification

`README.md` still marks fresh validation of `examples/todo-app` against real SQL
Server as pending. Historical validation exists in `docs/worklog.md`, but the
warning should remain until a current session reruns the example with a real
connection string and records the evidence.

## Rustdoc Findings

Public rustdoc already marks the same stability boundaries:

- `crates/sql-orm/src/tracking.rs` names the tracking module as
  experimental and lists its runtime limits.
- `DbContext::transaction(...)` documents the pool-backed transaction block.
- `DbSetQueryIncludeMany::split_query()` documents that execution is not
  implemented.

No additional public rustdoc surface was found that presents the deferred
items above as stable.

## Documentation Consistency

The audited documents are consistent with the current implementation:

- `README.md` lists the current limits and keeps `todo-app` validation marked
  as pending.
- `CHANGELOG.md` separates `0.1.0` available features from `0.2.0` planned
  stabilization, transactions-from-pool, downgrade, and aggregations work.
- `docs/api.md` and `docs/core-concepts.md` mark tracking as experimental and
  document deferred surfaces as current limits.
- `docs/context.md` records that Etapa 21 must stabilize tracking before
  removing the experimental label.

This audit does not graduate any API to stable status. The next executable
task is to define explicit stability criteria for `Tracked<T>` and
`save_changes()` before changing behavior.

The stability criteria for that next step are now recorded in
[Tracking stability criteria](tracking-stability.md).
