# Tracking Unit Of Work

This document defines the target unit-of-work design for Etapa 21. It is the
implementation guide for replacing wrapper-lifetime persistence with a
context-owned tracker.

The current implementation now keeps pending `Added`, `Modified` and `Deleted`
work in registry-owned snapshots after wrapper drop or consume. This document
does not claim that the runtime has already been stabilized.

## Implementation Status

As of 2026-05-17, the first registry-backed unit-of-work slice is implemented:

- loaded tracked entities are registered with identity derived from entity
  type, schema, table and single-column primary key value,
- duplicate loaded identities in one context are rejected with `OrmError`,
- added entities use temporary local identities,
- successful tracked inserts update the registry identity to the persisted
  primary key returned by SQL Server,
- `Tracked<T>` exposes explicit public state APIs: `state()`,
  `mark_modified()`, `mark_deleted()`, `mark_unchanged()` and `detach()`,
- `DbSet::detach_tracked(...)` removes one wrapper from the current tracker,
- `DbContext::clear_tracker()` removes all current tracker entries,
  with unit coverage proving it does not reset the visible state of the
  detached wrappers,
- `save_changes()` skips `Modified` entries when their original/current
  snapshots produce the same `EntityPersist::update_changes()` payload,
- `save_changes()` plans tracked operations deterministically from context
  entity metadata: `Added` and `Modified` run parent tables before child tables
  for simple foreign keys present in the context, and `Deleted` runs the same
  order in reverse so child tables are deleted before parent tables. Focused
  unit coverage also fixes the no-dependency fallback order, foreign keys whose
  principal is outside the context, and simple self-FKs that must not create an
  artificial cycle,
- `save_changes()` opens an internal transaction when the shared connection is
  not already inside `db.transaction(...)`, and reuses the outer transaction
  when one is active,
- the `Added`, `Modified` and `Deleted` routes used by `save_changes()` have
  focused coverage proving they continue through the existing insert, update
  and delete pipelines for tenant predicates, rowversion predicates, audit
  provider/request values and soft-delete provider/request values,
- composite primary keys are an explicit first-stable-cut limit for tracking:
  `find_tracked(...)` and pending `save_changes()` routes return
  `OrmError` before SQL execution when the entity primary key is not a single
  column,
- no-op phase helpers short-circuit before SQL and before unsupported-primary-key
  validation: `save_tracked_added()` returns `0` when there are no `Added`
  entries, `save_tracked_modified()` returns `0` when there are no `Modified`
  entries, and `save_tracked_deleted()` returns `0` after an `Added` entry was
  cancelled and detached,
- Active Record interop has explicit wrapper semantics: `tracked.save(&db)`
  syncs the wrapper snapshot after immediate persistence, and
  `tracked.delete(&db)` detaches after immediate delete so `save_changes()`
  does not persist the same wrapper a second time,
- Active Record interop no-op/error states have additional unit coverage:
  `tracked.save(&db)` on a registered `Unchanged` wrapper does not require a
  connection and keeps the registry entry, `tracked.save(&db)` on a registered
  `Deleted` wrapper returns the stable tracking error without unregistering the
  pending delete, and `tracked.delete(&db)` on an already detached `Deleted`
  wrapper is idempotent without touching Active Record,
- navigation interop is intentionally non-graph-aware in this slice: includes
  and explicit loads do not register related entities, assignment of single or
  collection navigations to a tracked root does not mark the root `Modified`,
  and `save_changes()` does not persist relationship wrapper mutations.
- ownership behavior for the current registry-backed implementation is
  covered: cloning a `Tracked<T>` produces a detached wrapper that cannot
  unregister the original entry, and consuming a pending registered wrapper
  with `into_current()` keeps the work in the registry through `Drop`.
- registry collision behavior is covered when converting an `Added` temporary
  identity to a persisted primary key: duplicate persisted identities return
  `OrmError` and do not replace the temporary entry identity. This now covers
  collisions against both live wrappers and detached registry-owned entries.
- registry error/idempotency behavior is covered for the current internal
  slice: updating a missing registration returns a stable `OrmError`,
  unregistering a missing registration is a no-op, and public
  `Tracked::detach()` can be called repeatedly without resetting visible
  wrapper state.
- explicit `mark_unchanged()` restoration is covered for `Deleted` wrappers:
  it accepts the current value as the new original snapshot and returns the
  wrapper to `Unchanged` without database I/O. Registered wrappers expose that
  transition through the registry state reader as well.
- cancellation and detach no-ops are covered for additional edge cases:
  `remove_tracked(...)` is idempotent after cancelling an `Added` entry, and
  `detach_tracked(...)` on an `Added` entry prevents later insertion while
  preserving the wrapper's visible `Added` state.
- detach/clear edge cases now also cover pending deletes and mixed pending
  states: `detach_tracked(...)` on a `Deleted` entry prevents a later delete
  phase while preserving visible `Deleted` state, and `clear_tracker()` can
  discard `Added` plus `Deleted` entries before unsupported-primary-key
  validation or SQL execution.
- loaded identity comparison is covered as scoped by Rust entity type in
  addition to schema/table/primary-key value; two different Rust entity types
  may share the same physical table/key without being treated as duplicate
  tracked identities.
- public `trybuild` coverage now includes `Tracked<T>::save(&db)` and
  `Tracked<T>::delete(&db)` from `sql_orm::prelude`, context-level
  `find_tracked(...)`, `remove_tracked(...)`, `save_changes()` and ownership
  operations `clone`, `into_current()` and repeated `detach()`, and rejects
  direct access to internal registry attachment helpers.
- dropping or consuming a registered `Added`, `Modified` or `Deleted` wrapper
  now synchronizes the current wrapper value into the registry-owned snapshot,
  detaches only the handle, and leaves the pending insert/update/delete in the
  context registry for `save_changes()`.
- helper paths that accept no-op modifications or sync persisted rows update
  registry-owned snapshots and state even when the original wrapper has already
  been dropped. Focused coverage fixes both `RegisteredTracked::accept_current(...)`
  and `RegisteredTracked::sync_persisted(...)` for detached entries.
- the first identity-map cut can reattach a detached loaded identity through
  `find_tracked(...)`: the new wrapper receives the registry-owned
  original/current snapshots and state, while a second live wrapper for the
  same identity returns the documented duplicate live-handle `OrmError`.
- `load_collection_tracked(...)` now consults the registry for related entities
  that are already tracked and attaches their registry-owned current snapshots
  to the navigation collection, without registering newly materialized related
  rows automatically.
- `include(...)`, `include_many(...)` and ordinary `load_collection(...)` now
  use the same registry lookup for related entities that are already tracked in
  the context, without turning navigation materialization into implicit graph
  tracking.

The registry still stores a pointer while a `Tracked<T>` wrapper is alive so
mutable wrapper changes can be synchronized into the registry-owned current
snapshot. That pointer is cleared on wrapper drop for pending states; the unit
of work no longer requires the wrapper to stay alive for `Added`, `Modified`
or `Deleted` persistence.

The public tracking surface is stable for explicit tracking with simple
primary keys after the release-level Stage 21 validation and documentation
pass. The current implementation has removed wrapper lifetime as a persistence
requirement for pending work, while retaining documented limits around
relationship graph persistence and transactions from pooled contexts.

As of 2026-05-16, registry diagnostics expose a stable `entry_id` through
`TrackedEntityRegistration`. This is the first observable step toward owned
registry entries, but it does not change persistence behavior by itself.

As of 2026-05-16, the observable tracked state stored in
`TrackingRegistry::registrations()` and used by `tracked_for::<T>()` lives in
the registry entry itself. `Tracked<T>` state transitions synchronize that
registry-owned state.

As of 2026-05-16, registry entries also own typed original/current snapshot
clones captured at registration time. `mark_unchanged()`, immediate tracked
save and successful tracked persistence helpers synchronize those owned
snapshots when they accept or replace the wrapper value. The helpers used by
`save_changes()` now read current values and change-detection snapshot pairs
from the registry-owned snapshots. While `DerefMut` is still wrapper-backed,
those helpers first synchronize the registry current snapshot from the live
wrapper when it is still attached.

As of 2026-05-16, `Added`, `Modified` and `Deleted` entries all use
wrapper-drop handle detach semantics: dropping or consuming the wrapper
synchronizes its current value into the registry-owned snapshot, clears the
live pointer and leaves pending work in the registry. `Modified` entries can
still skip SQL after drop when their persisted update payload is unchanged,
because no-op acceptance updates registry-owned snapshots/state without
writing through a dead wrapper pointer.

## Current Detach And State Policy

The current stable explicit-tracking policy is:

- `Unchanged`: `save_changes()` ignores the entry. `detach_tracked(...)`,
  `Tracked::detach()`, `clear_tracker()` or dropping the wrapper removes it
  with no SQL. `mark_modified()` moves it to `Modified`; `mark_deleted()`
  moves it to `Deleted`.
- `Modified`: `save_changes()` persists through the normal update pipeline.
  `mark_unchanged()` accepts the current value as the new original snapshot and
  returns it to `Unchanged`. `detach_tracked(...)`, `Tracked::detach()`,
  or `clear_tracker()` discards the pending update from the tracker; dropping
  or consuming the wrapper keeps the pending update in the registry.
- `Added`: `save_changes()` persists through the normal insert pipeline and
  syncs the registry identity to the persisted key. `remove_tracked(...)`
  cancels the pending insert by marking the wrapper `Deleted` and detaching it.
  `mark_modified()` keeps it `Added`; `mark_unchanged()` accepts the current
  value as unchanged. `detach_tracked(...)`, `Tracked::detach()`,
  or `clear_tracker()` discards the pending insert without SQL; dropping or
  consuming the wrapper keeps the pending insert in the registry.
- `Deleted`: `save_changes()` persists through the normal delete pipeline,
  using soft-delete when the entity declares that policy, and unregisters the
  entry after success. `mark_modified()` keeps it `Deleted`; `mark_unchanged()`
  explicitly restores it to `Unchanged` by accepting the current snapshot.
  `detach_tracked(...)`, `Tracked::detach()` or `clear_tracker()` discards the
  pending delete from the tracker; dropping or consuming the wrapper keeps the
  pending delete in the registry.

## Active Record Interop

Active Record remains a convenience layer over `DbSet`; it is not a parallel
tracking pipeline.

Stable rules for the first cut:

- `Entity::query(&db)`, `Entity::find(&db, id)`, `entity.save(&db)` and
  `entity.delete(&db)` operate on ordinary entity values and do not
  automatically register those values in the tracker.
- `find_tracked(...)`, `add_tracked(...)`, `remove_tracked(...)`,
  `detach_tracked(...)`, `Tracked::detach()`, `Tracked::mark_modified()`,
  `Tracked::mark_deleted()`, `Tracked::mark_unchanged()`, `clear_tracker()`
  and `save_changes()` are the explicit tracking surface.
- `Tracked<T>` has inherent `save(&db)` and `delete(&db)` methods so method
  calls on a tracked wrapper do not silently dereference to the `ActiveRecord`
  implementation for `T`.
- `tracked.save(&db)` is a no-op for `Unchanged`, returns an error for
  `Deleted`, and for `Added`/`Modified` persists through Active Record, then
  synchronizes `original`, `current`, state and persisted registry identity.
- `tracked.delete(&db)` cancels local `Added` entries without SQL. For
  persisted `Unchanged`/`Modified` entries it delegates to Active Record delete
  and detaches the wrapper after a row is affected, so a later
  `save_changes()` cannot delete the same row again.
- Calling Active Record on an ordinary clone or detached entity remains outside
  the tracker. If that row is also tracked elsewhere in the same context, the
  tracker cannot observe the external mutation until the stable owned-registry
  identity-map work is completed.

Dropping a pending `Added`, `Modified` or `Deleted` wrapper is no longer
equivalent to detach. The explicit APIs `detach_tracked(...)`,
`Tracked::detach()` and `clear_tracker()` remain the ways to discard work from
the current unit of work.

## Navigation Interop

Navigation loading remains explicit and outside graph persistence in the first
stable cut.

Current rules:

- `include(...)` and `include_many(...)` materialize ordinary entity values;
  they do not automatically register roots or related rows in the tracker.
- already tracked related rows materialized by `include(...)` and
  `include_many(...)` are replaced with registry-owned current snapshots when
  the target has a simple primary key present in the registry.
- `load_collection_tracked(...)` attaches a collection to an already tracked
  root without changing the root state to `Modified`; already tracked related
  rows are attached from registry-owned current snapshots when their identity
  is present.
- ordinary `load_collection(...)` uses the same related-snapshot replacement
  rule while keeping the root and newly materialized related rows untracked.
- The same no-modification rule applies to single navigation assignment through
  the generated `IncludeNavigation<T>` contract.
- Related entities assigned into `Navigation<T>`, `LazyNavigation<T>`,
  `Collection<T>` or `LazyCollection<T>` are not tracked automatically.
- Mutating navigation wrappers is ignored by `save_changes()`; it does not
  insert dependents, delete dependents, update foreign keys or persist direct
  many-to-many changes.

The current identity-map slice is context-owned for tracked entries and shared
with navigation materialization only as related-entity snapshot replacement.
Registry-owned pending snapshots remove the immediate wrapper-lifetime blocker,
and `include(...)`, `include_many(...)`, ordinary `load_collection(...)` and
`load_collection_tracked(...)` now reuse already tracked related snapshots.
Root materialization and untracked related rows remain ordinary values.

## Future Relationship Persistence

Relationship persistence is deliberately outside the first stable tracking cut.
The current navigation wrappers describe loaded graph state; they are not
change commands.

Before `save_changes()` can persist relationship changes, the project must
define explicit graph update semantics for:

- dependent inserts attached through `Collection<T>` or `LazyCollection<T>`;
- dependent deletes removed from a loaded collection;
- foreign-key updates caused by moving a dependent between principals;
- optional versus required relationships and `SET NULL` behavior;
- direct many-to-many exclusions, where explicit join entities remain the
  supported shape;
- conflict behavior when a related row is also tracked or modified separately;
- ordering and rollback behavior when relationship changes mix with ordinary
  `Added`, `Modified` and `Deleted` entity operations.

The future implementation must still route persistence through the existing
`DbSet` insert, update and delete paths so tenant, audit, soft-delete,
rowversion and SQL Server execution boundaries remain centralized. It must not
move SQL generation into `sql-orm-query` or execution into tracking.

## Goal

`save_changes()` must persist changes owned by the `DbContext`, not by the
lifetime of individual `Tracked<T>` values.

The stable unit of work must:

- store tracked entries inside the context-owned `TrackingRegistry`,
- identify persisted rows by deterministic entity identity,
- keep pending operations after a `Tracked<T>` wrapper is dropped,
- avoid duplicate tracked rows for the same persisted identity,
- preserve existing `DbSet` insert/update/delete policy pipelines,
- and leave SQL compilation in `sql-orm-sqlserver` and execution in
  `sql-orm-tiberius`.

## Current Baseline

Today, `TrackingRegistry` owns typed original/current snapshots and stores a
raw address of `TrackedInner<T>` only while the wrapper is still attached.

That has two important limits:

- `DerefMut` mutations still occur on the wrapper and must be synchronized
  into the registry snapshot before the handle is detached,
- unchanged dropped wrappers are still removed because they carry no pending
  work.

Stage 21 completed the stable cut by moving pending work ownership to the
context registry while keeping a live pointer only for synchronizing attached
wrappers.

## Ownership Model

The registry becomes the owner of tracked entries.

Target shape:

```rust
pub struct TrackingRegistry {
    state: Mutex<TrackingRegistryState>,
}

struct TrackingRegistryState {
    next_entry_id: u64,
    entries: Vec<TrackedEntry>,
}

struct TrackedEntry {
    entry_id: u64,
    identity: TrackedIdentity,
    original: Box<dyn TrackedSnapshot>,
    current: Box<dyn TrackedSnapshot>,
    state: EntityState,
}
```

`Tracked<T>` becomes a typed handle over one registry entry:

```rust
pub struct Tracked<T> {
    entry_id: u64,
    registry: TrackingRegistryHandle,
    detached_value: Option<T>,
}
```

The wrapper may cache or clone values for ergonomic access, but the registry is
the source of truth for `save_changes()`. Dropping the wrapper must not
unregister the entry.

## Identity Key

Each persisted tracked row uses a deterministic identity:

```rust
struct TrackedIdentity {
    entity_type: TypeId,
    rust_name: &'static str,
    schema: &'static str,
    table: &'static str,
    primary_key: PrimaryKeyIdentity,
}

enum PrimaryKeyIdentity {
    Simple(SqlValue),
}
```

The first stable cut keeps composite primary keys out of scope. Entities with a
composite primary key fail with a stable error when loaded through
`find_tracked(...)` or when pending `Added`, `Modified` or `Deleted` entries
are persisted through `save_changes()`. `add_tracked(...)` can still create a
temporary in-memory entry because it is infallible and does not need the
database key yet; that entry fails before SQL execution if `save_changes()` is
called. `remove_tracked(...)` remains an infallible state transition and the
same `save_changes()` guard applies when a composite-key entry is pending
delete.

For `Added` entities without a database-generated key yet, the registry uses a
temporary local identity:

```rust
enum PrimaryKeyIdentity {
    Simple(SqlValue),
    Temporary(u64),
}
```

After insert, the entry identity is replaced with the materialized persisted
primary key returned by SQL Server.

## Duplicate Tracking

Stable behavior must avoid duplicate persisted identities in one context.

Rules:

- the first stable cut permits only one live `Tracked<T>` handle for the same
  persisted identity in one context.
- `find_tracked(id)` reattaches a detached registry entry for the same
  persisted identity and returns a wrapper initialized from the registry-owned
  snapshots.
- `find_tracked(id)` returns an error if another live wrapper is still attached
  to the same persisted identity; callers must detach or drop that wrapper
  before loading the same identity again.
- navigation materialization through `include(...)`, `include_many(...)`,
  `load_collection(...)` and `load_collection_tracked(...)` may clone a
  registry-owned current snapshot for related rows that are already tracked,
  but it does not create a second live tracked handle.
- `add_tracked(entity)` uses a temporary identity until insert when the entity
  has an identity/generated key.
- persisted identity collisions discovered after insert/update of a temporary
  entry fail without mutating the existing entry.
- identity comparison uses entity type, schema, table and primary key value.

Supporting multiple live handles for the same persisted row remains outside
the first stable cut because it would require explicit canonical mutation
semantics over heterogeneous registry entries. The supported policy is to
reattach registry entries whose previous live wrapper was dropped or consumed,
and reject duplicate live handles.

## State Ownership

State lives in the registry entry. Wrapper methods delegate to the registry.

Stable state transitions:

- `Unchanged -> Modified` through explicit `mark_modified()` or value mutation,
- `Unchanged -> Deleted` through `remove_tracked(...)`,
- `Modified -> Unchanged` through explicit accept/sync after persistence,
- `Modified -> Deleted` through `remove_tracked(...)`,
- `Added -> Unchanged` after successful insert,
- `Added -> Deleted` as local cancellation,
- `Deleted -> detached` after successful delete.

Dropping `Tracked<T>` does not change state. Explicit `detach(...)` removes the
registry entry and makes the handle detached.

## Snapshot Contract

The registry needs typed snapshots without runtime reflection.

The implementation should introduce a root-crate trait generated or implemented
for entities:

```rust
pub trait TrackedEntitySnapshot: Entity + Clone + Send + 'static {
    fn persisted_identity(&self) -> Result<Option<PrimaryKeyIdentity>, OrmError>;
    fn current_snapshot(&self) -> Self;
    fn has_persisted_changes(original: &Self, current: &Self) -> bool;
}
```

The first runtime slice may conservatively keep mutable access as `Modified`.
`has_persisted_changes(...)` skips updates when persisted columns did not
change, ignoring navigation wrappers, identity, computed, rowversion and
non-updatable columns.

Generated comparison belongs in `sql-orm-macros` and public traits in
`sql-orm`. It must not be placed in `sql-orm-query`,
`sql-orm-sqlserver` or `sql-orm-tiberius`.

The current implementation uses `EntityPersist::has_persisted_changes(...)`,
whose default compares `original.update_changes()` with
`current.update_changes()`. That gives structural change detection over the
same generated updatable-column payload used by updates. It therefore ignores
navigation wrappers, primary keys, identity columns, rowversion columns,
computed columns and non-updatable columns, because those values are not part
of `update_changes()`.

## Save Pipeline

`save_changes()` remains generated by `#[derive(DbContext)]` and asks the
shared registry for entries by entity type instead of depending on wrapper
lifetime.

Per entity type:

1. collect registry entries for the context field entity type,
2. persist `Added` through `DbSet::insert_entity(...)`,
3. persist `Modified` through `DbSet::update_entity_by_sql_value(...)`,
4. persist `Deleted` through `DbSet::delete_tracked_by_sql_value(...)`,
5. sync successful entries back into the registry,
6. detach entries deleted successfully.

The current implementation keeps the phase order `Added -> Modified ->
Deleted`, but no longer relies on raw context field order inside each phase.
`#[derive(DbContext)]` asks `sql-orm` for a metadata-based operation plan.
For simple foreign keys between entities present in the same context, inserts
and updates run parent tables before child tables and deletes run child tables
before parent tables. Ties are resolved by the original context field order.
Foreign-key cycles are rejected with `OrmError`. Composite foreign keys and
self-references remain outside this ordering guarantee in the current slice.

## Transaction Boundary

The unit of work must be compatible with both direct connections and
transaction contexts.

The transaction slice of Etapa 21 is implemented for direct shared
connections:

- registry state is shared across context clones created by policy helpers and
  `db.transaction(...)`,
- save execution must keep using each `DbSet`'s existing `SharedConnection`,
- no SQL execution is introduced inside `TrackingRegistry`,
- `SharedConnection` tracks active transaction depth in runtime state shared by
  policy-derived connection handles,
- generated `save_changes()` starts `db.transaction(...)` internally when no
  transaction is active,
- generated `save_changes()` executes its persistence body directly when an
  outer `db.transaction(...)` is already active, avoiding nested `BEGIN
  TRANSACTION` calls.

This guarantees atomicity for the current registry-backed `save_changes()`
execution on direct connections. Contexts backed by pools remain blocked for
transactions until Etapa 22 pins one physical pooled connection for the entire
transaction closure.

## Public API Surface

The implementation following this design exposed the explicit APIs required for
stabilization:

- `Tracked<T>::state()`,
- `Tracked<T>::mark_modified()`,
- `Tracked<T>::mark_unchanged()`,
- `DbSet::remove_tracked(&mut Tracked<T>)`,
- `DbSet::detach(&mut Tracked<T>)`,
- `DbContext::clear_tracker()`,
- `DbContext::tracked_entries()` or a read-only equivalent for diagnostics.

These APIs are exposed from `sql_orm::prelude` after Stage 21 tests and
rustdoc validation.

## Migration Steps

Implementation should be split in this order:

1. introduce owned registry entry identifiers and public diagnostics without
   changing persistence behavior,
2. move `Tracked<T>` state reads/writes through registry-owned entries,
3. stop unregistering on `Drop`,
4. add explicit detach/clear APIs,
5. add duplicate identity detection for simple primary keys,
6. update `save_changes()` helpers to iterate owned registry snapshots,
7. add no-op change detection,
8. add deterministic FK-aware operation ordering,
9. finalize transaction behavior and public docs.

Each step must keep `core`, `query`, `sqlserver`, `tiberius`, `migrate` and
`cli` responsibilities unchanged.

## Out Of Scope

This design deliberately excludes:

- composite primary key persistence in the first stable cut,
- automatic lazy loading,
- graph-wide cascade persistence,
- direct many-to-many mutation persistence,
- SQL generation inside tracking,
- and Tiberius-specific state in the registry.
