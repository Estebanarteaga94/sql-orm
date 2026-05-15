# Navigation Properties

Navigation properties let an entity expose related entities as Rust fields while
keeping database access explicit. They are code-first metadata plus explicit
loading APIs; ordinary field access never performs I/O.

See also [Relationships and joins](relationships.md), [Query builder](query-builder.md)
and [Core concepts](core-concepts.md).

## Current Surface

The current implementation supports:

- `belongs_to` single navigation metadata and eager loading.
- `has_one` single navigation metadata and eager loading.
- `has_many` collection navigation metadata and join-based eager loading.
- explicit collection loading from a materialized entity or `Tracked<T>`.
- table aliases for repeated joins and self-joins.
- navigation join inference from `EntityMetadata.navigations`.
- lazy state wrappers that track loaded/unloaded state but never query by
  themselves.
- direct many-to-many modeling through an explicit join entity.

The current implementation intentionally does not support:

- direct `many_to_many` navigation attributes.
- hidden lazy loading from field access.
- automatic nested include chains.
- split-query execution for `include_many(...)`.
- automatic relationship persistence from navigation wrapper mutations.
- stable graph tracking or identity map behavior.

## Wrapper Types

Navigation fields use wrapper types from `sql_orm::prelude`:

- `Navigation<T>`: eager single navigation value, empty until loaded.
- `Collection<T>`: eager collection navigation value, empty until loaded.
- `LazyNavigation<T>`: single navigation with loaded/unloaded state.
- `LazyCollection<T>`: collection navigation with loaded/unloaded state.

These wrappers are not persisted columns. `#[derive(Entity)]` excludes them
from `ColumnMetadata`, initializes eager wrappers empty, initializes lazy
wrappers unloaded, and emits neutral `NavigationMetadata`.

## Declaring `belongs_to`

Use `belongs_to` on the dependent entity. The same entity must also declare a
structured foreign key field.

```rust
use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(length = 120)]
    pub name: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "posts", schema = "todo")]
pub struct Post {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub user_id: i64,

    #[orm(length = 160)]
    pub title: String,

    #[orm(belongs_to(User, foreign_key = user_id))]
    pub user: Navigation<User>,
}
```

The macro validates that `foreign_key = user_id` points to a structured
`#[orm(foreign_key(entity = User, column = id))]` declaration and that the
target type matches.

## Declaring `has_one`

Use `has_one` on the principal entity when at most one dependent row should be
attached.

```rust
#[derive(Entity, Debug, Clone)]
#[orm(table = "profiles", schema = "todo")]
pub struct Profile {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub user_id: i64,

    #[orm(length = 200)]
    pub bio: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(length = 120)]
    pub name: String,

    #[orm(has_one(Profile, foreign_key = user_id))]
    pub profile: Navigation<Profile>,
}
```

`has_one` reuses the `ForeignKeyMetadata` generated on the target entity. It
does not create a second constraint or a second migration path.

## Declaring `has_many`

Use `has_many` on the principal entity when many dependent rows can be attached.

```rust
#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(length = 120)]
    pub name: String,

    #[orm(has_many(Post, foreign_key = user_id))]
    pub posts: Collection<Post>,
}
```

`has_many` also reuses the foreign key metadata declared on the target entity.
It is not a persisted field and it does not change insert/update payloads.

## Lazy Wrappers

Use `LazyNavigation<T>` or `LazyCollection<T>` only when the entity needs to
distinguish unloaded state from loaded empty state.

```rust
#[derive(Entity, Debug, Clone)]
#[orm(table = "posts", schema = "todo")]
pub struct Post {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub user_id: i64,

    #[orm(belongs_to(User, foreign_key = user_id))]
    pub user: LazyNavigation<User>,
}
```

Lazy wrappers are memory-only state containers:

```rust
assert!(!post.user.is_loaded());
assert!(post.user.as_ref().is_none());
```

They do not store a `DbContext`, `DbSet`, connection, transaction or pool
handle. `Debug`, `Clone`, `as_ref()`, `as_slice()` and normal field access do
not execute SQL.

## Navigation Metadata

Navigation declarations generate `NavigationMetadata` inside
`EntityMetadata.navigations`.

Metadata records:

- Rust field name.
- navigation kind: `BelongsTo`, `HasOne` or `HasMany`.
- target Rust type name.
- target schema and table.
- local columns.
- target columns.
- associated foreign key name when available.

This metadata is neutral. It lives in `sql-orm-core`, does not depend on
Tiberius, and does not generate SQL by itself.

## Navigation Joins

Navigation joins infer the `ON` predicate from metadata but still behave like
ordinary joins. They shape filters, ordering or projections; they do not attach
related entities to wrappers.

```rust
let posts = db
    .users
    .query()
    .try_left_join_navigation_as::<Post>("posts", "posts")?
    .filter(Post::title.aliased("posts").contains("release"))
    .order_by(Post::id.aliased("posts").asc())
    .all()
    .await?;
```

Use the `_as` variants when the query needs aliases for repeated joins,
self-joins or filters over the joined table.

Available helpers:

- `try_inner_join_navigation::<T>("field")`
- `try_left_join_navigation::<T>("field")`
- `try_inner_join_navigation_as::<T>("field", "alias")`
- `try_left_join_navigation_as::<T>("field", "alias")`

The helpers are fallible because they validate that the navigation exists and
that the target table matches the joined entity type.

## Eager Loading Single Navigations

Use `include::<T>(...)` or `include_as::<T>(...)` for one `belongs_to` or
`has_one` navigation.

```rust
let posts = db
    .posts
    .query()
    .include_as::<User>("user", "user")?
    .filter(User::name.aliased("user").contains("Ana"))
    .all()
    .await?;

let author = posts[0].user.as_ref();
```

`include(...)` uses a `LEFT JOIN`, projects root columns normally, projects
included columns with internal aliases, materializes the included row with
`FromRow`, and attaches it through the generated `IncludeNavigation<T>`
contract.

When the related row is absent, the root row still materializes and the
navigation remains empty.

After configuring a single include, the builder still supports:

- `filter(...)`
- explicit joins
- `order_by(...)`
- `limit(...)` / `take(...)`
- `paginate(...)`
- `with_deleted()` / `only_deleted()` for the root entity
- `all().await`
- `first().await`

It intentionally does not expose `select(...)`, `all_as::<T>()` or
`first_as::<T>()`.

## Eager Loading Collections

Use `include_many::<T>(...)` or `include_many_as::<T>(...)` for one `has_many`
navigation.

```rust
let users = db
    .users
    .query()
    .include_many_as::<Post>("posts", "posts")?
    .max_joined_rows(2_000)
    .filter(Post::title.aliased("posts").contains("release"))
    .all()
    .await?;

let posts = users[0].posts.as_slice();
```

The current collection include implementation:

- uses one `LEFT JOIN`;
- projects root and related columns;
- materializes joined rows;
- groups rows by the root entity primary key;
- assigns each related collection to the correct root;
- returns each root entity once.

Pagination is rejected for this join-based path because SQL Server pagination
over joined rows does not produce stable pages of root entities.

The default safety limit is 10,000 joined rows before grouping. Use:

- `max_joined_rows(n)` to set a known bound.
- `unbounded_join()` to explicitly accept an unbounded joined result.
- `join_strategy()` to inspect the selected strategy.
- `split_query()` to select the future split-query strategy.

`split_query()` is public so callers can express intent, but execution returns a
clear not-implemented error in the current cut.

## Explicit Collection Loading

Use explicit loading when the root entity is already materialized.

```rust
let mut user = db.users.find(1_i64).await?.expect("user");

db.users
    .load_collection::<Post>(&mut user, "posts")
    .await?;

assert!(!user.posts.as_slice().is_empty());
```

For tracked roots:

```rust
let mut user = db.users.find_tracked(1_i64).await?.expect("user");

db.users
    .load_collection_tracked::<Post>(&mut user, "posts")
    .await?;

assert_eq!(user.state(), EntityState::Unchanged);
```

The tracked variant attaches the collection without marking the root as
`Modified`. Related entities are not registered in the tracking registry.

The current explicit loading cut supports `has_many` navigations where the root
has a simple primary key and the navigation local column is that primary key.

## Bounded Graph Loading

There is no automatic nested include chain yet. Load bounded graphs through
explicit steps.

```rust
let mut user = db
    .users
    .query()
    .include_as::<Profile>("profile", "profile")?
    .filter(User::id.eq(1_i64))
    .first()
    .await?
    .expect("user");

db.users
    .load_collection::<Post>(&mut user, "posts")
    .await?;
```

This makes the I/O boundary visible: each database operation has an explicit
`.await`. It also avoids hidden per-row queries and keeps ownership simple.

## Many-To-Many

Direct many-to-many navigation is intentionally rejected:

```rust
#[orm(many_to_many(Role))]
pub roles: Collection<Role>,
```

Model many-to-many with an explicit join entity:

```rust
#[derive(Entity, Debug, Clone)]
#[orm(table = "user_roles", schema = "todo")]
pub struct UserRole {
    #[orm(primary_key)]
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

Then query and persist `UserRole` rows directly. This keeps schema, auditing,
tenant filtering, soft delete and relationship updates explicit.

## Projections And Raw SQL

Includes are for entity graph materialization. Projections are a separate route.

Use `select(...).all_as::<T>()` for flat DTOs:

```rust
#[derive(Debug, FromRow)]
struct PostSummary {
    id: i64,
    #[orm(column = "author_name")]
    author: String,
}

let rows = db
    .posts
    .query()
    .try_inner_join_navigation_as::<User>("user", "user")?
    .select((
        Post::id,
        SelectProjection::expr_as(sql_orm::query::Expr::from(User::name.aliased("user")), "author_name"),
    ))
    .all_as::<PostSummary>()
    .await?;
```

Raw SQL does not inspect navigation metadata, infer joins or attach wrappers.
When using raw SQL, write all joins and policy filters explicitly.

## Runtime Policies

Navigation loading preserves the existing `tenant` and `soft_delete` safety
rules.

For root entities:

- tenant filters are added to the effective root query;
- default soft-delete visibility is added to the effective root query.

For included entities:

- tenant filters are added to the include `JOIN ... ON` predicate;
- default soft-delete visibility is added to the include `JOIN ... ON`
  predicate;
- missing or filtered related rows leave the navigation empty instead of
  dropping the root row;
- tenant-scoped included entities fail closed when the active tenant is missing
  or incompatible.

Manual joins are different: filters on manually joined entities must be written
explicitly by the caller.

## Change Tracking Boundary

Navigation loading does not make change tracking graph-aware.

Current rules:

- `include(...)` and `include_many(...)` return ordinary entity values.
- included roots and related entities are not automatically registered in the
  tracking registry.
- single navigation values assigned through the include contract on a
  `Tracked<T>` root do not mark the root as `Modified` and do not register the
  related entity.
- `load_collection_tracked(...)` attaches a collection to the tracked root
  without changing the root state to `Modified`.
- related entities loaded into wrappers are not automatically tracked.
- mutating navigation wrappers does not cause `save_changes()` to insert,
  delete or update relationship rows.

The planned stable direction is a context-owned identity map shared by roots,
includes and explicit loads. That remains future tracking stabilization work
because the current registry still depends on live `Tracked<T>` wrappers.

Relationship persistence is also future work. Removing an item from a loaded
collection, assigning a different parent navigation, or filling a navigation
wrapper does not currently mean "insert", "delete", "set null" or "update the
foreign key". Until graph update semantics are designed and validated, persist
relationship changes through ordinary entity operations: insert/update/delete
the dependent entity or explicit join entity directly.

## Validation

Navigation runtime behavior is covered by:

- unit tests for metadata, wrappers, aliasing and SQL compilation;
- `trybuild` fixtures for public valid and invalid navigation APIs;
- SQL snapshots for includes, aliases, repeated joins, self-joins, tenant,
  soft delete and parameter order;
- optional SQL Server integration test
  `crates/sql-orm/tests/stage20_navigation_runtime.rs`.

Run the focused runtime test with:

```bash
SQL_ORM_TEST_CONNECTION_STRING='<connection-string>' \
  cargo test -p sql-orm --test stage20_navigation_runtime -- --nocapture --test-threads=1
```

To keep the test tables for manual inspection:

```bash
KEEP_TEST_TABLES=1 SQL_ORM_TEST_CONNECTION_STRING='<connection-string>' \
  cargo test -p sql-orm --test stage20_navigation_runtime -- --nocapture --test-threads=1
```

The runtime test creates tables named:

- `dbo.sql_orm_nav_users`
- `dbo.sql_orm_nav_profiles`
- `dbo.sql_orm_nav_posts`

Without `KEEP_TEST_TABLES=1`, the test drops those tables after it finishes.

## Limits

- `include::<T>(...)` loads one `belongs_to` or `has_one` navigation.
- `include_many::<T>(...)` loads one `has_many` navigation.
- `include_many(...)` uses join loading today and rejects pagination.
- `split_query()` is selectable but not executable yet.
- explicit loading supports `has_many` only.
- explicit loading currently requires simple root primary keys.
- lazy wrappers never query by themselves.
- raw SQL does not attach navigation wrappers.
- DTO projections do not materialize navigation wrappers.
- graph tracking and relationship persistence are not stable.
