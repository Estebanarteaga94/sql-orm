# Relationships and Joins

In `sql-orm`, a relationship declared with `foreign_key` produces relational metadata, migration snapshots, diffs, and SQL Server DDL. Queries remain explicit: declaring a foreign key does not make joins implicit, and navigation loading must be requested through the public query APIs.

See also [Core concepts](core-concepts.md) and
[Navigation properties](navigation.md).

## Declaring a Foreign Key

A one-to-many relationship is declared on the dependent entity field that stores the local column.

```rust
#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists", schema = "todo")]
pub struct TodoList {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub owner_id: i64,
}
```

The structured form is preferred because it points to a Rust entity type and a generated target column symbol. The macro validates at compile time that the referenced column exists.

## Legacy String Syntax

The string syntax remains supported for compatibility:

```rust
#[orm(foreign_key = "users.id")]
pub owner_id: i64,

#[orm(foreign_key = "todo.users.id")]
pub owner_id: i64,
```

With two segments, the referenced schema defaults to `dbo`. With three segments, the first segment is the schema.

## Constraint Names

If no name is declared, the derive generates a stable name from the local table, local column, and referenced table.

Generated names are intended for deterministic metadata and migration output, not as a public naming convention guarantee for all future releases.

## Delete Behavior

The current public surface supports:

- `#[orm(on_delete = "no action")]`
- `#[orm(on_delete = "cascade")]`
- `#[orm(on_delete = "set null")]`

`set null` requires the local column to be nullable. The derive rejects non-nullable `set null` at compile time.

## Metadata Helpers

`ForeignKeyMetadata` and `EntityMetadata` expose helpers for inspecting relationships by name, local column, or referenced table. These helpers are for inspection, migrations, and tooling; they do not execute queries.

## Migrations and DDL

Derived foreign keys enter the code-first pipeline as normal metadata:

```text
EntityMetadata -> ModelSnapshot -> MigrationOperation -> SQL Server DDL
```

Generated DDL uses:

```sql
ALTER TABLE ... ADD CONSTRAINT ... FOREIGN KEY ... REFERENCES ...
```

and preserves `ON DELETE` when applicable.

The public derive syntax declares foreign keys from individual fields. Snapshots, diffs, and DDL already have shapes for composite foreign keys, but automatically deriving them from public attributes is outside this phase.

## Explicit Joins

Foreign keys describe the model. Joins decide how a specific query uses related tables.

```rust
let rows = db
    .todo_lists
    .query()
    .inner_join::<User>(TodoList::owner_id.eq(User::id))
    .filter(User::id.eq(7_i64))
    .all()
    .await?;
```

Use `left_join::<T>(...)` when the relationship can be missing or when you need to preserve rows from the base entity.

## Materialization

The default public `DbSetQuery<T>` materializes entities from the base table (`T`). Joins are used to filter or order through related tables. A first `include::<T>(...)` cut exists for single navigations and explicitly constructs one related `Navigation<T>`.

## Navigation Surface

For the full navigation guide, including wrapper behavior, eager loading,
explicit loading, many-to-many modeling, policies, tracking limits and runtime
validation, see [Navigation properties](navigation.md).

Navigation properties are available in the current workspace cut. The implemented surface supports syntax, metadata, table aliases, explicit join inference from navigation metadata, eager loading for one `belongs_to` / `has_one` navigation, join-based `has_many` eager loading, explicit `has_many` collection loading from materialized roots, and opt-in lazy state wrappers that never perform I/O by themselves. Fields can declare navigation attributes, the derive excludes those fields from column metadata, and `EntityMetadata.navigations` exposes neutral relationship metadata.

The relationship kinds are:

- `belongs_to`: the dependent entity stores the foreign key and points to one principal entity.
- `has_one`: the principal entity points to at most one dependent entity.
- `has_many`: the principal entity points to a collection of dependent entities.
- `many_to_many`: initially modeled through an explicit join entity. Direct many-to-many navigation remains a later layer until update semantics are stable.

The supported field shapes are marker wrappers, not persisted columns:

```rust
#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists", schema = "todo")]
pub struct TodoList {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub owner_id: i64,

    #[orm(belongs_to(User, foreign_key = owner_id))]
    pub owner: Navigation<User>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(has_many(TodoList, foreign_key = owner_id))]
    pub lists: Collection<TodoList>,
}
```

`Navigation<T>` and `Collection<T>` are public marker/value wrappers. The derive does not turn those fields into `ColumnMetadata`; it only uses them to generate navigation metadata. Materializing an entity without an explicit include/load initializes these wrappers empty.

`LazyNavigation<T>` and `LazyCollection<T>` are opt-in lazy-state wrappers. They
are accepted by the same `belongs_to`, `has_one` and `has_many` attributes, are
also excluded from column metadata, and start unloaded when materialized from a
row. They do not store a connection or context and do not query from ordinary
field access.

### Many-To-Many Through an Explicit Entity

Direct `#[orm(many_to_many(...))]` navigation is intentionally rejected in this
release line. Model many-to-many relationships with an explicit join entity so
schema, inserts, deletes, audit, tenant and soft-delete behavior stay ordinary
and visible.

```rust
#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(has_many(UserRole, foreign_key = user_id))]
    pub user_roles: Collection<UserRole>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "roles", schema = "todo")]
pub struct Role {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(has_many(UserRole, foreign_key = role_id))]
    pub user_roles: Collection<UserRole>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "user_roles", schema = "todo")]
pub struct UserRole {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub user_id: i64,

    #[orm(foreign_key(entity = Role, column = id))]
    pub role_id: i64,

    #[orm(belongs_to(User, foreign_key = user_id))]
    pub user: Navigation<User>,

    #[orm(belongs_to(Role, foreign_key = role_id))]
    pub role: Navigation<Role>,
}
```

Query through the join entity using explicit joins, navigation joins or
`include(...)` / `include_many(...)` on the supported `belongs_to` and
`has_many` edges. Persisting direct many-to-many relationship changes is also
explicit: insert or delete `UserRole` rows directly. Mutating
`User.roles`-style direct collections is not supported because `save_changes()`
does not define stable relationship-update semantics for added/removed links,
duplicate links, composite keys, tenant boundaries or soft-deleted join rows.

### Explicit Navigation Joins

`DbSetQuery` can build a SQL join predicate from a declared navigation:

```rust
let lists = db
    .users
    .query()
    .try_left_join_navigation_as::<TodoList>("lists", "lists")?
    .filter(TodoList::id.aliased("lists").gt(0_i64))
    .all()
    .await?;
```

The helper validates that the navigation exists on the root entity and that its
target table matches the joined entity type. It uses `local_columns` and
`target_columns` from `NavigationMetadata` to construct the `ON` predicate.
This is not eager loading: the query still materializes the root entity or an
explicit projection only.

### `include(...)` for Single Navigations

The current eager-loading API is explicit and supports `belongs_to` / `has_one`:

```rust
let lists = db
    .todo_lists
    .query()
    .include::<User>("owner")?
    .all()
    .await?;
```

The implementation uses a left join, projects root columns with their normal
aliases, projects included columns with an internal prefix, materializes the
related row through `FromRow`, and attaches it to the root `Navigation<T>`.
When the joined side is absent, the navigation stays empty.

If the included entity declares `tenant` or `soft_delete`, those filters are
applied inside the include join predicate. This preserves `LEFT JOIN`
semantics: a related row hidden by tenant or soft-delete policy leaves the
navigation empty instead of dropping the root entity. Tenant-scoped included
entities fail closed when the context has no compatible active tenant.

`include_as::<T>("owner", "owner_alias")` is available when the query needs a
specific SQL table alias.

The include query can still be refined before execution with root or aliased
related predicates:

```rust
let lists = db
    .todo_lists
    .query()
    .include_as::<User>("owner", "owner")?
    .filter(User::id.aliased("owner").gt(0_i64))
    .order_by(User::id.aliased("owner").desc())
    .take(20)
    .all()
    .await?;
```

Projection DTOs remain separate from includes; `include` materializes root
entities and attaches a `Navigation<T>`.

### `include_many(...)` for Collection Navigations

`has_many` collection eager loading is exposed separately:

```rust
let users = db
    .users
    .query()
    .include_many_as::<TodoList>("lists", "lists")?
    .max_joined_rows(2_000)
    .all()
    .await?;

let lists = users[0].lists.as_slice();
```

The first implementation uses a left join and groups joined rows by the root
entity primary key before assigning `Collection<T>`. This avoids returning
duplicate root entities to the caller. Pagination is rejected on this path
because limiting joined rows would produce unstable root-entity pages.

The default join strategy has a safety limit of 10,000 joined rows before
grouping. `max_joined_rows(...)` adjusts that limit for a query, and
`unbounded_join()` is available only as an explicit opt-out. `split_query()` is
available on the builder to make the intended large-collection strategy
visible, but it currently returns an error until the two-query loader is
implemented.

For large collections, the preferred implemented direction remains split
queries:

```text
1. Load root rows.
2. Load related rows with one filtered query.
3. Attach related rows to the matching root navigation collection.
```

Split queries keep row duplication predictable and avoid forcing every large
collection include through a wide join. The execution path for split queries
remains a separate backlog item.

### Explicit Loading

The current explicit loading cut supports `has_many` collections from a
materialized root entity:

```rust
let mut user = db.users.find(7_i64).await?.expect("user");

db.users
    .load_collection::<TodoList>(&mut user, "lists")
    .await?;

let lists = user.lists.as_slice();
```

For tracked entities, use the tracked variant:

```rust
let mut user = db.users.find_tracked(7_i64).await?.expect("user");

db.users
    .load_collection_tracked::<TodoList>(&mut user, "lists")
    .await?;
```

The tracked variant attaches the collection without marking the entity as
`Modified`. This first cut supports single-column root primary keys where the
`has_many` local column is that primary key. It is an explicit async call; no
field access performs I/O.

### Change Tracking Boundary

Navigation loading and stable explicit `Tracked<T>` are intentionally connected
only through the current context registry and the root entity for this cut:

- `include(...)` and `include_many(...)` materialize ordinary entity values;
  they do not automatically register the root or included entities in the
  tracking registry.
- `load_collection_tracked(...)` attaches a collection to the current tracked
  root without changing its state from `Unchanged` to `Modified`. If a related
  row is already tracked in the same context, the collection receives the
  registry-owned current snapshot for that identity.
- `include(...)`, `include_many(...)` and ordinary `load_collection(...)` also
  reuse registry-owned snapshots for related rows that are already tracked in
  the same context.
- Related entities loaded into `Navigation<T>`, `Collection<T>`,
  `LazyNavigation<T>` or `LazyCollection<T>` are not automatically tracked.
- Mutating a navigation field through tracked relationship helpers is persisted
  by `save_changes()` only for the validated simple-FK slice: dependent insert,
  FK move and optional removal as `SET NULL`. Required removals without
  explicit dependent delete fail before SQL. Direct many-to-many updates remain
  out of scope.

This is identity-map reuse plus a narrow graph-persistence slice, not broad
graph tracking. Navigation loading still does not register loaded graphs
automatically. Wrapper mutations are translated only for simple FK/PK
relationship commands; many-to-many link updates still go through explicit join
entities.

### Identity Map Design

The navigation/tracking integration is built around one identity map owned by
the context. The key is deterministic:

```text
(entity Rust type, schema, table, primary-key column values)
```

For tracked root queries and navigation loading, materialization can consult
that identity map before returning or attaching a related entity:

1. Build the entity from the row using `FromRow`.
2. Compute its identity key from metadata and primary-key values.
3. Reuse the existing tracked instance when the key is already present.
4. Insert one tracked instance when the key is new.
5. Attach navigation wrappers to those canonical instances.

This path avoids stale related values when a matching tracked snapshot already
exists in the same context. It preserves the current explicitness rules: raw
SQL does not join the identity map automatically, newly materialized related
rows are not registered automatically, and disconnected entities remain plain
values unless the caller explicitly tracks them.

Relationship persistence remains deliberately scoped. The current identity-map
cut defines snapshot reuse for already tracked identities, and the graph
planner handles simple FK/PK commands from tracked wrapper mutations. It still
does not infer direct many-to-many changes or composite relationship
persistence from wrapper mutations.

### Opt-In Lazy Loading

Lazy loading is not a default behavior. The first executable cut is limited to
opt-in wrapper state and explicit loading integration. Normal entity field
access never performs I/O.

The shape is explicit at the entity type:

```rust
#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists", schema = "todo")]
pub struct TodoList {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub owner_id: i64,

    #[orm(belongs_to(User, foreign_key = owner_id))]
    pub owner: LazyNavigation<User>,

    #[orm(has_many(TodoItem, foreign_key = list_id))]
    pub items: LazyCollection<TodoItem>,
}
```

Lazy wrappers expose memory-only state APIs:

```rust
assert!(!list.owner.is_loaded());
assert_eq!(list.owner.as_ref(), None);
```

The current executable contract is:

- `LazyNavigation<T>` / `LazyCollection<T>` are separate wrappers from eager-loaded `Navigation<T>` / `Collection<T>`.
- `include(...)` / `include_many(...)` can populate lazy wrappers and mark them loaded.
- `load_collection(...)` / `load_collection_tracked(...)` can populate `LazyCollection<T>` and mark it loaded.
- A future single-navigation explicit loader may populate `LazyNavigation<T>` outside an include; it must receive an explicit context-bearing value, such as `&DbSet<E>` or a future entry API.
- Lazy wrappers do not store an open SQL Server connection by themselves.
- A loaded value is cached inside the wrapper for that entity instance until the caller clears or refreshes it through an explicit method.
- The wrapper exposes state inspection, for example `is_loaded()`, so code can avoid accidental repeated loads.
- Missing single navigations produce an empty loaded state, matching `Navigation<T>`.

This keeps the I/O boundary visible in Rust syntax: `await` appears where the query happens, and ordinary field reads remain memory-only.

#### Why Implicit Loading Is Not The First Executable Cut

Rust async and ownership make transparent lazy loading a poor default for this ORM:

- `async` work cannot run inside a normal `Deref` or field accessor, so implicit loading would require blocking, hidden runtimes, or surprising APIs.
- Storing context or connection handles inside every entity would blur the current architecture where execution stays in `sql-orm-tiberius` and public entity values remain plain data.
- Entity clones would need clear rules for whether they share lazy state, cached values, and connection capability.
- Long-lived entities holding context references would introduce lifetime constraints that are hard to compose with web handlers, transactions and pools.
- Hidden per-row loads create N+1 query regressions that are hard to see in review and telemetry.

For those reasons, the stable path remains explicit eager loading with
`include(...)` / `include_many(...)`, explicit collection loading with
`load_collection(...)`, explicit joins for query shaping, and raw SQL for fully
manual shapes.

#### Required Guardrails Before Implementation

A broader automatic lazy-loading implementation would need all of these
guardrails before becoming available:

- Opt-in field types only; existing `Navigation<T>` and `Collection<T>` must not become lazy by default.
- No query from `Deref`, `as_ref`, `as_slice`, `Debug`, `Clone`, serialization or equality operations.
- Explicit context parameter on every load call, so no entity silently owns a connection.
- Compatibility with `tenant` and default `soft_delete` filters equal to explicit loading.
- Clear behavior inside transactions: a load inside a transaction must use that transaction's context-bearing value, not a separate pooled connection.
- Repeated-load semantics, cache invalidation and refresh behavior documented and tested.
- Diagnostics or API friction for collection lazy loading, because `LazyCollection<T>` is the highest N+1 risk.
- Tests proving that constructing, cloning, reading and formatting lazy wrappers do not execute SQL.

### Policies and Projections

Navigation loading must preserve existing safety behavior:

- `tenant` filters apply to included tenant-scoped entities inside the include `JOIN ... ON` predicate and fail closed when the active tenant is missing or incompatible.
- default `soft_delete` visibility applies to included soft-deleted entities inside the include `JOIN ... ON` predicate; a future API may add an explicit include-time visibility override.
- Raw SQL remains explicit, does not infer navigation filters and does not attach `Navigation<T>` / `Collection<T>`.
- `select(...)`, `all_as::<T>()`, `first_as::<T>()` and DTO projections remain separate from entity graph materialization; include builders intentionally do not expose projection methods.

### Required Infrastructure

Navigation support depends on earlier internal work:

- navigation metadata in `sql-orm-core`;
- macro validation for navigation fields that are not columns;
- table aliases in `sql-orm-query`;
- SQL Server alias compilation in `sql-orm-sqlserver`;
- explicit navigation join inference in `DbSetQuery`;
- materialization that can separate root columns from included-entity columns;
- grouping by root primary key for `has_many` collection includes;
- tests for repeated joins, self-joins, `tenant`, `soft_delete`, and row ordering.

## Limits

- `include::<T>(...)` currently supports one `belongs_to` or `has_one` navigation.
- `include_many::<T>(...)` currently supports one `has_many` navigation, defaults to join loading with a 10,000 joined-row safety limit, exposes explicit `split_query()`, and rejects pagination in the join-based loading path.
- `load_collection::<T>(...)` currently supports `has_many` collection loading for single-column root primary keys.
- Lazy wrappers exist, but they do not query by themselves; there is no automatic single-navigation lazy loader yet.
- No automatic projection of joined entity graphs.
- Tenant and soft-delete automatic filters apply to the root entity and to explicitly included entities; filters on manually joined entities must be explicit.
