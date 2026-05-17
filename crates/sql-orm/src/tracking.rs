//! Experimental change tracking surface.
//!
//! Stability audit status, 2026-04-30: this is the only root-crate public
//! surface still explicitly marked experimental. It remains implemented but
//! not stable until the remaining identity-map, runtime and documentation
//! hardening work is complete. The current slice has validated identity
//! registration, registry-owned pending snapshots after wrapper drop, explicit
//! state APIs, no-op change detection, operation ordering, transaction
//! behavior for direct connections, policy integration and public compile-time
//! coverage.
//!
//! This module intentionally defines only the minimal public contracts for the
//! future tracking pipeline. In this stage it does not:
//! - replace the explicit `DbSet`/`ActiveRecord` APIs
//! - infer inserts, updates or deletes globally outside of `Tracked<T>`
//! - keep dropped unchanged wrappers in the unit of work
//! - support composite primary keys through `save_changes()`; that limit is
//!   now an explicit first-stable-cut scope rather than an implicit behavior
//!
//! Current experimental entry points:
//! - `DbSet::find_tracked(id)` for existing entities with single-column PK
//! - `DbSet::add_tracked(entity)` for new entities pending insertion
//! - `DbSet::remove_tracked(&mut tracked)` for explicit tracked deletion
//! - `Tracked::mark_modified()`, `Tracked::mark_deleted()`,
//!   `Tracked::mark_unchanged()` and `Tracked::detach()` for explicit state
//!   transitions on a wrapper
//! - `DbContext::save_changes()` for explicit persistence of registry entries
//!
//! Observable limits in the current stage:
//! - dropped `Added`, `Modified` and `Deleted` wrappers still participate in
//!   `save_changes()` through registry-owned snapshots
//! - mutable access marks `Unchanged` entities as `Modified` immediately
//! - loaded entities are registered with a deterministic identity made from
//!   entity type, schema, table and single-column primary key value
//! - reloading a detached loaded identity reattaches to the registry-owned
//!   snapshot; loading the same identity while another wrapper is still
//!   attached returns `OrmError` instead of keeping silent duplicates. This is
//!   the first stable-cut public policy: one live `Tracked<T>` handle per
//!   persisted identity per context.
//! - added entities use temporary local identities until a successful insert
//!   returns their persisted primary key
//! - explicit detach removes an entry from the registry without touching the
//!   database
//! - clearing the tracker removes every current registry entry
//! - dropping an unchanged wrapper is still equivalent to detach in this
//!   experimental slice
//! - removing a tracked `Added` entity cancels the pending insert locally
//! - successful tracked deletes unregister the wrapper from the internal registry
//! - rowversion conflicts are still surfaced as `OrmError::ConcurrencyConflict`
//! - composite-primary-key entities fail with a stable `OrmError` when loaded
//!   through `find_tracked(...)` or persisted through `save_changes()`; the
//!   first stable cut intentionally keeps tracking persistence scoped to
//!   single-column primary keys
//! - `tracked.save(&db).await` and `tracked.delete(&db).await` have explicit
//!   wrapper semantics, so they do not dereference to Active Record and leave
//!   stale tracker state behind
//! - navigation includes and explicit navigation loads attach values to the
//!   root entity only; related entities are not automatically registered in the
//!   experimental tracker and relationship changes are not persisted as graph
//!   updates
//! - future relationship persistence is intentionally deferred until graph
//!   update semantics can define dependent insert/delete behavior, foreign-key
//!   updates, direct many-to-many exclusions and conflict handling without
//!   bypassing the existing `DbSet` persistence paths

use crate::EntityPersist;
use core::ops::{Deref, DerefMut};
use sql_orm_core::{Entity, EntityMetadata, OrmError, SqlValue};
use std::any::{Any, TypeId};
use std::fmt;
use std::marker::PhantomData;
use std::sync::{Arc, Mutex};

/// Lifecycle state for an experimentally tracked entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EntityState {
    /// Entity was loaded and has not requested mutable access.
    Unchanged,
    /// Entity was added locally and should be inserted by `save_changes()`.
    Added,
    /// Entity was loaded and then mutably accessed.
    Modified,
    /// Entity was explicitly marked for deletion.
    Deleted,
}

/// Snapshot-based wrapper for entities tracked experimentally.
///
/// `Tracked<T>` keeps the original snapshot together with the current value so
/// later stages can compare and persist changes without relying on runtime
/// proxies or reflection. Registry-owned snapshots keep pending `Added`,
/// `Modified` and `Deleted` work alive after a wrapper is dropped or consumed.
/// Cloning a wrapper copies its visible state and snapshots, but the clone is
/// detached from the registry.
/// Calling [`Tracked::detach`] repeatedly is a no-op after the first detach and
/// does not reset the visible wrapper state.
///
/// State can be inspected with [`Tracked::state`] and changed explicitly with
/// [`Tracked::mark_modified`], [`Tracked::mark_deleted`],
/// [`Tracked::mark_unchanged`] and [`Tracked::detach`]. Immediate
/// persistence through [`Tracked::save`] and [`Tracked::delete`] delegates to
/// the same `DbSet`/Active Record pipelines used by ordinary CRUD.
pub struct Tracked<T> {
    inner: Box<TrackedInner<T>>,
    registration_id: Option<usize>,
    tracking_registry: Option<TrackingRegistryHandle>,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrackedEntityRegistration {
    pub entry_id: usize,
    pub entity_rust_name: &'static str,
    pub state: EntityState,
}

#[doc(hidden)]
#[derive(Debug, Default)]
pub struct TrackingRegistry {
    state: Mutex<TrackingRegistryState>,
}

#[doc(hidden)]
pub type TrackingRegistryHandle = Arc<TrackingRegistry>;

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SaveChangesOperationPlan {
    added_order: Vec<usize>,
    modified_order: Vec<usize>,
    deleted_order: Vec<usize>,
}

struct TrackedInner<T> {
    original: T,
    current: T,
    state: EntityState,
}

#[derive(Debug, Default)]
struct TrackingRegistryState {
    next_registration_id: usize,
    next_temporary_identity: u64,
    entries: Vec<TrackingRegistration>,
}

struct TrackingRegistration {
    registration_id: usize,
    identity: TrackedIdentity,
    entity_type_id: TypeId,
    entity_rust_name: &'static str,
    inner_address: usize,
    wrapper_attached: bool,
    state: EntityState,
    snapshots: Box<dyn Any + Send + Sync>,
    sync_current_from_wrapper: unsafe fn(&mut Box<dyn Any + Send + Sync>, usize),
}

impl fmt::Debug for TrackingRegistration {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TrackingRegistration")
            .field("registration_id", &self.registration_id)
            .field("identity", &self.identity)
            .field("entity_type_id", &self.entity_type_id)
            .field("entity_rust_name", &self.entity_rust_name)
            .field("inner_address", &self.inner_address)
            .field("wrapper_attached", &self.wrapper_attached)
            .field("state", &self.state)
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Clone, PartialEq)]
struct TrackedIdentity {
    entity_type_id: TypeId,
    entity_rust_name: &'static str,
    schema: &'static str,
    table: &'static str,
    primary_key: TrackedPrimaryKeyIdentity,
}

#[derive(Debug, Clone, PartialEq)]
enum TrackedPrimaryKeyIdentity {
    Simple(SqlValue),
    Temporary(u64),
}

#[derive(Clone)]
#[allow(dead_code)]
struct TrackingSnapshots<E> {
    original: E,
    current: E,
}

#[derive(Clone)]
pub(crate) struct RegisteredTracked<E> {
    registration_id: usize,
    inner_address: usize,
    tracking_registry: TrackingRegistryHandle,
    _entity: PhantomData<fn() -> E>,
}

impl<T: Clone> Tracked<T> {
    /// Creates a tracked value loaded from persistence.
    pub fn from_loaded(entity: T) -> Self {
        Self {
            inner: Box::new(TrackedInner {
                original: entity.clone(),
                current: entity,
                state: EntityState::Unchanged,
            }),
            registration_id: None,
            tracking_registry: None,
        }
    }

    /// Creates a tracked value that represents a new entity pending insertion.
    pub fn from_added(entity: T) -> Self {
        Self {
            inner: Box::new(TrackedInner {
                original: entity.clone(),
                current: entity,
                state: EntityState::Added,
            }),
            registration_id: None,
            tracking_registry: None,
        }
    }
}

impl<T> Tracked<T> {
    /// Returns the original snapshot captured when tracking started.
    pub fn original(&self) -> &T {
        &self.inner.original
    }

    /// Returns the current in-memory value.
    pub fn current(&self) -> &T {
        &self.inner.current
    }

    /// Returns the current tracking state.
    pub const fn state(&self) -> EntityState {
        self.inner.state
    }

    /// Explicitly marks this tracked value as `Modified`.
    ///
    /// `Added` values remain `Added` because they still need an insert, and
    /// `Deleted` values remain `Deleted` because deletion wins over pending
    /// modifications until the caller explicitly marks the value unchanged.
    pub fn mark_modified(&mut self) {
        self.mark_modified_if_unchanged();
    }

    /// Explicitly marks this tracked value as `Deleted`.
    ///
    /// This is a state transition only; it does not execute SQL. Calling it
    /// for an `Added` wrapper cancels the pending local insert by detaching the
    /// wrapper from the tracker.
    pub fn mark_deleted(&mut self) {
        let was_added = self.inner.state == EntityState::Added;
        self.set_state(EntityState::Deleted);
        if was_added {
            self.detach_registry();
        }
    }

    /// Explicitly accepts the current in-memory value as unchanged.
    ///
    /// The current value becomes the new original snapshot and later
    /// `save_changes()` calls ignore this wrapper until it is marked or
    /// mutably accessed again.
    pub fn mark_unchanged(&mut self)
    where
        T: Clone + Send + Sync + 'static,
    {
        self.inner.original = self.inner.current.clone();
        if let (Some(registration_id), Some(registry)) =
            (self.registration_id, self.tracking_registry.as_ref())
        {
            registry.set_snapshots(
                registration_id,
                self.inner.original.clone(),
                self.inner.current.clone(),
            );
        }
        self.set_state(EntityState::Unchanged);
    }

    /// Detaches this wrapper from its context tracker without executing SQL.
    ///
    /// Detach removes the registration from the current context unit of work
    /// and leaves the visible wrapper state unchanged.
    pub fn detach(&mut self) {
        self.detach_registry();
    }

    /// Returns mutable access to the current value and marks the entity as
    /// modified when it was previously loaded as unchanged.
    pub fn current_mut(&mut self) -> &mut T {
        self.mark_modified_if_unchanged();
        &mut self.inner.current
    }

    pub(crate) fn current_mut_without_state_change(&mut self) -> &mut T {
        &mut self.inner.current
    }

    fn mark_modified_if_unchanged(&mut self) {
        if self.inner.state == EntityState::Unchanged {
            self.set_state(EntityState::Modified);
        }
    }

    fn set_state(&mut self, state: EntityState) {
        self.inner.state = state;
        if let (Some(registration_id), Some(registry)) =
            (self.registration_id, self.tracking_registry.as_ref())
        {
            registry.set_state(registration_id, state);
        }
    }

    pub(crate) fn detach_registry(&mut self) {
        if let (Some(registration_id), Some(registry)) =
            (self.registration_id.take(), self.tracking_registry.take())
        {
            registry.unregister(registration_id);
        }
    }

    /// Persists this tracked entity immediately through the Active Record
    /// pipeline and synchronizes the tracking snapshot after success.
    ///
    /// This method exists so `tracked.save(&db).await` has explicit tracking
    /// semantics instead of dereferencing to `T::save(&db)` and leaving the
    /// tracker with a stale original snapshot. `Unchanged` wrappers are a
    /// no-op, `Added` and `Modified` wrappers use the same persistence path as
    /// Active Record, and `Deleted` wrappers return an error.
    pub fn save<C>(
        &mut self,
        db: &C,
    ) -> impl core::future::Future<Output = Result<(), OrmError>> + Send
    where
        C: crate::DbContextEntitySet<T> + Sync,
        T: crate::ActiveRecord
            + crate::AuditEntity
            + crate::EntityPersist
            + crate::EntityPrimaryKey
            + crate::SoftDeleteEntity
            + crate::TenantScopedEntity
            + Clone
            + sql_orm_core::FromRow
            + Send,
    {
        async move {
            match self.inner.state {
                EntityState::Unchanged => Ok(()),
                EntityState::Deleted => Err(OrmError::new(
                    "tracked deleted entities cannot be saved; detach them or persist deletion",
                )),
                EntityState::Added | EntityState::Modified => {
                    crate::ActiveRecord::save(&mut self.inner.current, db).await?;
                    self.inner.original = self.inner.current.clone();
                    self.set_state(EntityState::Unchanged);

                    if let (Some(registration_id), Some(registry)) =
                        (self.registration_id, self.tracking_registry.as_ref())
                    {
                        let key =
                            <T as crate::EntityPrimaryKey>::primary_key_value(&self.inner.current)?;
                        registry.update_persisted_identity::<T>(registration_id, key)?;
                    }

                    Ok(())
                }
            }
        }
    }

    /// Deletes this tracked entity immediately through the Active Record
    /// pipeline and removes it from the context tracker after success.
    ///
    /// Calling `tracked.delete(&db).await` on an `Added` wrapper cancels the
    /// local insert without touching the database. Persisted wrappers delegate
    /// to Active Record delete and detach after the row is affected, so a later
    /// `save_changes()` will not issue a second delete for the same wrapper.
    pub fn delete<C>(
        &mut self,
        db: &C,
    ) -> impl core::future::Future<Output = Result<bool, OrmError>> + Send
    where
        C: crate::DbContextEntitySet<T> + Sync,
        T: crate::ActiveRecord
            + crate::EntityPersist
            + crate::EntityPrimaryKey
            + crate::SoftDeleteEntity
            + crate::TenantScopedEntity
            + Clone
            + sql_orm_core::FromRow
            + Send,
    {
        async move {
            match self.inner.state {
                EntityState::Added => {
                    self.set_state(EntityState::Deleted);
                    self.detach_registry();
                    Ok(false)
                }
                EntityState::Deleted => Ok(false),
                EntityState::Unchanged | EntityState::Modified => {
                    let deleted = crate::ActiveRecord::delete(&self.inner.current, db).await?;
                    if deleted {
                        self.set_state(EntityState::Deleted);
                        self.detach_registry();
                    }
                    Ok(deleted)
                }
            }
        }
    }
}

impl<T: Clone> Tracked<T> {
    /// Consumes the tracked wrapper and returns the current entity value.
    pub fn into_current(self) -> T {
        self.current().clone()
    }
}

impl<T: Entity + Clone> Tracked<T> {
    pub(crate) fn attach_registry_loaded(
        &mut self,
        registry: TrackingRegistryHandle,
        key: SqlValue,
    ) -> Result<(), OrmError> {
        let registration_id = registry.register_or_attach_loaded(self, key)?;
        self.registration_id = Some(registration_id);
        self.tracking_registry = Some(registry);
        Ok(())
    }

    pub(crate) fn attach_registry_added(&mut self, registry: TrackingRegistryHandle) {
        let registration_id = registry.register_added(self);
        self.registration_id = Some(registration_id);
        self.tracking_registry = Some(registry);
    }

    #[cfg(test)]
    pub(crate) fn attach_registry(&mut self, registry: TrackingRegistryHandle) {
        self.attach_registry_added(registry);
    }
}

impl<T> Deref for Tracked<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        self.current()
    }
}

impl<T> DerefMut for Tracked<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.current_mut()
    }
}

impl TrackingRegistry {
    pub(crate) fn register_or_attach_loaded<E: Entity + Clone>(
        &self,
        tracked: &mut Tracked<E>,
        key: SqlValue,
    ) -> Result<usize, OrmError> {
        let identity =
            TrackedIdentity::for_entity::<E>(TrackedPrimaryKeyIdentity::Simple(key.clone()));
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");

        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.identity == identity)
        {
            if entry.wrapper_attached {
                return Err(duplicate_live_identity_error::<E>(&key));
            }

            let Some(snapshots) = entry.snapshots.downcast_ref::<TrackingSnapshots<E>>() else {
                return Err(OrmError::new(format!(
                    "tracked entity `{}` has incompatible registry snapshots",
                    E::metadata().rust_name,
                )));
            };

            tracked.inner.original = snapshots.original.clone();
            tracked.inner.current = snapshots.current.clone();
            tracked.inner.state = entry.state;
            entry.inner_address = tracked.inner.as_ref() as *const TrackedInner<E> as usize;
            entry.wrapper_attached = true;
            return Ok(entry.registration_id);
        }

        Ok(state.push_registration(tracked, identity))
    }

    pub(crate) fn register_added<E: Entity + Clone>(&self, tracked: &Tracked<E>) -> usize {
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");
        let temporary_identity = state.next_temporary_identity;
        state.next_temporary_identity += 1;
        let identity = TrackedIdentity::for_entity::<E>(TrackedPrimaryKeyIdentity::Temporary(
            temporary_identity,
        ));
        state.push_registration(tracked, identity)
    }

    pub(crate) fn unregister(&self, registration_id: usize) {
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");
        state
            .entries
            .retain(|entry| entry.registration_id != registration_id);
    }

    pub(crate) fn set_state(&self, registration_id: usize, tracked_state: EntityState) {
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");
        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.registration_id == registration_id)
        {
            entry.state = tracked_state;
        }
    }

    pub(crate) fn detach_wrapper(&self, registration_id: usize) {
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");
        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.registration_id == registration_id)
        {
            if entry.wrapper_attached {
                unsafe {
                    (entry.sync_current_from_wrapper)(&mut entry.snapshots, entry.inner_address);
                }
            }
            entry.wrapper_attached = false;
            entry.inner_address = 0;
        }
    }

    pub(crate) fn set_snapshots<E: Clone + Send + Sync + 'static>(
        &self,
        registration_id: usize,
        original: E,
        current: E,
    ) {
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");
        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.registration_id == registration_id)
        {
            entry.snapshots = Box::new(TrackingSnapshots::<E> { original, current });
        }
    }

    fn sync_current_snapshot_from_wrapper(&self, registration_id: usize) {
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");
        if let Some(entry) = state
            .entries
            .iter_mut()
            .find(|entry| entry.registration_id == registration_id)
            .filter(|entry| entry.wrapper_attached)
        {
            unsafe {
                (entry.sync_current_from_wrapper)(&mut entry.snapshots, entry.inner_address);
            }
        }
    }

    fn is_wrapper_attached(&self, registration_id: usize) -> bool {
        self.state
            .lock()
            .expect("tracking registry mutex poisoned")
            .entries
            .iter()
            .find(|entry| entry.registration_id == registration_id)
            .is_some_and(|entry| entry.wrapper_attached)
    }

    fn state_of(&self, registration_id: usize) -> Option<EntityState> {
        self.state
            .lock()
            .expect("tracking registry mutex poisoned")
            .entries
            .iter()
            .find(|entry| entry.registration_id == registration_id)
            .map(|entry| entry.state)
    }

    #[allow(dead_code)]
    fn original_snapshot_of<E: Clone + Send + Sync + 'static>(
        &self,
        registration_id: usize,
    ) -> Option<E> {
        self.state
            .lock()
            .expect("tracking registry mutex poisoned")
            .entries
            .iter()
            .find(|entry| entry.registration_id == registration_id)
            .and_then(|entry| entry.snapshots.downcast_ref::<TrackingSnapshots<E>>())
            .map(|snapshots| snapshots.original.clone())
    }

    #[allow(dead_code)]
    fn current_snapshot_of<E: Clone + Send + Sync + 'static>(
        &self,
        registration_id: usize,
    ) -> Option<E> {
        self.state
            .lock()
            .expect("tracking registry mutex poisoned")
            .entries
            .iter()
            .find(|entry| entry.registration_id == registration_id)
            .and_then(|entry| entry.snapshots.downcast_ref::<TrackingSnapshots<E>>())
            .map(|snapshots| snapshots.current.clone())
    }

    fn snapshot_pair_of<E: Clone + Send + Sync + 'static>(
        &self,
        registration_id: usize,
    ) -> Option<(E, E)> {
        self.state
            .lock()
            .expect("tracking registry mutex poisoned")
            .entries
            .iter()
            .find(|entry| entry.registration_id == registration_id)
            .and_then(|entry| entry.snapshots.downcast_ref::<TrackingSnapshots<E>>())
            .map(|snapshots| (snapshots.original.clone(), snapshots.current.clone()))
    }

    pub(crate) fn current_snapshot_for_key<E: Entity + Clone + Send + Sync + 'static>(
        &self,
        key: SqlValue,
    ) -> Option<E> {
        let identity = TrackedIdentity::for_entity::<E>(TrackedPrimaryKeyIdentity::Simple(key));
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");
        let entry = state
            .entries
            .iter_mut()
            .find(|entry| entry.identity == identity)?;

        if entry.wrapper_attached {
            unsafe {
                (entry.sync_current_from_wrapper)(&mut entry.snapshots, entry.inner_address);
            }
        }

        entry
            .snapshots
            .downcast_ref::<TrackingSnapshots<E>>()
            .map(|snapshots| snapshots.current.clone())
    }

    pub fn clear(&self) {
        self.state
            .lock()
            .expect("tracking registry mutex poisoned")
            .entries
            .clear();
    }

    pub(crate) fn tracked_for<E: Entity>(self: &Arc<Self>) -> Vec<RegisteredTracked<E>> {
        let state = self.state.lock().expect("tracking registry mutex poisoned");

        state
            .entries
            .iter()
            .filter(|entry| entry.entity_type_id == TypeId::of::<E>())
            .map(|entry| RegisteredTracked::<E> {
                registration_id: entry.registration_id,
                inner_address: entry.inner_address,
                tracking_registry: Arc::clone(self),
                _entity: PhantomData,
            })
            .collect()
    }

    pub(crate) fn update_persisted_identity<E: Entity>(
        &self,
        registration_id: usize,
        key: SqlValue,
    ) -> Result<(), OrmError> {
        let identity =
            TrackedIdentity::for_entity::<E>(TrackedPrimaryKeyIdentity::Simple(key.clone()));
        let mut state = self.state.lock().expect("tracking registry mutex poisoned");

        if state
            .entries
            .iter()
            .any(|entry| entry.registration_id != registration_id && entry.identity == identity)
        {
            return Err(OrmError::new(format!(
                "entity `{}` with primary key value `{:?}` is already tracked in this context",
                E::metadata().rust_name,
                key
            )));
        }

        let entry = state
            .entries
            .iter_mut()
            .find(|entry| entry.registration_id == registration_id)
            .ok_or_else(|| OrmError::new("tracked entity registration was not found"))?;
        entry.identity = identity;
        Ok(())
    }

    pub fn entry_count(&self) -> usize {
        self.state
            .lock()
            .expect("tracking registry mutex poisoned")
            .entries
            .len()
    }

    pub fn registrations(&self) -> Vec<TrackedEntityRegistration> {
        self.state
            .lock()
            .expect("tracking registry mutex poisoned")
            .entries
            .iter()
            .map(|entry| TrackedEntityRegistration {
                entry_id: entry.registration_id,
                entity_rust_name: entry.entity_rust_name,
                state: entry.state,
            })
            .collect()
    }
}

#[doc(hidden)]
pub fn save_changes_operation_plan(
    entities: &[&'static EntityMetadata],
) -> Result<SaveChangesOperationPlan, OrmError> {
    let insert_order = topological_entity_order(entities)?;
    let mut delete_order = insert_order.clone();
    delete_order.reverse();

    Ok(SaveChangesOperationPlan {
        added_order: insert_order.clone(),
        modified_order: insert_order,
        deleted_order: delete_order,
    })
}

impl SaveChangesOperationPlan {
    pub fn added_order(&self) -> &[usize] {
        &self.added_order
    }

    pub fn modified_order(&self) -> &[usize] {
        &self.modified_order
    }

    pub fn deleted_order(&self) -> &[usize] {
        &self.deleted_order
    }
}

fn topological_entity_order(entities: &[&'static EntityMetadata]) -> Result<Vec<usize>, OrmError> {
    let mut outgoing_edges = vec![Vec::<usize>::new(); entities.len()];
    let mut incoming_edge_count = vec![0usize; entities.len()];

    for (child_index, child) in entities.iter().enumerate() {
        for foreign_key in child.foreign_keys {
            if foreign_key.columns.len() != 1 || foreign_key.referenced_columns.len() != 1 {
                continue;
            }

            let Some(parent_index) = entities.iter().position(|candidate| {
                candidate.schema == foreign_key.referenced_schema
                    && candidate.table == foreign_key.referenced_table
            }) else {
                continue;
            };

            if parent_index == child_index || outgoing_edges[parent_index].contains(&child_index) {
                continue;
            }

            outgoing_edges[parent_index].push(child_index);
            incoming_edge_count[child_index] += 1;
        }
    }

    let mut order = Vec::with_capacity(entities.len());
    let mut ready: Vec<usize> = incoming_edge_count
        .iter()
        .enumerate()
        .filter_map(|(index, count)| (*count == 0).then_some(index))
        .collect();

    while !ready.is_empty() {
        ready.sort_unstable();
        let entity_index = ready.remove(0);
        order.push(entity_index);

        for child_index in &outgoing_edges[entity_index] {
            incoming_edge_count[*child_index] -= 1;
            if incoming_edge_count[*child_index] == 0 {
                ready.push(*child_index);
            }
        }
    }

    if order.len() != entities.len() {
        return Err(OrmError::new(
            "save_changes cannot determine a deterministic order for tracked operations because the context contains a foreign-key cycle",
        ));
    }

    Ok(order)
}

impl TrackingRegistryState {
    fn push_registration<E: Entity + Clone>(
        &mut self,
        tracked: &Tracked<E>,
        identity: TrackedIdentity,
    ) -> usize {
        let registration_id = self.next_registration_id;
        self.next_registration_id += 1;
        self.entries.push(TrackingRegistration {
            registration_id,
            identity,
            entity_type_id: TypeId::of::<E>(),
            entity_rust_name: E::metadata().rust_name,
            inner_address: tracked.inner.as_ref() as *const TrackedInner<E> as usize,
            wrapper_attached: true,
            state: tracked.inner.state,
            snapshots: Box::new(TrackingSnapshots::<E> {
                original: tracked.inner.original.clone(),
                current: tracked.inner.current.clone(),
            }),
            sync_current_from_wrapper: sync_current_snapshot_from_wrapper::<E>,
        });
        registration_id
    }
}

impl TrackedIdentity {
    fn for_entity<E: Entity>(primary_key: TrackedPrimaryKeyIdentity) -> Self {
        let metadata = E::metadata();
        Self {
            entity_type_id: TypeId::of::<E>(),
            entity_rust_name: metadata.rust_name,
            schema: metadata.schema,
            table: metadata.table,
            primary_key,
        }
    }
}

impl<E: Clone + Send + Sync + 'static> RegisteredTracked<E> {
    pub(crate) fn registration_id(&self) -> usize {
        self.registration_id
    }

    pub(crate) fn state(&self) -> EntityState {
        self.tracking_registry
            .state_of(self.registration_id)
            .unwrap_or_else(|| unsafe { (&*(self.inner_address as *const TrackedInner<E>)).state })
    }

    pub(crate) fn current_clone(&self) -> E {
        self.sync_current_snapshot_from_wrapper();
        self.tracking_registry
            .current_snapshot_of::<E>(self.registration_id)
            .unwrap_or_else(|| unsafe {
                (&*(self.inner_address as *const TrackedInner<E>))
                    .current
                    .clone()
            })
    }

    fn sync_current_snapshot_from_wrapper(&self) {
        self.tracking_registry
            .sync_current_snapshot_from_wrapper(self.registration_id);
    }

    pub(crate) fn accept_current(&self) {
        let current = self.current_clone();
        if self
            .tracking_registry
            .is_wrapper_attached(self.registration_id)
        {
            unsafe {
                let inner = self.inner_address as *mut TrackedInner<E>;
                (*inner).original = current.clone();
                (*inner).state = EntityState::Unchanged;
            }
        }
        self.tracking_registry
            .set_snapshots(self.registration_id, current.clone(), current);
        self.tracking_registry
            .set_state(self.registration_id, EntityState::Unchanged);
    }

    pub(crate) fn sync_persisted(&self, persisted: E) {
        let snapshot = persisted.clone();
        if self
            .tracking_registry
            .is_wrapper_attached(self.registration_id)
        {
            unsafe {
                let inner = self.inner_address as *mut TrackedInner<E>;
                (*inner).original = persisted.clone();
                (*inner).current = persisted;
                (*inner).state = EntityState::Unchanged;
            }
        }
        self.tracking_registry
            .set_snapshots(self.registration_id, snapshot.clone(), snapshot);
        self.tracking_registry
            .set_state(self.registration_id, EntityState::Unchanged);
    }
}

impl<E: EntityPersist + Clone + Send + Sync + 'static> RegisteredTracked<E> {
    pub(crate) fn has_persisted_changes(&self) -> bool {
        self.sync_current_snapshot_from_wrapper();
        self.tracking_registry
            .snapshot_pair_of::<E>(self.registration_id)
            .map(|(original, current)| E::has_persisted_changes(&original, &current))
            .unwrap_or_else(|| unsafe {
                let inner = &*(self.inner_address as *const TrackedInner<E>);
                E::has_persisted_changes(&inner.original, &inner.current)
            })
    }
}

fn duplicate_live_identity_error<E: Entity>(key: &SqlValue) -> OrmError {
    OrmError::new(format!(
        "entity `{}` with primary key value `{:?}` already has a live tracked handle in this context; detach or drop the existing handle before loading it again",
        E::metadata().rust_name,
        key
    ))
}

unsafe fn sync_current_snapshot_from_wrapper<E: Clone + Send + Sync + 'static>(
    snapshots: &mut Box<dyn Any + Send + Sync>,
    inner_address: usize,
) {
    let Some(snapshots) = snapshots.downcast_mut::<TrackingSnapshots<E>>() else {
        return;
    };
    let inner = unsafe { &*(inner_address as *const TrackedInner<E>) };
    snapshots.current = inner.current.clone();
}

impl<T: Clone> Clone for Tracked<T> {
    fn clone(&self) -> Self {
        Self {
            inner: Box::new(TrackedInner {
                original: self.original().clone(),
                current: self.current().clone(),
                state: self.state(),
            }),
            registration_id: None,
            tracking_registry: None,
        }
    }
}

impl<T: core::fmt::Debug> core::fmt::Debug for Tracked<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Tracked")
            .field("original", self.original())
            .field("current", self.current())
            .field("state", &self.state())
            .finish()
    }
}

impl<T: PartialEq> PartialEq for Tracked<T> {
    fn eq(&self, other: &Self) -> bool {
        self.original() == other.original()
            && self.current() == other.current()
            && self.state() == other.state()
    }
}

impl<T: Eq> Eq for Tracked<T> {}

impl<T> Drop for Tracked<T> {
    fn drop(&mut self) {
        if let (Some(registration_id), Some(registry)) =
            (self.registration_id.take(), self.tracking_registry.take())
        {
            if matches!(
                self.inner.state,
                EntityState::Added | EntityState::Modified | EntityState::Deleted
            ) {
                registry.detach_wrapper(registration_id);
            } else {
                registry.unregister(registration_id);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        EntityState, Tracked, TrackedEntityRegistration, TrackingRegistry,
        save_changes_operation_plan,
    };
    use crate::{EntityPersist, EntityPersistMode};
    use sql_orm_core::{
        ColumnValue, Entity, EntityMetadata, ForeignKeyMetadata, OrmError, PrimaryKeyMetadata,
        ReferentialAction, SqlValue,
    };
    use std::sync::Arc;

    #[derive(Clone)]
    struct DummyEntity;

    #[derive(Clone)]
    struct DummyEntityAlias;

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct SnapshotEntity {
        name: String,
    }

    #[derive(Clone, Debug, PartialEq, Eq)]
    struct SnapshotEntityAlias {
        name: String,
    }

    static DUMMY_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "DummyEntity",
        schema: "dbo",
        table: "dummy_entities",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &[],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static ORDER_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "Order",
        schema: "sales",
        table: "orders",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static ORDER_ITEM_FOREIGN_KEYS: [ForeignKeyMetadata; 1] = [ForeignKeyMetadata::new(
        "fk_order_items_orders",
        &["order_id"],
        "sales",
        "orders",
        &["id"],
        ReferentialAction::NoAction,
        ReferentialAction::NoAction,
    )];

    static ORDER_ITEM_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "OrderItem",
        schema: "sales",
        table: "order_items",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &ORDER_ITEM_FOREIGN_KEYS,
        navigations: &[],
    };

    static CATEGORY_FOREIGN_KEYS: [ForeignKeyMetadata; 1] = [ForeignKeyMetadata::new(
        "fk_categories_parent",
        &["parent_id"],
        "catalog",
        "categories",
        &["id"],
        ReferentialAction::NoAction,
        ReferentialAction::NoAction,
    )];

    static CATEGORY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "Category",
        schema: "catalog",
        table: "categories",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &CATEGORY_FOREIGN_KEYS,
        navigations: &[],
    };

    static CYCLE_A_FOREIGN_KEYS: [ForeignKeyMetadata; 1] = [ForeignKeyMetadata::new(
        "fk_cycle_a_cycle_b",
        &["cycle_b_id"],
        "dbo",
        "cycle_b",
        &["id"],
        ReferentialAction::NoAction,
        ReferentialAction::NoAction,
    )];

    static CYCLE_B_FOREIGN_KEYS: [ForeignKeyMetadata; 1] = [ForeignKeyMetadata::new(
        "fk_cycle_b_cycle_a",
        &["cycle_a_id"],
        "dbo",
        "cycle_a",
        &["id"],
        ReferentialAction::NoAction,
        ReferentialAction::NoAction,
    )];

    static CYCLE_A_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "CycleA",
        schema: "dbo",
        table: "cycle_a",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &CYCLE_A_FOREIGN_KEYS,
        navigations: &[],
    };

    static CYCLE_B_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "CycleB",
        schema: "dbo",
        table: "cycle_b",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &CYCLE_B_FOREIGN_KEYS,
        navigations: &[],
    };

    impl Entity for DummyEntity {
        fn metadata() -> &'static EntityMetadata {
            &DUMMY_ENTITY_METADATA
        }
    }

    impl Entity for DummyEntityAlias {
        fn metadata() -> &'static EntityMetadata {
            &DUMMY_ENTITY_METADATA
        }
    }

    impl Entity for SnapshotEntity {
        fn metadata() -> &'static EntityMetadata {
            &DUMMY_ENTITY_METADATA
        }
    }

    impl Entity for SnapshotEntityAlias {
        fn metadata() -> &'static EntityMetadata {
            &DUMMY_ENTITY_METADATA
        }
    }

    impl EntityPersist for SnapshotEntity {
        fn persist_mode(&self) -> Result<EntityPersistMode, OrmError> {
            Ok(EntityPersistMode::Update(SqlValue::I64(1)))
        }

        fn insert_values(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn update_changes(&self) -> Vec<ColumnValue> {
            vec![ColumnValue::new(
                "name",
                SqlValue::String(self.name.clone()),
            )]
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError> {
            Ok(None)
        }

        fn sync_persisted(&mut self, persisted: Self) {
            *self = persisted;
        }
    }

    #[test]
    fn tracked_loaded_value_keeps_original_and_current_snapshots() {
        let tracked = Tracked::from_loaded(String::from("Ana"));

        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert_eq!(tracked.original(), "Ana");
        assert_eq!(tracked.current(), "Ana");
    }

    #[test]
    fn tracked_added_value_starts_in_added_state() {
        let tracked = Tracked::from_added(String::from("Luis"));

        assert_eq!(tracked.state(), EntityState::Added);
        assert_eq!(tracked.original(), "Luis");
        assert_eq!(tracked.current(), "Luis");
    }

    #[test]
    fn tracked_can_release_current_value() {
        let tracked = Tracked::from_loaded(String::from("Maria"));

        assert_eq!(tracked.into_current(), "Maria");
    }

    #[test]
    fn into_current_consumes_registered_wrapper_and_unregisters_it() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);
        tracked.attach_registry(Arc::clone(&registry));

        assert_eq!(registry.entry_count(), 1);

        let _current = tracked.into_current();

        assert_eq!(registry.entry_count(), 0);
    }

    #[test]
    fn cloned_tracked_wrapper_is_detached_from_original_registry_entry() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut original = Tracked::from_loaded(DummyEntity);
        original.attach_registry(Arc::clone(&registry));
        original.mark_modified();

        let clone = original.clone();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(clone.state(), EntityState::Modified);

        drop(clone);

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Modified);
    }

    #[test]
    fn mutable_access_transitions_loaded_entity_to_modified() {
        let mut tracked = Tracked::from_loaded(String::from("Ana"));

        tracked.push_str(" Maria");

        assert_eq!(tracked.state(), EntityState::Modified);
        assert_eq!(tracked.original(), "Ana");
        assert_eq!(tracked.current(), "Ana Maria");
    }

    #[test]
    fn current_mut_transitions_loaded_entity_to_modified() {
        let mut tracked = Tracked::from_loaded(String::from("Luis"));

        tracked.current_mut().push_str(" Alberto");

        assert_eq!(tracked.state(), EntityState::Modified);
        assert_eq!(tracked.original(), "Luis");
        assert_eq!(tracked.current(), "Luis Alberto");
    }

    #[test]
    fn explicit_mark_modified_transitions_unchanged_only() {
        let mut loaded = Tracked::from_loaded(String::from("Ana"));
        loaded.mark_modified();

        let mut added = Tracked::from_added(String::from("Luis"));
        added.mark_modified();

        let mut deleted = Tracked::from_loaded(String::from("Maria"));
        deleted.mark_deleted();
        deleted.mark_modified();

        assert_eq!(loaded.state(), EntityState::Modified);
        assert_eq!(added.state(), EntityState::Added);
        assert_eq!(deleted.state(), EntityState::Deleted);
    }

    #[test]
    fn explicit_mark_deleted_transitions_wrapper_to_deleted() {
        let mut tracked = Tracked::from_loaded(String::from("Ana"));

        tracked.mark_deleted();

        assert_eq!(tracked.state(), EntityState::Deleted);
    }

    #[test]
    fn explicit_mark_unchanged_accepts_current_snapshot() {
        let mut tracked = Tracked::from_loaded(String::from("Ana"));
        tracked.current_mut().push_str(" Maria");

        tracked.mark_unchanged();

        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert_eq!(tracked.original(), "Ana Maria");
        assert_eq!(tracked.current(), "Ana Maria");
    }

    #[test]
    fn explicit_mark_unchanged_restores_deleted_wrapper_with_current_snapshot() {
        let mut tracked = Tracked::from_loaded(String::from("Ana"));
        tracked.current_mut().push_str(" Maria");
        tracked.mark_deleted();

        tracked.mark_unchanged();

        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert_eq!(tracked.original(), "Ana Maria");
        assert_eq!(tracked.current(), "Ana Maria");
    }

    #[test]
    fn explicit_mark_unchanged_on_registered_wrapper_updates_registry_state() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);
        tracked.attach_registry(Arc::clone(&registry));
        tracked.mark_deleted();

        tracked.mark_unchanged();

        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Unchanged);
    }

    #[test]
    fn mark_deleted_transitions_any_registered_entity_to_deleted() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);
        tracked.attach_registry(Arc::clone(&registry));

        tracked.mark_deleted();

        assert_eq!(tracked.state(), EntityState::Deleted);
        assert_eq!(registry.registrations()[0].state, EntityState::Deleted);
    }

    #[test]
    fn mark_deleted_on_added_registered_entry_cancels_pending_insert() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_added(DummyEntity);
        tracked.attach_registry_added(Arc::clone(&registry));

        tracked.mark_deleted();

        assert_eq!(tracked.state(), EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);
    }

    #[test]
    fn mutable_access_keeps_added_state_for_new_entities() {
        let mut tracked = Tracked::from_added(String::from("Maria"));

        tracked.push_str(" Fernanda");

        assert_eq!(tracked.state(), EntityState::Added);
        assert_eq!(tracked.original(), "Maria");
        assert_eq!(tracked.current(), "Maria Fernanda");
    }

    #[test]
    fn tracking_registry_records_loaded_entities() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);

        tracked
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations(),
            vec![TrackedEntityRegistration {
                entry_id: 0,
                entity_rust_name: "DummyEntity",
                state: EntityState::Unchanged,
            }]
        );
    }

    #[test]
    fn tracking_registry_records_added_entities() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_added(DummyEntity);

        tracked.attach_registry(Arc::clone(&registry));

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations(),
            vec![TrackedEntityRegistration {
                entry_id: 0,
                entity_rust_name: "DummyEntity",
                state: EntityState::Added,
            }]
        );
    }

    #[test]
    fn tracking_registry_diagnostics_expose_stable_entry_ids() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_added(DummyEntity);
        let mut second = Tracked::from_added(DummyEntity);

        first.attach_registry_added(Arc::clone(&registry));
        second.attach_registry_added(Arc::clone(&registry));

        let registrations = registry.registrations();

        assert_eq!(registrations.len(), 2);
        assert_eq!(registrations[0].entry_id, 0);
        assert_eq!(registrations[1].entry_id, 1);
        assert_eq!(registrations[0].entity_rust_name, "DummyEntity");
        assert_eq!(registrations[1].entity_rust_name, "DummyEntity");
    }

    #[test]
    fn tracking_registry_diagnostic_entry_ids_are_not_reused_after_unregister() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_added(DummyEntity);
        let mut second = Tracked::from_added(DummyEntity);

        first.attach_registry_added(Arc::clone(&registry));
        let first_registration_id = first.registration_id.expect("registered first entity");
        registry.unregister(first_registration_id);
        second.attach_registry_added(Arc::clone(&registry));

        let registrations = registry.registrations();

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].entry_id, 1);
        assert_eq!(registrations[0].state, EntityState::Added);
    }

    #[test]
    fn tracking_registry_diagnostic_entry_ids_are_not_reused_after_clear() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_added(DummyEntity);
        let mut second = Tracked::from_added(DummyEntity);
        let mut third = Tracked::from_added(DummyEntity);

        first.attach_registry_added(Arc::clone(&registry));
        second.attach_registry_added(Arc::clone(&registry));
        registry.clear();
        third.attach_registry_added(Arc::clone(&registry));

        let registrations = registry.registrations();

        assert_eq!(registrations.len(), 1);
        assert_eq!(registrations[0].entry_id, 2);
        assert_eq!(registrations[0].state, EntityState::Added);
    }

    #[test]
    fn tracking_registry_owns_observable_state_for_registered_entries() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);
        tracked.attach_registry(Arc::clone(&registry));

        tracked.inner.state = EntityState::Deleted;

        assert_eq!(tracked.state(), EntityState::Deleted);
        assert_eq!(registry.registrations()[0].state, EntityState::Unchanged);
        assert_eq!(
            registry.tracked_for::<DummyEntity>()[0].state(),
            EntityState::Unchanged
        );

        tracked.mark_unchanged();

        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert_eq!(registry.registrations()[0].state, EntityState::Unchanged);
    }

    #[test]
    fn tracking_registry_owns_initial_snapshots_for_registered_entries() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(SnapshotEntity {
            name: "loaded".to_string(),
        });
        tracked.attach_registry(Arc::clone(&registry));
        let registration_id = tracked.registration_id.expect("registered");

        tracked.inner.original.name = "wrapper original changed".to_string();
        tracked.inner.current.name = "wrapper current changed".to_string();

        assert_eq!(
            registry
                .original_snapshot_of::<SnapshotEntity>(registration_id)
                .unwrap()
                .name,
            "loaded"
        );
        assert_eq!(
            registry
                .current_snapshot_of::<SnapshotEntity>(registration_id)
                .unwrap()
                .name,
            "loaded"
        );
    }

    #[test]
    fn mark_unchanged_syncs_registry_owned_snapshots() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(SnapshotEntity {
            name: "loaded".to_string(),
        });
        tracked.attach_registry(Arc::clone(&registry));
        let registration_id = tracked.registration_id.expect("registered");

        tracked.current_mut().name = "accepted".to_string();
        tracked.mark_unchanged();

        assert_eq!(
            registry
                .original_snapshot_of::<SnapshotEntity>(registration_id)
                .unwrap()
                .name,
            "accepted"
        );
        assert_eq!(
            registry
                .current_snapshot_of::<SnapshotEntity>(registration_id)
                .unwrap()
                .name,
            "accepted"
        );
    }

    #[test]
    fn registered_tracked_helpers_read_snapshots_from_registry() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(SnapshotEntity {
            name: "loaded".to_string(),
        });
        tracked.attach_registry(Arc::clone(&registry));
        let registration_id = tracked.registration_id.expect("registered");

        tracked.inner.original.name = "wrapper original changed".to_string();
        tracked.inner.current.name = "changed".to_string();

        let registered = registry.tracked_for::<SnapshotEntity>()[0].clone();

        assert!(registered.has_persisted_changes());
        assert_eq!(registered.current_clone().name, "changed");
        assert_eq!(
            registry
                .original_snapshot_of::<SnapshotEntity>(registration_id)
                .unwrap()
                .name,
            "loaded"
        );
        assert_eq!(
            registry
                .current_snapshot_of::<SnapshotEntity>(registration_id)
                .unwrap()
                .name,
            "changed"
        );
    }

    #[test]
    fn registered_tracked_sync_persisted_updates_detached_registry_owned_snapshots() {
        let registry = Arc::new(TrackingRegistry::default());

        {
            let mut tracked = Tracked::from_loaded(SnapshotEntity {
                name: "loaded".to_string(),
            });
            tracked
                .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
                .unwrap();
            tracked.current_mut().name = "changed before drop".to_string();
        }

        let registered = registry.tracked_for::<SnapshotEntity>()[0].clone();
        registered.sync_persisted(SnapshotEntity {
            name: "persisted value".to_string(),
        });

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Unchanged);
        assert!(!registered.has_persisted_changes());
        assert_eq!(registered.current_clone().name, "persisted value");

        let mut reattached = Tracked::from_loaded(SnapshotEntity {
            name: "stale database value".to_string(),
        });
        reattached
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        assert_eq!(reattached.state(), EntityState::Unchanged);
        assert_eq!(reattached.original().name, "persisted value");
        assert_eq!(reattached.current().name, "persisted value");
    }

    #[test]
    fn registered_tracked_accept_current_updates_detached_registry_owned_snapshots() {
        let registry = Arc::new(TrackingRegistry::default());

        {
            let mut tracked = Tracked::from_loaded(SnapshotEntity {
                name: "loaded".to_string(),
            });
            tracked
                .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
                .unwrap();
            tracked.current_mut().name = "accepted detached current".to_string();
        }

        let registered = registry.tracked_for::<SnapshotEntity>()[0].clone();
        assert!(registered.has_persisted_changes());

        registered.accept_current();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Unchanged);
        assert!(!registered.has_persisted_changes());
        assert_eq!(registered.current_clone().name, "accepted detached current");

        let mut reattached = Tracked::from_loaded(SnapshotEntity {
            name: "stale database value".to_string(),
        });
        reattached
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        assert_eq!(reattached.state(), EntityState::Unchanged);
        assert_eq!(reattached.original().name, "accepted detached current");
        assert_eq!(reattached.current().name, "accepted detached current");
    }

    #[test]
    fn dropping_added_wrapper_detaches_handle_without_removing_registry_entry() {
        let registry = Arc::new(TrackingRegistry::default());

        {
            let mut tracked = Tracked::from_added(SnapshotEntity {
                name: "new".to_string(),
            });
            tracked.attach_registry_added(Arc::clone(&registry));
            tracked.current_mut().name = "changed before drop".to_string();

            assert_eq!(registry.entry_count(), 1);
        }

        let registered = registry.tracked_for::<SnapshotEntity>()[0].clone();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Added);
        assert_eq!(registered.current_clone().name, "changed before drop");
    }

    #[test]
    fn dropping_modified_wrapper_detaches_handle_without_removing_registry_entry() {
        let registry = Arc::new(TrackingRegistry::default());

        {
            let mut tracked = Tracked::from_loaded(SnapshotEntity {
                name: "loaded".to_string(),
            });
            tracked.attach_registry(Arc::clone(&registry));
            tracked.current_mut().name = "changed before drop".to_string();

            assert_eq!(registry.entry_count(), 1);
            assert_eq!(registry.registrations()[0].state, EntityState::Modified);
        }

        let registered = registry.tracked_for::<SnapshotEntity>()[0].clone();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Modified);
        assert_eq!(registered.current_clone().name, "changed before drop");
        assert!(registered.has_persisted_changes());
    }

    #[test]
    fn dropping_deleted_wrapper_detaches_handle_without_removing_registry_entry() {
        let registry = Arc::new(TrackingRegistry::default());

        {
            let mut tracked = Tracked::from_loaded(SnapshotEntity {
                name: "loaded".to_string(),
            });
            tracked.attach_registry(Arc::clone(&registry));
            tracked.current_mut().name = "changed before delete".to_string();
            tracked.mark_deleted();

            assert_eq!(registry.entry_count(), 1);
            assert_eq!(registry.registrations()[0].state, EntityState::Deleted);
        }

        let registered = registry.tracked_for::<SnapshotEntity>()[0].clone();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Deleted);
        assert_eq!(registered.current_clone().name, "changed before delete");
    }

    #[test]
    fn loaded_identity_reattaches_detached_registry_entry_with_owned_snapshots() {
        let registry = Arc::new(TrackingRegistry::default());

        {
            let mut tracked = Tracked::from_loaded(SnapshotEntity {
                name: "loaded".to_string(),
            });
            tracked
                .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
                .unwrap();
            tracked.current_mut().name = "changed before drop".to_string();

            assert_eq!(tracked.state(), EntityState::Modified);
            assert_eq!(registry.entry_count(), 1);
        }

        let mut reattached = Tracked::from_loaded(SnapshotEntity {
            name: "stale database value".to_string(),
        });
        reattached
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(reattached.state(), EntityState::Modified);
        assert_eq!(reattached.original().name, "loaded");
        assert_eq!(reattached.current().name, "changed before drop");
        assert_eq!(registry.registrations()[0].state, EntityState::Modified);
        assert_eq!(
            registry.tracked_for::<SnapshotEntity>()[0]
                .current_clone()
                .name,
            "changed before drop"
        );
    }

    #[test]
    fn loaded_identity_reattach_rejects_incompatible_registry_snapshots() {
        let registry = Arc::new(TrackingRegistry::default());
        let registration_id;

        {
            let mut tracked = Tracked::from_loaded(SnapshotEntity {
                name: "loaded".to_string(),
            });
            tracked
                .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
                .unwrap();
            tracked.current_mut().name = "changed before drop".to_string();
            registration_id = tracked.registration_id.expect("registered");
        }

        registry.set_snapshots(
            registration_id,
            SnapshotEntityAlias {
                name: "wrong original type".to_string(),
            },
            SnapshotEntityAlias {
                name: "wrong current type".to_string(),
            },
        );

        let mut reattached = Tracked::from_loaded(SnapshotEntity {
            name: "fresh database value".to_string(),
        });
        let error = reattached
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap_err();

        assert_eq!(
            error.message(),
            "tracked entity `DummyEntity` has incompatible registry snapshots"
        );
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(reattached.state(), EntityState::Unchanged);
        assert_eq!(reattached.current().name, "fresh database value");
    }

    #[test]
    fn detached_loaded_identity_can_be_registered_again() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_loaded(SnapshotEntity {
            name: "first".to_string(),
        });
        first
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        first.detach();

        let mut second = Tracked::from_loaded(SnapshotEntity {
            name: "second".to_string(),
        });
        second
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        let registrations = registry.registrations();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registrations[0].entry_id, 1);
        assert_eq!(second.current().name, "second");
        assert_eq!(second.state(), EntityState::Unchanged);
        assert_eq!(
            registry
                .current_snapshot_for_key::<SnapshotEntity>(SqlValue::I64(7))
                .expect("newly registered identity should be available")
                .name,
            "second"
        );
    }

    #[test]
    fn cleared_loaded_identity_can_be_registered_again() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_loaded(SnapshotEntity {
            name: "first".to_string(),
        });
        first
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        registry.clear();

        let mut second = Tracked::from_loaded(SnapshotEntity {
            name: "second".to_string(),
        });
        second
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        let registrations = registry.registrations();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registrations[0].entry_id, 1);
        assert_eq!(second.current().name, "second");
        assert_eq!(second.state(), EntityState::Unchanged);
        assert_eq!(
            registry
                .current_snapshot_for_key::<SnapshotEntity>(SqlValue::I64(7))
                .expect("newly registered identity should be available")
                .name,
            "second"
        );
    }

    #[test]
    fn tracked_for_does_not_return_stale_handles_after_clear() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_loaded(SnapshotEntity {
            name: "first".to_string(),
        });
        let mut second = Tracked::from_loaded(SnapshotEntity {
            name: "second".to_string(),
        });
        first
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();
        second
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(8))
            .unwrap();

        registry.clear();

        assert!(registry.tracked_for::<SnapshotEntity>().is_empty());
        assert!(registry.registrations().is_empty());
        assert_eq!(first.state(), EntityState::Unchanged);
        assert_eq!(second.state(), EntityState::Unchanged);
    }

    #[test]
    fn current_snapshot_for_key_syncs_attached_wrapper_current() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(SnapshotEntity {
            name: "loaded".to_string(),
        });
        tracked
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        tracked.current_mut().name = "changed through wrapper".to_string();

        let snapshot = registry
            .current_snapshot_for_key::<SnapshotEntity>(SqlValue::I64(7))
            .expect("tracked identity should have a current snapshot");

        assert_eq!(snapshot.name, "changed through wrapper");
        assert_eq!(tracked.state(), EntityState::Modified);
        assert_eq!(registry.registrations()[0].state, EntityState::Modified);
    }

    #[test]
    fn current_snapshot_for_key_scopes_lookup_by_rust_type() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(SnapshotEntity {
            name: "loaded".to_string(),
        });
        tracked
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        assert_eq!(
            registry.current_snapshot_for_key::<SnapshotEntityAlias>(SqlValue::I64(7)),
            None
        );
        assert_eq!(
            registry
                .current_snapshot_for_key::<SnapshotEntity>(SqlValue::I64(7))
                .expect("tracked identity should have a snapshot")
                .name,
            "loaded"
        );
    }

    #[test]
    fn current_snapshot_for_key_ignores_unregistered_identity() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(SnapshotEntity {
            name: "loaded".to_string(),
        });
        tracked
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();
        let registration_id = tracked.registration_id.expect("registered");

        registry.unregister(registration_id);

        assert_eq!(
            registry.current_snapshot_for_key::<SnapshotEntity>(SqlValue::I64(7)),
            None
        );
    }

    #[test]
    fn current_snapshot_for_key_ignores_cleared_identity() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(SnapshotEntity {
            name: "loaded".to_string(),
        });
        tracked
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        registry.clear();

        assert_eq!(
            registry.current_snapshot_for_key::<SnapshotEntity>(SqlValue::I64(7)),
            None
        );
    }

    #[test]
    fn tracking_registry_rejects_duplicate_loaded_identity() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_loaded(DummyEntity);
        let mut second = Tracked::from_loaded(DummyEntity);

        first
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();
        let error = second
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap_err();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "entity `DummyEntity` with primary key value `I64(7)` already has a live tracked handle in this context; detach or drop the existing handle before loading it again"
        );
    }

    #[test]
    fn duplicate_loaded_identity_error_leaves_rejected_wrapper_detached() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_loaded(DummyEntity);

        first
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        {
            let mut duplicate = Tracked::from_loaded(DummyEntity);
            let error = duplicate
                .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
                .unwrap_err();

            assert_eq!(
                error.message(),
                "entity `DummyEntity` with primary key value `I64(7)` already has a live tracked handle in this context; detach or drop the existing handle before loading it again"
            );
            assert_eq!(duplicate.state(), EntityState::Unchanged);
            assert_eq!(registry.entry_count(), 1);
        }

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Unchanged);
    }

    #[test]
    fn tracking_registry_scopes_loaded_identity_by_rust_type() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_loaded(DummyEntity);
        let mut second = Tracked::from_loaded(DummyEntityAlias);

        first
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();
        second
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();

        assert_eq!(registry.entry_count(), 2);
    }

    #[test]
    fn tracking_registry_allows_multiple_added_entities_with_temporary_identities() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_added(DummyEntity);
        let mut second = Tracked::from_added(DummyEntity);

        first.attach_registry_added(Arc::clone(&registry));
        second.attach_registry_added(Arc::clone(&registry));

        assert_eq!(registry.entry_count(), 2);
    }

    #[test]
    fn tracking_registry_updates_temporary_identity_to_persisted_identity() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_added(DummyEntity);
        tracked.attach_registry_added(Arc::clone(&registry));

        registry
            .update_persisted_identity::<DummyEntity>(
                tracked.registration_id.expect("registered"),
                SqlValue::I64(11),
            )
            .unwrap();

        let mut duplicate = Tracked::from_loaded(DummyEntity);
        let error = duplicate
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(11))
            .unwrap_err();

        assert!(error.message().contains("live tracked handle"));
    }

    #[test]
    fn tracking_registry_rejects_persisted_identity_update_collision_without_mutating_entry() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut existing = Tracked::from_loaded(DummyEntity);
        let mut pending = Tracked::from_added(DummyEntity);

        existing
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(11))
            .unwrap();
        pending.attach_registry_added(Arc::clone(&registry));

        let pending_registration = pending.registration_id.expect("registered pending entity");
        let error = registry
            .update_persisted_identity::<DummyEntity>(pending_registration, SqlValue::I64(11))
            .unwrap_err();

        assert!(error.message().contains("already tracked"));
        assert_eq!(registry.entry_count(), 2);

        let mut duplicate = Tracked::from_loaded(DummyEntity);
        let duplicate_error = duplicate
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(11))
            .unwrap_err();
        assert!(duplicate_error.message().contains("live tracked handle"));

        registry
            .update_persisted_identity::<DummyEntity>(pending_registration, SqlValue::I64(12))
            .unwrap();

        let mut second_duplicate = Tracked::from_loaded(DummyEntity);
        let second_duplicate_error = second_duplicate
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(12))
            .unwrap_err();
        assert!(
            second_duplicate_error
                .message()
                .contains("live tracked handle")
        );
    }

    #[test]
    fn tracking_registry_rejects_persisted_identity_update_collision_with_detached_entry() {
        let registry = Arc::new(TrackingRegistry::default());

        {
            let mut existing = Tracked::from_loaded(SnapshotEntity {
                name: "existing".to_string(),
            });
            existing
                .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(11))
                .unwrap();
            existing.current_mut().name = "existing changed".to_string();
        }

        let mut pending = Tracked::from_added(SnapshotEntity {
            name: "pending".to_string(),
        });
        pending.attach_registry_added(Arc::clone(&registry));
        let pending_registration = pending.registration_id.expect("registered pending entity");

        let error = registry
            .update_persisted_identity::<SnapshotEntity>(pending_registration, SqlValue::I64(11))
            .unwrap_err();

        assert_eq!(
            error.message(),
            "entity `DummyEntity` with primary key value `I64(11)` is already tracked in this context"
        );
        assert_eq!(registry.entry_count(), 2);

        let mut reattached_existing = Tracked::from_loaded(SnapshotEntity {
            name: "fresh database value".to_string(),
        });
        reattached_existing
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(11))
            .unwrap();

        assert_eq!(reattached_existing.current().name, "existing changed");

        registry
            .update_persisted_identity::<SnapshotEntity>(pending_registration, SqlValue::I64(12))
            .unwrap();

        let mut duplicate_pending = Tracked::from_loaded(SnapshotEntity {
            name: "duplicate pending".to_string(),
        });
        let duplicate_error = duplicate_pending
            .attach_registry_loaded(Arc::clone(&registry), SqlValue::I64(12))
            .unwrap_err();

        assert!(duplicate_error.message().contains("live tracked handle"));
    }

    #[test]
    fn tracking_registry_rejects_persisted_identity_update_for_missing_registration() {
        let registry = TrackingRegistry::default();

        let error = registry
            .update_persisted_identity::<DummyEntity>(99, SqlValue::I64(11))
            .unwrap_err();

        assert_eq!(error.message(), "tracked entity registration was not found");
    }

    #[test]
    fn tracking_registry_clear_removes_all_entries() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut first = Tracked::from_added(DummyEntity);
        let mut second = Tracked::from_added(DummyEntity);
        first.attach_registry_added(Arc::clone(&registry));
        second.attach_registry_added(Arc::clone(&registry));

        registry.clear();

        assert_eq!(registry.entry_count(), 0);
        assert!(registry.registrations().is_empty());
    }

    #[test]
    fn detach_registry_unregisters_without_dropping_wrapper() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);
        tracked.attach_registry(Arc::clone(&registry));

        tracked.detach_registry();

        assert_eq!(registry.entry_count(), 0);
        assert_eq!(tracked.state(), EntityState::Unchanged);
    }

    #[test]
    fn public_detach_is_idempotent_and_keeps_visible_state() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);
        tracked.attach_registry(Arc::clone(&registry));
        tracked.mark_deleted();

        tracked.detach();
        tracked.detach();

        assert_eq!(registry.entry_count(), 0);
        assert_eq!(tracked.state(), EntityState::Deleted);
    }

    #[test]
    fn public_detach_unregisters_without_resetting_state() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);
        tracked.attach_registry(Arc::clone(&registry));
        tracked.mark_modified();

        tracked.detach();

        assert_eq!(registry.entry_count(), 0);
        assert_eq!(tracked.state(), EntityState::Modified);
    }

    #[test]
    fn tracking_registry_unregister_missing_registration_is_noop() {
        let registry = Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(DummyEntity);
        tracked.attach_registry(Arc::clone(&registry));

        registry.unregister(99);

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Unchanged);
    }

    #[test]
    fn dropping_tracked_entity_unregisters_it_from_registry() {
        let registry = Arc::new(TrackingRegistry::default());

        {
            let mut tracked = Tracked::from_loaded(DummyEntity);
            tracked.attach_registry(Arc::clone(&registry));
            assert_eq!(registry.entry_count(), 1);
        }

        assert_eq!(registry.entry_count(), 0);
    }

    #[test]
    fn save_changes_plan_orders_added_parents_before_children() {
        let plan = save_changes_operation_plan(&[
            &ORDER_ITEM_METADATA,
            &DUMMY_ENTITY_METADATA,
            &ORDER_METADATA,
        ])
        .unwrap();

        assert_eq!(plan.added_order(), &[1, 2, 0]);
        assert_eq!(plan.modified_order(), &[1, 2, 0]);
    }

    #[test]
    fn save_changes_plan_orders_deleted_children_before_parents() {
        let plan = save_changes_operation_plan(&[
            &ORDER_ITEM_METADATA,
            &DUMMY_ENTITY_METADATA,
            &ORDER_METADATA,
        ])
        .unwrap();

        assert_eq!(plan.deleted_order(), &[0, 2, 1]);
    }

    #[test]
    fn save_changes_plan_preserves_context_order_without_dependencies() {
        let plan = save_changes_operation_plan(&[&ORDER_METADATA, &DUMMY_ENTITY_METADATA]).unwrap();

        assert_eq!(plan.added_order(), &[0, 1]);
        assert_eq!(plan.modified_order(), &[0, 1]);
        assert_eq!(plan.deleted_order(), &[1, 0]);
    }

    #[test]
    fn save_changes_plan_ignores_foreign_keys_to_entities_outside_context() {
        let plan =
            save_changes_operation_plan(&[&ORDER_ITEM_METADATA, &DUMMY_ENTITY_METADATA]).unwrap();

        assert_eq!(plan.added_order(), &[0, 1]);
        assert_eq!(plan.modified_order(), &[0, 1]);
        assert_eq!(plan.deleted_order(), &[1, 0]);
    }

    #[test]
    fn save_changes_plan_ignores_simple_self_references() {
        let plan =
            save_changes_operation_plan(&[&CATEGORY_METADATA, &DUMMY_ENTITY_METADATA]).unwrap();

        assert_eq!(plan.added_order(), &[0, 1]);
        assert_eq!(plan.modified_order(), &[0, 1]);
        assert_eq!(plan.deleted_order(), &[1, 0]);
    }

    #[test]
    fn save_changes_plan_rejects_foreign_key_cycles() {
        let error =
            save_changes_operation_plan(&[&CYCLE_A_METADATA, &CYCLE_B_METADATA]).unwrap_err();

        assert!(error.message().contains("foreign-key cycle"));
    }
}
