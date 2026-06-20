use crate::audit_runtime::apply_audit_values;
use crate::dbset_query::{DbSetQuery, tenant_value_matches_column_type};
use crate::soft_delete_runtime::{
    SoftDeleteOperation, SoftDeleteProvider, SoftDeleteRequestValues, SoftDeleteValues,
    apply_soft_delete_values,
};
use crate::tracking::{
    ReconciledRelationshipOperationKind, RelationshipCommandBatchContext,
    RelationshipPrincipalValues, RelationshipReconciliationPlan, relationship_commands_from_batch,
};
use crate::{AuditEntity, AuditOperation, AuditProvider, AuditRequestValues, AuditValues};
use crate::{
    IncludeCollection, RawCommand, RawQuery, RelationshipMutationSource, SoftDeleteEntity,
    TenantContext, TenantScopedEntity, Tracked, TrackingRegistry, TrackingRegistryHandle,
};
use core::future::Future;
use std::marker::PhantomData;
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use crate::{EntityPersist, EntityPrimaryKey};
use sql_orm_core::{
    Changeset, ColumnValue, Entity, EntityMetadata, FromRow, Insertable, NavigationKind,
    NavigationMetadata, OrmError, SqlTypeMapping, SqlValue,
};
use sql_orm_query::{
    ColumnRef, DeleteQuery, Expr, InsertQuery, Predicate, SelectQuery, TableRef, UpdateQuery,
};
use sql_orm_sqlserver::SqlServerCompiler;
use sql_orm_tiberius::{
    MssqlConnection, MssqlConnectionConfig, MssqlOperationalOptions, MssqlRetryOptions,
    TokioConnectionStream,
};
#[cfg(feature = "pool-bb8")]
use sql_orm_tiberius::{MssqlPool, MssqlPooledConnection};

/// Shared database access handle used by contexts, `DbSet`s, raw SQL, and
/// transactions.
///
/// A `SharedConnection` can wrap either one direct SQL Server connection or,
/// behind the `pool-bb8` feature, a pool. Runtime values such as audit values,
/// soft-delete values, and the active tenant are stored alongside the physical
/// connection handle and are preserved when derived contexts clone the handle.
#[derive(Clone)]
pub struct SharedConnection {
    inner: Arc<SharedConnectionInner>,
    runtime: Arc<SharedConnectionRuntime>,
    transaction_scope: Option<usize>,
}

/// Active tenant value currently attached to a shared connection.
///
/// Tenant-scoped entities compare their tenant policy column with this value
/// before compiling reads and writes. A column mismatch or missing tenant fails
/// closed for tenant-scoped entities.
#[derive(Debug, Clone, PartialEq)]
pub struct ActiveTenant {
    /// Physical tenant column name expected by tenant-scoped entities.
    pub column_name: &'static str,
    /// SQL value compared against the tenant column.
    pub value: SqlValue,
}

impl ActiveTenant {
    /// Normalizes a user-defined tenant context into the runtime tenant value.
    pub fn from_context<T: TenantContext>(tenant: &T) -> Self {
        Self {
            column_name: T::COLUMN_NAME,
            value: tenant.tenant_value(),
        }
    }
}

enum SharedConnectionInner {
    Direct(Box<tokio::sync::Mutex<MssqlConnection<TokioConnectionStream>>>),
    #[cfg(feature = "pool-bb8")]
    Pool(Box<MssqlPool>),
}

#[derive(Clone, Default)]
struct SharedConnectionRuntime {
    audit_provider: Option<Arc<dyn AuditProvider>>,
    audit_request_values: Option<Arc<AuditRequestValues>>,
    soft_delete_provider: Option<Arc<dyn SoftDeleteProvider>>,
    soft_delete_request_values: Option<Arc<SoftDeleteRequestValues>>,
    active_tenant: Option<ActiveTenant>,
    transaction_depth: Arc<AtomicUsize>,
    active_transaction_scope: Arc<AtomicUsize>,
    next_transaction_scope: Arc<AtomicUsize>,
    #[cfg(feature = "pool-bb8")]
    pinned_pool_connection: Arc<tokio::sync::Mutex<Option<MssqlPooledConnection<'static>>>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SharedConnectionKind {
    Direct,
    #[cfg(feature = "pool-bb8")]
    Pool,
}

#[cfg(feature = "pool-bb8")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PooledTransactionCleanupPhase {
    BeginError,
    AfterCommitAttempt,
    AfterRollbackAttempt,
}

#[cfg(feature = "pool-bb8")]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PooledTransactionCleanupPlan {
    restore_retry: bool,
    exit_transaction_scope: bool,
    clear_pinned_connection: bool,
}

#[cfg(feature = "pool-bb8")]
impl PooledTransactionCleanupPlan {
    fn for_phase(phase: PooledTransactionCleanupPhase) -> Self {
        match phase {
            PooledTransactionCleanupPhase::BeginError => Self {
                restore_retry: false,
                exit_transaction_scope: false,
                clear_pinned_connection: true,
            },
            PooledTransactionCleanupPhase::AfterCommitAttempt
            | PooledTransactionCleanupPhase::AfterRollbackAttempt => Self {
                restore_retry: true,
                exit_transaction_scope: true,
                clear_pinned_connection: true,
            },
        }
    }
}

pub enum SharedConnectionGuard<'a> {
    /// Guard for a direct connection held through an async mutex.
    Direct(tokio::sync::MutexGuard<'a, MssqlConnection<TokioConnectionStream>>),
    #[cfg(feature = "pool-bb8")]
    /// Guard for one connection acquired from a pool.
    Pool(Box<MssqlPooledConnection<'a>>),
    #[cfg(feature = "pool-bb8")]
    /// Guard for the connection pinned to the active pooled transaction.
    PinnedPool(tokio::sync::MutexGuard<'a, Option<MssqlPooledConnection<'static>>>),
}

impl SharedConnection {
    /// Creates a shared handle from an already-open direct SQL Server
    /// connection.
    pub fn from_connection(connection: MssqlConnection<TokioConnectionStream>) -> Self {
        Self {
            inner: Arc::new(SharedConnectionInner::Direct(Box::new(
                tokio::sync::Mutex::new(connection),
            ))),
            runtime: Arc::new(SharedConnectionRuntime::default()),
            transaction_scope: None,
        }
    }

    #[cfg(feature = "pool-bb8")]
    /// Creates a shared handle backed by an `MssqlPool`.
    ///
    /// Each operation acquires a pooled connection as needed. Runtime context
    /// values still live on the `SharedConnection` wrapper, not inside the pool.
    pub fn from_pool(pool: MssqlPool) -> Self {
        Self {
            inner: Arc::new(SharedConnectionInner::Pool(Box::new(pool))),
            runtime: Arc::new(SharedConnectionRuntime::default()),
            transaction_scope: None,
        }
    }

    /// Returns a clone of this handle with an audit provider configured.
    ///
    /// The provider is consulted by insert/update paths for entities declaring
    /// `#[orm(audit = Audit)]` after explicit mutation values and request
    /// values have had priority.
    pub fn with_audit_provider(&self, provider: Arc<dyn AuditProvider>) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::new(SharedConnectionRuntime {
                audit_provider: Some(provider),
                audit_request_values: self.runtime.audit_request_values.clone(),
                soft_delete_provider: self.runtime.soft_delete_provider.clone(),
                soft_delete_request_values: self.runtime.soft_delete_request_values.clone(),
                active_tenant: self.runtime.active_tenant.clone(),
                transaction_depth: Arc::clone(&self.runtime.transaction_depth),
                active_transaction_scope: Arc::clone(&self.runtime.active_transaction_scope),
                next_transaction_scope: Arc::clone(&self.runtime.next_transaction_scope),
                #[cfg(feature = "pool-bb8")]
                pinned_pool_connection: Arc::clone(&self.runtime.pinned_pool_connection),
            }),
            transaction_scope: self.transaction_scope,
        }
    }

    /// Returns a clone of this handle with low-level audit request values.
    ///
    /// Prefer `with_audit_values(...)` when using a struct derived with
    /// `#[derive(AuditFields)]`.
    pub fn with_audit_request_values(&self, request_values: AuditRequestValues) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::new(SharedConnectionRuntime {
                audit_provider: self.runtime.audit_provider.clone(),
                audit_request_values: Some(Arc::new(request_values)),
                soft_delete_provider: self.runtime.soft_delete_provider.clone(),
                soft_delete_request_values: self.runtime.soft_delete_request_values.clone(),
                active_tenant: self.runtime.active_tenant.clone(),
                transaction_depth: Arc::clone(&self.runtime.transaction_depth),
                active_transaction_scope: Arc::clone(&self.runtime.active_transaction_scope),
                next_transaction_scope: Arc::clone(&self.runtime.next_transaction_scope),
                #[cfg(feature = "pool-bb8")]
                pinned_pool_connection: Arc::clone(&self.runtime.pinned_pool_connection),
            }),
            transaction_scope: self.transaction_scope,
        }
    }

    /// Returns a clone of this handle with typed audit request values.
    ///
    /// The same struct used for `#[derive(AuditFields)]` can be passed here as
    /// runtime values. Values are converted to `AuditRequestValues` and keep
    /// the existing precedence rules.
    pub fn with_audit_values<V: AuditValues>(&self, values: V) -> Self {
        self.with_audit_request_values(AuditRequestValues::new(values.audit_values()))
    }

    /// Returns a clone of this handle with audit request values cleared.
    pub fn clear_audit_request_values(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::new(SharedConnectionRuntime {
                audit_provider: self.runtime.audit_provider.clone(),
                audit_request_values: None,
                soft_delete_provider: self.runtime.soft_delete_provider.clone(),
                soft_delete_request_values: self.runtime.soft_delete_request_values.clone(),
                active_tenant: self.runtime.active_tenant.clone(),
                transaction_depth: Arc::clone(&self.runtime.transaction_depth),
                active_transaction_scope: Arc::clone(&self.runtime.active_transaction_scope),
                next_transaction_scope: Arc::clone(&self.runtime.next_transaction_scope),
                #[cfg(feature = "pool-bb8")]
                pinned_pool_connection: Arc::clone(&self.runtime.pinned_pool_connection),
            }),
            transaction_scope: self.transaction_scope,
        }
    }

    /// Returns a clone of this handle with a soft-delete provider configured.
    ///
    /// The provider is used by delete paths for entities declaring
    /// `#[orm(soft_delete = SoftDelete)]`.
    pub fn with_soft_delete_provider(&self, provider: Arc<dyn SoftDeleteProvider>) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::new(SharedConnectionRuntime {
                audit_provider: self.runtime.audit_provider.clone(),
                audit_request_values: self.runtime.audit_request_values.clone(),
                soft_delete_provider: Some(provider),
                soft_delete_request_values: self.runtime.soft_delete_request_values.clone(),
                active_tenant: self.runtime.active_tenant.clone(),
                transaction_depth: Arc::clone(&self.runtime.transaction_depth),
                active_transaction_scope: Arc::clone(&self.runtime.active_transaction_scope),
                next_transaction_scope: Arc::clone(&self.runtime.next_transaction_scope),
                #[cfg(feature = "pool-bb8")]
                pinned_pool_connection: Arc::clone(&self.runtime.pinned_pool_connection),
            }),
            transaction_scope: self.transaction_scope,
        }
    }

    /// Returns a clone of this handle with low-level soft-delete request
    /// values.
    ///
    /// Prefer `with_soft_delete_values(...)` when using a struct derived with
    /// `#[derive(SoftDeleteFields)]`.
    pub fn with_soft_delete_request_values(&self, request_values: SoftDeleteRequestValues) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::new(SharedConnectionRuntime {
                audit_provider: self.runtime.audit_provider.clone(),
                audit_request_values: self.runtime.audit_request_values.clone(),
                soft_delete_provider: self.runtime.soft_delete_provider.clone(),
                soft_delete_request_values: Some(Arc::new(request_values)),
                active_tenant: self.runtime.active_tenant.clone(),
                transaction_depth: Arc::clone(&self.runtime.transaction_depth),
                active_transaction_scope: Arc::clone(&self.runtime.active_transaction_scope),
                next_transaction_scope: Arc::clone(&self.runtime.next_transaction_scope),
                #[cfg(feature = "pool-bb8")]
                pinned_pool_connection: Arc::clone(&self.runtime.pinned_pool_connection),
            }),
            transaction_scope: self.transaction_scope,
        }
    }

    /// Returns a clone of this handle with typed soft-delete request values.
    ///
    /// The same struct used for `#[derive(SoftDeleteFields)]` can be passed
    /// here as runtime delete values.
    pub fn with_soft_delete_values<V: SoftDeleteValues>(&self, values: V) -> Self {
        self.with_soft_delete_request_values(SoftDeleteRequestValues::new(
            values.soft_delete_values(),
        ))
    }

    /// Returns a clone of this handle with soft-delete request values cleared.
    pub fn clear_soft_delete_request_values(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::new(SharedConnectionRuntime {
                audit_provider: self.runtime.audit_provider.clone(),
                audit_request_values: self.runtime.audit_request_values.clone(),
                soft_delete_provider: self.runtime.soft_delete_provider.clone(),
                soft_delete_request_values: None,
                active_tenant: self.runtime.active_tenant.clone(),
                transaction_depth: Arc::clone(&self.runtime.transaction_depth),
                active_transaction_scope: Arc::clone(&self.runtime.active_transaction_scope),
                next_transaction_scope: Arc::clone(&self.runtime.next_transaction_scope),
                #[cfg(feature = "pool-bb8")]
                pinned_pool_connection: Arc::clone(&self.runtime.pinned_pool_connection),
            }),
            transaction_scope: self.transaction_scope,
        }
    }

    /// Returns a clone of this handle with an active tenant configured.
    ///
    /// Tenant-scoped reads and writes fail closed if this tenant is absent,
    /// has a different column name, or has a value incompatible with the tenant
    /// column.
    pub fn with_tenant<T: TenantContext>(&self, tenant: T) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::new(SharedConnectionRuntime {
                audit_provider: self.runtime.audit_provider.clone(),
                audit_request_values: self.runtime.audit_request_values.clone(),
                soft_delete_provider: self.runtime.soft_delete_provider.clone(),
                soft_delete_request_values: self.runtime.soft_delete_request_values.clone(),
                active_tenant: Some(ActiveTenant::from_context(&tenant)),
                transaction_depth: Arc::clone(&self.runtime.transaction_depth),
                active_transaction_scope: Arc::clone(&self.runtime.active_transaction_scope),
                next_transaction_scope: Arc::clone(&self.runtime.next_transaction_scope),
                #[cfg(feature = "pool-bb8")]
                pinned_pool_connection: Arc::clone(&self.runtime.pinned_pool_connection),
            }),
            transaction_scope: self.transaction_scope,
        }
    }

    /// Returns a clone of this handle without an active tenant.
    pub fn clear_tenant(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::new(SharedConnectionRuntime {
                audit_provider: self.runtime.audit_provider.clone(),
                audit_request_values: self.runtime.audit_request_values.clone(),
                soft_delete_provider: self.runtime.soft_delete_provider.clone(),
                soft_delete_request_values: self.runtime.soft_delete_request_values.clone(),
                active_tenant: None,
                transaction_depth: Arc::clone(&self.runtime.transaction_depth),
                active_transaction_scope: Arc::clone(&self.runtime.active_transaction_scope),
                next_transaction_scope: Arc::clone(&self.runtime.next_transaction_scope),
                #[cfg(feature = "pool-bb8")]
                pinned_pool_connection: Arc::clone(&self.runtime.pinned_pool_connection),
            }),
            transaction_scope: self.transaction_scope,
        }
    }

    /// Acquires the underlying SQL Server connection for one operation.
    ///
    /// Direct connections lock the shared mutex. Pooled connections acquire a
    /// connection from the pool for the lifetime of the returned guard.
    pub async fn lock(&self) -> Result<SharedConnectionGuard<'_>, OrmError> {
        self.ensure_can_use_active_transaction_scope()?;
        match self.inner.as_ref() {
            SharedConnectionInner::Direct(connection) => {
                Ok(SharedConnectionGuard::Direct(connection.lock().await))
            }
            #[cfg(feature = "pool-bb8")]
            SharedConnectionInner::Pool(pool) => {
                let pinned_connection = self.runtime.pinned_pool_connection.lock().await;
                if pinned_connection.is_some() {
                    return Ok(SharedConnectionGuard::PinnedPool(pinned_connection));
                }
                drop(pinned_connection);

                Ok(SharedConnectionGuard::Pool(Box::new(pool.acquire().await?)))
            }
        }
    }

    #[doc(hidden)]
    pub async fn run_transaction<F, Fut, T>(&self, operation: F) -> Result<T, OrmError>
    where
        F: FnOnce(SharedConnection) -> Fut,
        Fut: Future<Output = Result<T, OrmError>>,
    {
        if self.is_transaction_active() {
            return Err(OrmError::transaction(
                "nested db.transaction calls are not supported; use the transaction context passed to the active transaction",
            ));
        }
        ensure_transactions_supported(self.kind())?;

        let transaction_scope = self.allocate_transaction_scope();
        let transaction_connection = self.with_transaction_scope(transaction_scope);

        #[cfg(feature = "pool-bb8")]
        if let SharedConnectionInner::Pool(pool) = self.inner.as_ref() {
            return transaction_connection
                .run_pooled_transaction(pool, operation)
                .await;
        }

        let mut connection = transaction_connection.lock().await?;
        connection.begin_transaction_scope().await?;
        let retry_options = connection.replace_retry_options(MssqlRetryOptions::disabled());
        drop(connection);

        self.enter_transaction_scope(transaction_scope);

        let result = operation(transaction_connection.clone()).await;

        match result {
            Ok(value) => {
                let mut connection = transaction_connection.lock().await?;
                let commit_result = connection.commit_transaction().await;
                connection.replace_retry_options(retry_options);
                self.exit_transaction_scope();
                commit_result?;
                Ok(value)
            }
            Err(error) => {
                let mut connection = transaction_connection.lock().await?;
                let rollback_result = connection.rollback_transaction().await;
                connection.replace_retry_options(retry_options);
                self.exit_transaction_scope();
                rollback_result?;
                Err(error)
            }
        }
    }

    #[cfg(feature = "pool-bb8")]
    async fn run_pooled_transaction<F, Fut, T>(
        &self,
        pool: &MssqlPool,
        operation: F,
    ) -> Result<T, OrmError>
    where
        F: FnOnce(SharedConnection) -> Fut,
        Fut: Future<Output = Result<T, OrmError>>,
    {
        self.install_pinned_pool_connection(pool.acquire_owned().await?)
            .await?;

        let begin_result = async {
            let mut connection = self.lock().await?;
            connection.begin_transaction_scope().await
        }
        .await;

        if let Err(error) = begin_result {
            self.cleanup_pinned_pool_transaction(
                PooledTransactionCleanupPlan::for_phase(PooledTransactionCleanupPhase::BeginError),
                None,
            )
            .await?;
            return Err(error);
        }

        let retry_options = self.disable_pinned_pool_retry().await?;
        let transaction_scope = self
            .transaction_scope
            .expect("pooled transaction handle must be scoped");
        self.enter_transaction_scope(transaction_scope);
        let result = operation(self.clone()).await;

        match result {
            Ok(value) => {
                let commit_result = async {
                    let mut connection = self.lock().await?;
                    connection.commit_transaction().await
                }
                .await;
                self.cleanup_pinned_pool_transaction(
                    PooledTransactionCleanupPlan::for_phase(
                        PooledTransactionCleanupPhase::AfterCommitAttempt,
                    ),
                    Some(retry_options),
                )
                .await?;
                commit_result?;
                Ok(value)
            }
            Err(error) => {
                let rollback_result = async {
                    let mut connection = self.lock().await?;
                    connection.rollback_transaction().await
                }
                .await;
                self.cleanup_pinned_pool_transaction(
                    PooledTransactionCleanupPlan::for_phase(
                        PooledTransactionCleanupPhase::AfterRollbackAttempt,
                    ),
                    Some(retry_options),
                )
                .await?;
                rollback_result?;
                Err(error)
            }
        }
    }

    fn kind(&self) -> SharedConnectionKind {
        match self.inner.as_ref() {
            SharedConnectionInner::Direct(_) => SharedConnectionKind::Direct,
            #[cfg(feature = "pool-bb8")]
            SharedConnectionInner::Pool(_) => SharedConnectionKind::Pool,
        }
    }

    #[doc(hidden)]
    pub fn is_transaction_active(&self) -> bool {
        self.runtime.transaction_depth.load(Ordering::SeqCst) > 0
    }

    fn allocate_transaction_scope(&self) -> usize {
        let scope = self
            .runtime
            .next_transaction_scope
            .fetch_add(1, Ordering::SeqCst)
            .wrapping_add(1);

        if scope == 0 { 1 } else { scope }
    }

    fn with_transaction_scope(&self, scope: usize) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            runtime: Arc::clone(&self.runtime),
            transaction_scope: Some(scope),
        }
    }

    fn active_transaction_scope(&self) -> Option<usize> {
        match self.runtime.active_transaction_scope.load(Ordering::SeqCst) {
            0 => None,
            scope => Some(scope),
        }
    }

    fn ensure_can_use_active_transaction_scope(&self) -> Result<(), OrmError> {
        ensure_transaction_scope_allows_lock(
            self.active_transaction_scope(),
            self.transaction_scope,
        )
    }

    fn enter_transaction_scope(&self, scope: usize) {
        self.runtime
            .active_transaction_scope
            .store(scope, Ordering::SeqCst);
        self.runtime
            .transaction_depth
            .fetch_add(1, Ordering::SeqCst);
    }

    fn exit_transaction_scope(&self) {
        let _ = self.runtime.transaction_depth.fetch_update(
            Ordering::SeqCst,
            Ordering::SeqCst,
            |depth| Some(depth.saturating_sub(1)),
        );
        self.runtime
            .active_transaction_scope
            .store(0, Ordering::SeqCst);
    }

    #[cfg(feature = "pool-bb8")]
    async fn install_pinned_pool_connection(
        &self,
        connection: MssqlPooledConnection<'static>,
    ) -> Result<(), OrmError> {
        let mut pinned_connection = self.runtime.pinned_pool_connection.lock().await;
        if pinned_connection.is_some() {
            return Err(OrmError::transaction(
                "a pooled transaction connection is already pinned",
            ));
        }

        *pinned_connection = Some(connection);
        Ok(())
    }

    #[cfg(feature = "pool-bb8")]
    async fn clear_pinned_pool_connection(&self) {
        let mut pinned_connection = self.runtime.pinned_pool_connection.lock().await;
        *pinned_connection = None;
    }

    #[cfg(feature = "pool-bb8")]
    async fn disable_pinned_pool_retry(&self) -> Result<MssqlRetryOptions, OrmError> {
        let mut pinned_connection = self.runtime.pinned_pool_connection.lock().await;
        let connection = pinned_connection.as_mut().ok_or_else(|| {
            OrmError::transaction("pinned pooled transaction connection is missing")
        })?;

        Ok(connection.replace_retry_options(MssqlRetryOptions::disabled()))
    }

    #[cfg(feature = "pool-bb8")]
    async fn restore_pinned_pool_retry(
        &self,
        retry_options: MssqlRetryOptions,
    ) -> Result<(), OrmError> {
        let mut pinned_connection = self.runtime.pinned_pool_connection.lock().await;
        let connection = pinned_connection.as_mut().ok_or_else(|| {
            OrmError::transaction("pinned pooled transaction connection is missing")
        })?;

        connection.replace_retry_options(retry_options);
        Ok(())
    }

    #[cfg(feature = "pool-bb8")]
    async fn cleanup_pinned_pool_transaction(
        &self,
        plan: PooledTransactionCleanupPlan,
        retry_options: Option<MssqlRetryOptions>,
    ) -> Result<(), OrmError> {
        let restore_result = if plan.restore_retry {
            match retry_options {
                Some(retry_options) => self.restore_pinned_pool_retry(retry_options).await,
                None => Err(OrmError::transaction(
                    "missing retry options for pooled transaction cleanup",
                )),
            }
        } else {
            Ok(())
        };

        if plan.exit_transaction_scope {
            self.exit_transaction_scope();
        }

        if plan.clear_pinned_connection {
            self.clear_pinned_pool_connection().await;
        }

        restore_result
    }

    #[allow(dead_code)]
    pub(crate) fn audit_provider(&self) -> Option<Arc<dyn AuditProvider>> {
        self.runtime.audit_provider.clone()
    }

    #[allow(dead_code)]
    pub(crate) fn audit_request_values(&self) -> Option<Arc<AuditRequestValues>> {
        self.runtime.audit_request_values.clone()
    }

    pub(crate) fn soft_delete_provider(&self) -> Option<Arc<dyn SoftDeleteProvider>> {
        self.runtime.soft_delete_provider.clone()
    }

    pub(crate) fn soft_delete_request_values(&self) -> Option<Arc<SoftDeleteRequestValues>> {
        self.runtime.soft_delete_request_values.clone()
    }

    #[doc(hidden)]
    /// Returns the active tenant attached to this handle, if any.
    pub fn active_tenant(&self) -> Option<ActiveTenant> {
        self.runtime.active_tenant.clone()
    }
}

fn transaction_scope_allows_lock(active_scope: Option<usize>, handle_scope: Option<usize>) -> bool {
    active_scope.is_none() || active_scope == handle_scope
}

fn ensure_transaction_scope_allows_lock(
    active_scope: Option<usize>,
    handle_scope: Option<usize>,
) -> Result<(), OrmError> {
    if transaction_scope_allows_lock(active_scope, handle_scope) {
        Ok(())
    } else {
        Err(OrmError::transaction(
            "a transaction is active on this shared connection; use the transaction context passed to db.transaction(...)",
        ))
    }
}

fn ensure_transactions_supported(kind: SharedConnectionKind) -> Result<(), OrmError> {
    match kind {
        SharedConnectionKind::Direct => Ok(()),
        #[cfg(feature = "pool-bb8")]
        SharedConnectionKind::Pool => Ok(()),
    }
}

impl core::ops::Deref for SharedConnectionGuard<'_> {
    type Target = MssqlConnection<TokioConnectionStream>;

    fn deref(&self) -> &Self::Target {
        match self {
            SharedConnectionGuard::Direct(connection) => connection,
            #[cfg(feature = "pool-bb8")]
            SharedConnectionGuard::Pool(connection) => connection,
            #[cfg(feature = "pool-bb8")]
            SharedConnectionGuard::PinnedPool(connection) => connection
                .as_ref()
                .expect("pinned pooled transaction connection is missing"),
        }
    }
}

impl core::ops::DerefMut for SharedConnectionGuard<'_> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        match self {
            SharedConnectionGuard::Direct(connection) => connection,
            #[cfg(feature = "pool-bb8")]
            SharedConnectionGuard::Pool(connection) => connection,
            #[cfg(feature = "pool-bb8")]
            SharedConnectionGuard::PinnedPool(connection) => connection
                .as_mut()
                .expect("pinned pooled transaction connection is missing"),
        }
    }
}

/// Application database context contract.
///
/// `#[derive(DbContext)]` implements this trait for structs whose fields are
/// `DbSet<T>`. The trait centralizes connection access, health checks, raw SQL,
/// transactions, and `save_changes()` support while keeping SQL
/// generation in `sql-orm-sqlserver` and execution in `sql-orm-tiberius`.
pub trait DbContext: Sized {
    /// Builds a context from an existing shared connection handle.
    fn from_shared_connection(connection: SharedConnection) -> Self;
    /// Returns the shared connection handle used by this context.
    fn shared_connection(&self) -> SharedConnection;
    #[doc(hidden)]
    fn tracking_registry(&self) -> TrackingRegistryHandle;

    /// Clears every tracking entry currently registered on this
    /// context.
    ///
    /// This does not execute SQL. Pending inserts, updates and deletes are
    /// discarded from the unit of work represented by the current tracker.
    fn clear_tracker(&self) {
        self.tracking_registry().clear();
    }

    /// Executes the configured SQL Server health check through the current
    /// connection handle.
    fn health_check(&self) -> impl Future<Output = Result<(), OrmError>> + Send {
        let shared_connection = self.shared_connection();

        async move {
            let mut connection = shared_connection.lock().await?;
            connection.health_check().await
        }
    }

    /// Creates a typed raw SQL query.
    ///
    /// Raw SQL is executed exactly as written after ORM parameter handling; it
    /// does not automatically apply tenant or soft-delete filters.
    fn raw<T>(&self, sql: impl Into<String>) -> RawQuery<T>
    where
        T: FromRow + Send,
    {
        RawQuery::new(self.shared_connection(), sql)
    }

    /// Creates a raw SQL command for statements that do not materialize rows.
    fn raw_exec(&self, sql: impl Into<String>) -> RawCommand {
        RawCommand::new(self.shared_connection(), sql)
    }

    /// Executes an operation inside a transaction on a direct shared
    /// connection.
    ///
    /// The closure receives a context bound to the same shared connection and
    /// runtime values. Returning `Ok` commits; returning `Err` rolls back.
    /// Contexts backed by a pool currently return an error because pooled
    /// transactions must pin one physical connection for the full closure.
    fn transaction<F, Fut, T>(
        &self,
        operation: F,
    ) -> impl Future<Output = Result<T, OrmError>> + Send
    where
        F: FnOnce(Self) -> Fut + Send,
        Fut: Future<Output = Result<T, OrmError>> + Send,
        T: Send,
    {
        let shared_connection = self.shared_connection();
        async move {
            shared_connection
                .run_transaction(|transaction_connection| async move {
                    let transaction_context = Self::from_shared_connection(transaction_connection);
                    operation(transaction_context).await
                })
                .await
        }
    }
}

/// Gives generic code access to the `DbSet<E>` declared on a context.
///
/// `#[derive(DbContext)]` implements this for each entity set field.
pub trait DbContextEntitySet<E: Entity>: DbContext {
    /// Returns the typed set for entity `E`.
    fn db_set(&self) -> &DbSet<E>;
}

/// Typed entry point for querying and persisting one entity type.
///
/// `DbSet<E>` is normally declared as a field on a derived `DbContext`. It
/// builds query ASTs, applies runtime policies such as tenant and soft-delete
/// visibility, compiles through the SQL Server crate, and executes through the
/// shared Tiberius connection handle.
#[derive(Clone)]
pub struct DbSet<E: Entity> {
    connection: Option<SharedConnection>,
    tracking_registry: TrackingRegistryHandle,
    _entity: PhantomData<fn() -> E>,
}

impl<E: Entity> DbSet<E> {
    /// Creates a set backed by the given shared connection.
    pub fn new(connection: SharedConnection) -> Self {
        Self::with_tracking_registry(connection, Arc::new(TrackingRegistry::default()))
    }

    #[doc(hidden)]
    pub fn with_tracking_registry(
        connection: SharedConnection,
        tracking_registry: TrackingRegistryHandle,
    ) -> Self {
        Self {
            connection: Some(connection),
            tracking_registry,
            _entity: PhantomData,
        }
    }

    #[cfg(test)]
    pub(crate) fn disconnected() -> Self {
        Self {
            connection: None,
            tracking_registry: Arc::new(TrackingRegistry::default()),
            _entity: PhantomData,
        }
    }

    /// Returns the static metadata generated for entity `E`.
    pub fn entity_metadata(&self) -> &'static EntityMetadata {
        E::metadata()
    }

    /// Starts a query for the full entity.
    ///
    /// Tenant and soft-delete visibility are materialized when the query is
    /// compiled or executed, so callers cannot bypass those policies through
    /// the public query surface.
    pub fn query(&self) -> DbSetQuery<E> {
        DbSetQuery::new(
            self.connection.as_ref().cloned(),
            SelectQuery::from_entity::<E>(),
        )
        .with_tracking_registry(Arc::clone(&self.tracking_registry))
    }

    /// Starts a query from a caller-provided `SelectQuery`.
    ///
    /// This is useful for advanced composition while still routing execution
    /// through `DbSetQuery`, so mandatory tenant and soft-delete behavior can
    /// be applied before SQL compilation.
    pub fn query_with(&self, select_query: SelectQuery) -> DbSetQuery<E> {
        DbSetQuery::new(self.connection.as_ref().cloned(), select_query)
            .with_tracking_registry(Arc::clone(&self.tracking_registry))
    }

    fn query_with_internal_visibility(&self, select_query: SelectQuery) -> DbSetQuery<E> {
        DbSetQuery::new(self.connection.as_ref().cloned(), select_query)
            .with_tracking_registry(Arc::clone(&self.tracking_registry))
            .with_deleted()
    }

    /// Finds one entity by its single-column primary key.
    ///
    /// Composite primary keys are rejected in this stage. Tenant and
    /// soft-delete policies are applied through the normal query path.
    pub async fn find<K>(&self, key: K) -> Result<Option<E>, OrmError>
    where
        E: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
        K: SqlTypeMapping,
    {
        self.query_with(self.find_select_query(key)?).first().await
    }

    /// Loads an entity by its single-column primary key and wraps it in the
    /// snapshot-based tracking container.
    ///
    /// The loaded row is registered in this context's tracker using entity
    /// type, schema, table and primary key value. If that identity already has
    /// a detached registry entry, the returned wrapper reattaches to the
    /// registry-owned snapshot instead of creating a duplicate. The public
    /// first-stable-cut policy allows only one live `Tracked<E>` handle for the
    /// same persisted identity in one context: loading that identity while
    /// another wrapper is still attached returns `OrmError`. Composite primary
    /// keys are rejected with a stable tracking error in the first stable cut.
    /// Included navigation graphs are not registered automatically; use
    /// explicit tracking entry points for every entity that should participate
    /// in `save_changes()`.
    pub async fn find_tracked<K>(&self, key: K) -> Result<Option<Tracked<E>>, OrmError>
    where
        E: Clone + FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
        K: SqlTypeMapping,
    {
        self.ensure_tracking_primary_key_scope()?;

        let key = key.to_sql_value();
        let mut tracked = self
            .query_with(self.find_select_query_sql_value(key.clone())?)
            .first()
            .await
            .map(|entity| entity.map(Tracked::from_loaded))?;

        if let Some(entity) = tracked.as_mut() {
            entity.attach_registry_loaded(Arc::clone(&self.tracking_registry), key)?;
        }

        Ok(tracked)
    }

    /// Registers a new in-memory entity as tracked in `Added`
    /// state so a later `save_changes()` can persist it via `insert`.
    ///
    /// `Added` entries use a temporary identity until persistence. Entities
    /// with composite primary keys can be held in memory, but `save_changes()`
    /// rejects them before executing SQL in the first stable cut. A successful
    /// tracked insert replaces the temporary identity with the persisted
    /// single-column primary key returned by SQL Server. Dropping the returned
    /// wrapper keeps the pending insert in the registry; use explicit detach
    /// or clear APIs to discard it.
    pub fn add_tracked(&self, entity: E) -> Tracked<E>
    where
        E: Clone,
    {
        let mut tracked = Tracked::from_added(entity);
        tracked.attach_registry_added(Arc::clone(&self.tracking_registry));
        tracked
    }

    /// Marks a tracked entity for deletion so a later `save_changes()` can
    /// persist it through the regular delete pipeline.
    ///
    /// Calling this on an `Added` wrapper cancels the pending insert locally:
    /// the wrapper becomes `Deleted` and is detached from the tracker, so no
    /// database delete is issued by a later `save_changes()`. Calling this on
    /// a loaded or modified wrapper marks only that wrapper; relationship
    /// wrappers are not interpreted as cascade instructions.
    pub fn remove_tracked(&self, tracked: &mut Tracked<E>) {
        let was_added = tracked.state() == crate::EntityState::Added;
        tracked.mark_deleted();

        // Deleting an entity that was never inserted should simply cancel the
        // pending tracked insert instead of issuing a database delete.
        if was_added {
            tracked.detach_registry();
        }
    }

    /// Detaches a tracked wrapper from this context's tracker.
    ///
    /// Detach does not execute SQL and does not reset the wrapper state. It
    /// only removes the entry from the context unit of work so later
    /// `save_changes()` calls ignore it.
    pub fn detach_tracked(&self, tracked: &mut Tracked<E>) {
        tracked.detach_registry();
    }

    #[doc(hidden)]
    pub async fn save_tracked_added(&self) -> Result<usize, OrmError>
    where
        E: AuditEntity
            + Clone
            + EntityPersist
            + EntityPrimaryKey
            + FromRow
            + Send
            + TenantScopedEntity,
    {
        let tracked_entities = self.tracking_registry.tracked_for::<E>();
        let has_pending_added = tracked_entities
            .iter()
            .any(|tracked| tracked.state() == crate::EntityState::Added);
        if !has_pending_added {
            return Ok(0);
        }

        self.ensure_tracking_primary_key_scope()?;

        let mut saved = 0;

        for tracked in tracked_entities {
            if tracked.state() != crate::EntityState::Added {
                continue;
            }

            let current: E = tracked.current_clone();
            let persisted = self.insert_entity(&current).await?;
            let persisted_key = persisted.primary_key_value()?;

            tracked.sync_persisted(persisted);
            self.tracking_registry
                .update_persisted_identity::<E>(tracked.registration_id(), persisted_key)?;
            saved += 1;
        }

        Ok(saved)
    }

    #[doc(hidden)]
    pub async fn save_tracked_deleted(&self) -> Result<usize, OrmError>
    where
        E: Clone
            + EntityPersist
            + EntityPrimaryKey
            + FromRow
            + Send
            + SoftDeleteEntity
            + TenantScopedEntity,
    {
        let tracked_entities = self.tracking_registry.tracked_for::<E>();
        let has_pending_deleted = tracked_entities
            .iter()
            .any(|tracked| tracked.state() == crate::EntityState::Deleted);
        if !has_pending_deleted {
            return Ok(0);
        }

        self.ensure_tracking_primary_key_scope()?;

        let mut saved = 0;

        for tracked in tracked_entities {
            if tracked.state() != crate::EntityState::Deleted {
                continue;
            }

            let current: E = tracked.current_clone();
            let key = current.primary_key_value()?;
            let deleted = self
                .delete_tracked_by_sql_value(key, current.concurrency_token()?)
                .await?;

            if !deleted {
                return Err(OrmError::concurrency(
                    "save_changes could not delete a tracked entity for the current primary key",
                ));
            }

            self.tracking_registry.unregister(tracked.registration_id());
            saved += 1;
        }

        Ok(saved)
    }

    #[doc(hidden)]
    pub async fn save_tracked_modified(&self) -> Result<usize, OrmError>
    where
        E: AuditEntity
            + Clone
            + EntityPersist
            + EntityPrimaryKey
            + FromRow
            + Send
            + SoftDeleteEntity
            + TenantScopedEntity,
    {
        let tracked_entities = self.tracking_registry.tracked_for::<E>();
        let has_pending_modified = tracked_entities
            .iter()
            .any(|tracked| tracked.state() == crate::EntityState::Modified);
        if !has_pending_modified {
            return Ok(0);
        }

        self.ensure_tracking_primary_key_scope()?;

        let mut saved = 0;

        for tracked in tracked_entities {
            if tracked.state() != crate::EntityState::Modified {
                continue;
            }

            if !tracked.has_persisted_changes() {
                tracked.accept_current();
                continue;
            }

            let current: E = tracked.current_clone();
            let key = current.primary_key_value()?;
            let persisted = self
                .update_entity_by_sql_value(key, &current, current.concurrency_token()?)
                .await?
                .ok_or_else(|| {
                    OrmError::concurrency(
                        "save_changes could not update a tracked entity for the current primary key",
                    )
                })?;

            tracked.sync_persisted(persisted);
            saved += 1;
        }

        Ok(saved)
    }

    #[doc(hidden)]
    pub async fn save_reconciled_relationship_operations(
        &self,
        plan: &RelationshipReconciliationPlan,
    ) -> Result<usize, OrmError>
    where
        E: AuditEntity
            + Clone
            + EntityPersist
            + EntityPrimaryKey
            + FromRow
            + Send
            + SoftDeleteEntity
            + Sync
            + TenantScopedEntity
            + 'static,
    {
        let tracked_entities = self.tracking_registry.tracked_for::<E>();
        let has_reconciled_operation = tracked_entities.iter().any(|tracked| {
            plan.operations()
                .iter()
                .any(|operation| operation.registration_id() == tracked.registration_id())
        });
        if !has_reconciled_operation {
            return Ok(0);
        }

        self.ensure_tracking_primary_key_scope()?;

        let mut saved = 0;

        for tracked in tracked_entities {
            let Some(operation) = plan
                .operations()
                .iter()
                .find(|operation| operation.registration_id() == tracked.registration_id())
            else {
                continue;
            };

            match operation.kind() {
                ReconciledRelationshipOperationKind::Insert => {
                    if tracked.state() != crate::EntityState::Added {
                        return Err(OrmError::compile(format!(
                            "reconciled relationship insert for entity `{}` requires an Added tracked entry",
                            E::metadata().rust_name
                        )));
                    }

                    let current: E = tracked.current_clone();
                    let values = merge_relationship_values(
                        current.insert_values(),
                        operation.relationship_values(),
                    )?;
                    let persisted = self.insert_entity_values(values).await?;
                    let persisted_key = persisted.primary_key_value()?;

                    tracked.sync_persisted(persisted);
                    self.tracking_registry
                        .update_persisted_identity::<E>(tracked.registration_id(), persisted_key)?;
                    saved += 1;
                }
                ReconciledRelationshipOperationKind::Update => {
                    if tracked.state() != crate::EntityState::Modified {
                        return Err(OrmError::compile(format!(
                            "reconciled relationship update for entity `{}` requires a Modified tracked entry",
                            E::metadata().rust_name
                        )));
                    }

                    let current: E = tracked.current_clone();
                    let key = current.primary_key_value()?;
                    let changes = merge_relationship_values(
                        current.update_changes(),
                        operation.relationship_values(),
                    )?;
                    let persisted = self
                        .update_entity_values_by_sql_value(
                            key,
                            changes,
                            current.concurrency_token()?,
                        )
                        .await?
                        .ok_or_else(|| {
                            OrmError::concurrency(
                                "save_changes could not update a tracked entity for the current primary key",
                            )
                        })?;

                    tracked.sync_persisted(persisted);
                    saved += 1;
                }
                ReconciledRelationshipOperationKind::Delete => {
                    if tracked.state() != crate::EntityState::Deleted {
                        return Err(OrmError::compile(format!(
                            "reconciled relationship delete for entity `{}` requires a Deleted tracked entry",
                            E::metadata().rust_name
                        )));
                    }

                    let current: E = tracked.current_clone();
                    let key = current.primary_key_value()?;
                    let deleted = self
                        .delete_tracked_by_sql_value(key, current.concurrency_token()?)
                        .await?;

                    if !deleted {
                        return Err(OrmError::concurrency(
                            "save_changes could not delete a tracked entity for the current primary key",
                        ));
                    }

                    self.tracking_registry.unregister(tracked.registration_id());
                    saved += 1;
                }
            }
        }

        Ok(saved)
    }

    #[doc(hidden)]
    pub fn collect_has_many_relationship_commands(
        &self,
        context_entities: &[&'static EntityMetadata],
    ) -> Result<Vec<crate::RelationshipCommand>, OrmError>
    where
        E: Clone + EntityPrimaryKey + RelationshipMutationSource + Send + Sync + 'static,
    {
        let mut commands = Vec::new();

        for tracked in self.tracking_registry.tracked_for::<E>() {
            let current = tracked.current_clone();
            let batches = current.relationship_change_batches();
            if batches
                .iter()
                .any(|batch| batch.navigation().kind == NavigationKind::HasMany)
                && tracked.state() == crate::EntityState::Added
            {
                return Err(OrmError::compile(format!(
                    "save_changes cannot persist has_many relationship mutations for Added principal `{}` before its primary key is persisted",
                    E::metadata().rust_name
                )));
            }

            for batch in batches {
                if batch.navigation().kind != NavigationKind::HasMany {
                    continue;
                }

                let context = has_many_relationship_command_context::<E>(
                    tracked.registration_id(),
                    &current,
                    batch.navigation(),
                    context_entities,
                )?;
                commands.extend(relationship_commands_from_batch(&batch, &context)?);
            }
        }

        Ok(commands)
    }

    #[doc(hidden)]
    pub fn collect_dependent_relationship_commands<P>(
        &self,
        context_entities: &[&'static EntityMetadata],
    ) -> Result<Vec<crate::RelationshipCommand>, OrmError>
    where
        E: Clone + RelationshipMutationSource + Send + Sync + 'static,
        P: Clone + EntityPrimaryKey + Send + Sync + 'static,
    {
        let principal_metadata = P::metadata();
        let mut principal_values = None::<Vec<RelationshipPrincipalValues>>;
        let mut commands = Vec::new();

        for tracked in self.tracking_registry.tracked_for::<E>() {
            let current = tracked.current_clone();

            for batch in current.relationship_change_batches() {
                if !matches!(
                    batch.navigation().kind,
                    NavigationKind::BelongsTo | NavigationKind::HasOne
                ) || batch.navigation().target_schema != principal_metadata.schema
                    || batch.navigation().target_table != principal_metadata.table
                    || batch.navigation().target_rust_name != principal_metadata.rust_name
                {
                    continue;
                }

                let principal_values = match &principal_values {
                    Some(principal_values) => principal_values.clone(),
                    None => {
                        let dependent_column = dependent_relationship_column::<E>(
                            batch.navigation(),
                            principal_metadata,
                            context_entities,
                        )?;
                        let values = self
                            .tracking_registry
                            .registered_current_values_for_metadata::<P>(principal_metadata)
                            .into_iter()
                            .map(|(registration_id, principal)| {
                                Ok(RelationshipPrincipalValues::new(
                                    registration_id,
                                    vec![ColumnValue::new(
                                        dependent_column,
                                        principal.primary_key_value()?,
                                    )],
                                ))
                            })
                            .collect::<Result<Vec<_>, OrmError>>()?;
                        principal_values = Some(values.clone());
                        values
                    }
                };
                let context = dependent_relationship_command_context::<E>(
                    tracked.registration_id(),
                    batch.navigation(),
                    principal_metadata,
                    context_entities,
                    principal_values,
                )?;
                commands.extend(relationship_commands_from_batch(&batch, &context)?);
            }
        }

        Ok(commands)
    }

    #[doc(hidden)]
    pub fn reject_pending_relationship_changes(&self) -> Result<(), OrmError>
    where
        E: Clone + RelationshipMutationSource + Send + Sync + 'static,
    {
        for tracked in self.tracking_registry.tracked_for::<E>() {
            let current = tracked.current_clone();
            if !current.relationship_change_batches().is_empty() {
                return Err(OrmError::compile(format!(
                    "save_changes cannot persist relationship wrapper mutations for entity `{}` yet because relationship command collection is not connected to execution",
                    E::metadata().rust_name
                )));
            }
        }

        Ok(())
    }

    #[doc(hidden)]
    pub fn reject_pending_unsupported_relationship_changes(&self) -> Result<(), OrmError>
    where
        E: Clone + RelationshipMutationSource + Send + Sync + 'static,
    {
        for tracked in self.tracking_registry.tracked_for::<E>() {
            let current = tracked.current_clone();
            if current.relationship_change_batches().iter().any(|batch| {
                !matches!(
                    batch.navigation().kind,
                    NavigationKind::HasMany | NavigationKind::BelongsTo | NavigationKind::HasOne
                )
            }) {
                return Err(OrmError::compile(format!(
                    "save_changes cannot persist unsupported relationship wrapper mutations for entity `{}` yet",
                    E::metadata().rust_name
                )));
            }
        }

        Ok(())
    }

    #[doc(hidden)]
    pub fn clear_tracked_relationship_changes(&self)
    where
        E: Clone + RelationshipMutationSource + Send + Sync + 'static,
    {
        for tracked in self.tracking_registry.tracked_for::<E>() {
            let mut current = tracked.current_clone();
            if current.pending_relationship_change_count() == 0 {
                continue;
            }
            current.clear_relationship_changes();
            tracked.replace_current_snapshot(current);
        }
    }

    /// Inserts a new row and materializes the inserted entity.
    ///
    /// The insert path applies tenant insert fill/validation and audit runtime
    /// values for entities that opt into those policies.
    pub async fn insert<I>(&self, insertable: I) -> Result<E, OrmError>
    where
        E: AuditEntity + FromRow + Send + TenantScopedEntity,
        I: Insertable<E>,
    {
        let compiled = SqlServerCompiler::compile_insert(&self.insert_query(&insertable)?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        let inserted = connection.fetch_one(compiled).await?;

        inserted.ok_or_else(|| OrmError::mapping("insert query did not return a row"))
    }

    /// Updates one row by single-column primary key and materializes the
    /// updated entity when a row matched.
    ///
    /// Rowversion mismatches are surfaced as `OrmError::ConcurrencyConflict`
    /// when the entity still exists. Tenant and audit policies are applied by
    /// the shared update pipeline.
    pub async fn update<K, C>(&self, key: K, changeset: C) -> Result<Option<E>, OrmError>
    where
        E: AuditEntity + FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
        K: SqlTypeMapping,
        C: Changeset<E>,
    {
        let key = key.to_sql_value();
        let concurrency_token = changeset.concurrency_token()?;
        let compiled = SqlServerCompiler::compile_update(&self.update_query_sql_value_audited(
            key.clone(),
            changeset.changes(),
            concurrency_token.clone(),
        )?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        let updated = connection.fetch_one(compiled).await?;
        drop(connection);

        if updated.is_none()
            && concurrency_token.is_some()
            && self.exists_by_sql_value_internal(key).await?
        {
            return Err(OrmError::concurrency_conflict());
        }

        Ok(updated)
    }

    /// Deletes one row by single-column primary key.
    ///
    /// Entities with `soft_delete` emit an `UPDATE` through the soft-delete
    /// pipeline; other entities emit a physical `DELETE`. The return value is
    /// `true` when a row was affected.
    pub async fn delete<K>(&self, key: K) -> Result<bool, OrmError>
    where
        E: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
        K: SqlTypeMapping,
    {
        self.delete_by_sql_value(key.to_sql_value(), None).await
    }

    pub(crate) async fn delete_by_sql_value(
        &self,
        key: SqlValue,
        concurrency_token: Option<SqlValue>,
    ) -> Result<bool, OrmError>
    where
        E: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        let shared_connection = self.require_connection()?;
        let soft_delete_provider = shared_connection.soft_delete_provider();
        let soft_delete_request_values = shared_connection.soft_delete_request_values();
        let compiled = self.delete_compiled_query_sql_value(
            key.clone(),
            concurrency_token.clone(),
            soft_delete_provider.as_deref(),
            soft_delete_request_values.as_deref(),
        )?;
        let mut connection = shared_connection.lock().await?;
        let result = connection.execute(compiled).await?;
        let deleted = result.total() > 0;

        drop(connection);

        if !deleted && concurrency_token.is_some() && self.exists_by_sql_value_internal(key).await?
        {
            return Err(OrmError::concurrency_conflict());
        }

        Ok(deleted)
    }

    pub(crate) async fn delete_tracked_by_sql_value(
        &self,
        key: SqlValue,
        concurrency_token: Option<SqlValue>,
    ) -> Result<bool, OrmError>
    where
        E: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        self.delete_by_sql_value(key, concurrency_token).await
    }

    async fn find_by_sql_value_internal(&self, key: SqlValue) -> Result<Option<E>, OrmError>
    where
        E: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        self.query_with_internal_visibility(self.find_select_query_sql_value(key)?)
            .first()
            .await
    }

    pub(crate) async fn exists_by_sql_value_internal(&self, key: SqlValue) -> Result<bool, OrmError>
    where
        E: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        Ok(self.find_by_sql_value_internal(key).await?.is_some())
    }

    pub(crate) async fn insert_entity_values(
        &self,
        values: Vec<sql_orm_core::ColumnValue>,
    ) -> Result<E, OrmError>
    where
        E: AuditEntity + FromRow + Send + TenantScopedEntity,
    {
        let compiled = SqlServerCompiler::compile_insert(&self.insert_query_values(values)?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        let inserted = connection.fetch_one(compiled).await?;

        inserted.ok_or_else(|| OrmError::mapping("insert query did not return a row"))
    }

    pub(crate) async fn insert_entity(&self, entity: &E) -> Result<E, OrmError>
    where
        E: AuditEntity + EntityPersist + FromRow + Send + TenantScopedEntity,
    {
        self.insert_entity_values(entity.insert_values()).await
    }

    pub(crate) async fn update_entity_values_by_sql_value(
        &self,
        key: SqlValue,
        changes: Vec<sql_orm_core::ColumnValue>,
        concurrency_token: Option<SqlValue>,
    ) -> Result<Option<E>, OrmError>
    where
        E: AuditEntity + FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        let compiled = SqlServerCompiler::compile_update(&self.update_query_sql_value_audited(
            key.clone(),
            changes,
            concurrency_token.clone(),
        )?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        let updated = connection.fetch_one(compiled).await?;
        drop(connection);

        if updated.is_none()
            && concurrency_token.is_some()
            && self.exists_by_sql_value_internal(key).await?
        {
            return Err(OrmError::concurrency_conflict());
        }

        Ok(updated)
    }

    pub(crate) async fn update_entity_by_sql_value(
        &self,
        key: SqlValue,
        entity: &E,
        concurrency_token: Option<SqlValue>,
    ) -> Result<Option<E>, OrmError>
    where
        E: AuditEntity + EntityPersist + FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        self.update_entity_values_by_sql_value(key, entity.update_changes(), concurrency_token)
            .await
    }

    /// Returns the shared connection handle backing this set.
    pub fn shared_connection(&self) -> SharedConnection {
        self.connection
            .as_ref()
            .expect("DbSet requires an initialized shared connection")
            .clone()
    }

    /// Explicitly loads a `has_many` collection navigation into an already
    /// materialized entity.
    ///
    /// This performs I/O only at this call site. It does not install lazy
    /// loading behavior on the entity or navigation field.
    pub async fn load_collection<J>(
        &self,
        entity: &mut E,
        navigation: &'static str,
    ) -> Result<(), OrmError>
    where
        E: EntityPrimaryKey + IncludeCollection<J>,
        J: Clone
            + EntityPrimaryKey
            + FromRow
            + Send
            + SoftDeleteEntity
            + Sync
            + TenantScopedEntity
            + 'static,
    {
        let related = self
            .explicit_collection_query::<J>(entity, navigation)?
            .all()
            .await?;
        let related = self.identity_mapped_navigation_values(related)?;
        entity.set_included_collection(navigation, related)
    }

    /// Explicitly loads a `has_many` collection navigation into a tracked
    /// entity without marking it as modified.
    pub async fn load_collection_tracked<J>(
        &self,
        tracked: &mut Tracked<E>,
        navigation: &'static str,
    ) -> Result<(), OrmError>
    where
        E: EntityPrimaryKey + IncludeCollection<J>,
        J: Clone
            + EntityPrimaryKey
            + FromRow
            + Send
            + SoftDeleteEntity
            + Sync
            + TenantScopedEntity
            + 'static,
    {
        let related = self
            .explicit_collection_query::<J>(tracked.current(), navigation)?
            .all()
            .await?;
        let related = self.identity_mapped_navigation_values(related)?;
        tracked
            .current_mut_without_state_change()
            .set_included_collection(navigation, related)
    }

    fn identity_mapped_navigation_values<J>(&self, values: Vec<J>) -> Result<Vec<J>, OrmError>
    where
        J: Entity + EntityPrimaryKey + Clone + Send + Sync + 'static,
    {
        values
            .into_iter()
            .map(|value| {
                let key = value.primary_key_value()?;
                Ok(self
                    .tracking_registry
                    .current_snapshot_for_key::<J>(key)
                    .unwrap_or(value))
            })
            .collect()
    }

    #[doc(hidden)]
    pub fn tracking_registry(&self) -> TrackingRegistryHandle {
        Arc::clone(&self.tracking_registry)
    }

    fn require_connection(&self) -> Result<SharedConnection, OrmError> {
        self.connection
            .as_ref()
            .cloned()
            .ok_or_else(|| OrmError::execution("DbSet requires an initialized shared connection"))
    }

    fn active_tenant(&self) -> Option<ActiveTenant> {
        self.connection
            .as_ref()
            .and_then(SharedConnection::active_tenant)
    }

    fn explicit_collection_query<J>(
        &self,
        entity: &E,
        navigation: &'static str,
    ) -> Result<DbSetQuery<J>, OrmError>
    where
        E: EntityPrimaryKey,
        J: Entity,
    {
        let navigation_metadata = E::metadata().navigation(navigation).ok_or_else(|| {
            OrmError::compile(format!(
                "entity `{}` does not declare navigation `{}`",
                E::metadata().rust_name,
                navigation
            ))
        })?;

        if navigation_metadata.kind != NavigationKind::HasMany {
            return Err(OrmError::compile(format!(
                "explicit collection loading only supports has_many navigations; `{}` is {:?}",
                navigation_metadata.rust_field, navigation_metadata.kind
            )));
        }

        if navigation_metadata.local_columns.len() != 1
            || navigation_metadata.target_columns.len() != 1
        {
            return Err(OrmError::compile(
                "explicit collection loading currently supports only single-column navigation joins",
            ));
        }

        let root_primary_key = E::metadata().primary_key.columns;
        if root_primary_key.len() != 1
            || root_primary_key[0] != navigation_metadata.local_columns[0]
        {
            return Err(OrmError::compile(
                "explicit collection loading requires the has_many local column to be the root entity single-column primary key",
            ));
        }

        let target_metadata = J::metadata();
        if navigation_metadata.target_schema != target_metadata.schema
            || navigation_metadata.target_table != target_metadata.table
        {
            return Err(OrmError::compile(format!(
                "navigation `{}` on `{}` targets `{}.{}`, not entity `{}` (`{}.{}`)",
                navigation_metadata.rust_field,
                E::metadata().rust_name,
                navigation_metadata.target_schema,
                navigation_metadata.target_table,
                target_metadata.rust_name,
                target_metadata.schema,
                target_metadata.table
            )));
        }

        let target_column = target_metadata
            .column(navigation_metadata.target_columns[0])
            .ok_or_else(|| {
                OrmError::compile(format!(
                    "entity `{}` metadata does not contain column `{}` required by explicit collection loading",
                    target_metadata.rust_name, navigation_metadata.target_columns[0]
                ))
            })?;

        let key = entity.primary_key_value()?;
        Ok(DbSetQuery::new(
            self.connection.as_ref().cloned(),
            SelectQuery::from_entity::<J>().filter(Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::for_entity::<J>(),
                    target_column.rust_field,
                    target_column.column_name,
                )),
                Expr::Value(key),
            )),
        ))
        .map(|query| query.with_tracking_registry(Arc::clone(&self.tracking_registry)))
    }
}

impl<E: Entity> std::fmt::Debug for DbSet<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DbSet")
            .field("entity", &E::metadata().rust_name)
            .field("table", &E::metadata().table)
            .finish()
    }
}

#[allow(dead_code)]
fn merge_relationship_values(
    mut entity_values: Vec<ColumnValue>,
    relationship_values: &[ColumnValue],
) -> Result<Vec<ColumnValue>, OrmError> {
    for relationship_value in relationship_values {
        if let Some(existing_value) = entity_values
            .iter()
            .find(|value| value.column_name == relationship_value.column_name)
        {
            if existing_value.value != relationship_value.value {
                return Err(OrmError::compile(format!(
                    "relationship operation assigns column `{}` but the tracked entity already has a different value",
                    relationship_value.column_name
                )));
            }
            continue;
        }

        entity_values.push(relationship_value.clone());
    }

    Ok(entity_values)
}

fn has_many_relationship_command_context<E>(
    principal_registration_id: usize,
    principal: &E,
    navigation: &NavigationMetadata,
    context_entities: &[&'static EntityMetadata],
) -> Result<RelationshipCommandBatchContext, OrmError>
where
    E: EntityPrimaryKey,
{
    if navigation.kind != NavigationKind::HasMany {
        return Err(OrmError::compile(
            "has_many relationship command context requires a has_many navigation",
        ));
    }

    if navigation.local_columns.len() != 1 || navigation.target_columns.len() != 1 {
        return Err(OrmError::compile(
            "has_many relationship command context currently supports only simple foreign keys",
        ));
    }

    let target_metadata = context_entities
        .iter()
        .copied()
        .find(|metadata| {
            metadata.schema == navigation.target_schema
                && metadata.table == navigation.target_table
                && metadata.rust_name == navigation.target_rust_name
        })
        .ok_or_else(|| {
            OrmError::compile(format!(
                "has_many relationship `{}` targets entity `{}` which is not part of this DbContext",
                navigation.rust_field, navigation.target_rust_name
            ))
        })?;

    if let Some(foreign_key_name) = navigation.foreign_key_name {
        let foreign_key = target_metadata.foreign_key(foreign_key_name).ok_or_else(|| {
            OrmError::compile(format!(
                "has_many relationship `{}` references foreign key `{}` missing from target entity `{}`",
                navigation.rust_field, foreign_key_name, target_metadata.rust_name
            ))
        })?;

        if foreign_key.columns != navigation.target_columns
            || foreign_key.referenced_columns != navigation.local_columns
        {
            return Err(OrmError::compile(format!(
                "has_many relationship `{}` foreign key metadata does not match navigation columns",
                navigation.rust_field
            )));
        }
    }

    let target_column_name = navigation.target_columns[0];
    let target_column = target_metadata.column(target_column_name).ok_or_else(|| {
        OrmError::compile(format!(
            "has_many relationship `{}` target column `{}` was not found on `{}`",
            navigation.rust_field, target_column_name, target_metadata.rust_name
        ))
    })?;
    let principal_key = principal.primary_key_value()?;

    Ok(RelationshipCommandBatchContext::for_principal(
        principal_registration_id,
        vec![ColumnValue::new(target_column.column_name, principal_key)],
        vec![ColumnValue::new(target_column.column_name, SqlValue::Null)],
        !target_column.nullable,
    ))
}

fn dependent_relationship_command_context<E>(
    dependent_registration_id: usize,
    navigation: &NavigationMetadata,
    principal_metadata: &'static EntityMetadata,
    context_entities: &[&'static EntityMetadata],
    principal_values: Vec<RelationshipPrincipalValues>,
) -> Result<RelationshipCommandBatchContext, OrmError>
where
    E: Entity,
{
    if !matches!(
        navigation.kind,
        NavigationKind::BelongsTo | NavigationKind::HasOne
    ) {
        return Err(OrmError::compile(
            "dependent relationship command context requires belongs_to or has_one navigation",
        ));
    }

    let dependent_column =
        dependent_relationship_column::<E>(navigation, principal_metadata, context_entities)?;
    let dependent_metadata = context_entity_metadata::<E>(context_entities)?;
    let dependent_column_metadata =
        dependent_metadata.column(dependent_column).ok_or_else(|| {
            OrmError::compile(format!(
                "dependent relationship `{}` column `{}` was not found on `{}`",
                navigation.rust_field, dependent_column, dependent_metadata.rust_name
            ))
        })?;

    Ok(RelationshipCommandBatchContext::for_dependent(
        dependent_registration_id,
        principal_values,
        vec![ColumnValue::new(dependent_column, SqlValue::Null)],
        !dependent_column_metadata.nullable,
    ))
}

fn dependent_relationship_column<E>(
    navigation: &NavigationMetadata,
    principal_metadata: &'static EntityMetadata,
    context_entities: &[&'static EntityMetadata],
) -> Result<&'static str, OrmError>
where
    E: Entity,
{
    let dependent_metadata = context_entity_metadata::<E>(context_entities)?;
    let foreign_key = if let Some(foreign_key_name) = navigation.foreign_key_name {
        dependent_metadata
            .foreign_key(foreign_key_name)
            .ok_or_else(|| {
                OrmError::compile(format!(
                    "dependent relationship `{}` references foreign key `{}` missing from entity `{}`",
                    navigation.rust_field, foreign_key_name, dependent_metadata.rust_name
                ))
            })?
    } else {
        let matching_foreign_keys = dependent_metadata
            .foreign_keys_referencing(principal_metadata.schema, principal_metadata.table);
        match matching_foreign_keys.as_slice() {
            [foreign_key] => *foreign_key,
            [] => {
                return Err(OrmError::compile(format!(
                    "dependent entity `{}` has no foreign key referencing principal `{}`",
                    dependent_metadata.rust_name, principal_metadata.rust_name
                )));
            }
            _ => {
                return Err(OrmError::compile(format!(
                    "dependent entity `{}` has multiple foreign keys referencing principal `{}`; dependent-side relationship collection requires an unambiguous foreign key",
                    dependent_metadata.rust_name, principal_metadata.rust_name
                )));
            }
        }
    };

    if foreign_key.columns.len() != 1 || foreign_key.referenced_columns.len() != 1 {
        return Err(OrmError::compile(
            "dependent relationship command context currently supports only simple foreign keys",
        ));
    }

    if !foreign_key.references_table(principal_metadata.schema, principal_metadata.table) {
        return Err(OrmError::compile(format!(
            "dependent relationship `{}` foreign key does not reference principal `{}`",
            navigation.rust_field, principal_metadata.rust_name
        )));
    }

    if foreign_key.columns != navigation.local_columns
        || foreign_key.referenced_columns != navigation.target_columns
    {
        return Err(OrmError::compile(format!(
            "dependent relationship `{}` foreign key metadata does not match navigation columns",
            navigation.rust_field
        )));
    }

    Ok(foreign_key.columns[0])
}

fn context_entity_metadata<E>(
    context_entities: &[&'static EntityMetadata],
) -> Result<&'static EntityMetadata, OrmError>
where
    E: Entity,
{
    let metadata = E::metadata();
    context_entities
        .iter()
        .copied()
        .find(|candidate| {
            candidate.schema == metadata.schema
                && candidate.table == metadata.table
                && candidate.rust_name == metadata.rust_name
        })
        .ok_or_else(|| {
            OrmError::compile(format!(
                "entity `{}` is not part of this DbContext",
                metadata.rust_name
            ))
        })
}

impl<E: Entity> DbSet<E> {
    fn find_select_query<K>(&self, key: K) -> Result<SelectQuery, OrmError>
    where
        K: SqlTypeMapping,
    {
        Ok(SelectQuery::from_entity::<E>().filter(self.primary_key_predicate(key)?))
    }

    fn find_select_query_sql_value(&self, key: SqlValue) -> Result<SelectQuery, OrmError> {
        Ok(SelectQuery::from_entity::<E>().filter(self.primary_key_predicate_value(key)?))
    }

    fn insert_query<I>(&self, insertable: &I) -> Result<InsertQuery, OrmError>
    where
        E: AuditEntity + TenantScopedEntity,
        I: Insertable<E>,
    {
        self.insert_query_values(insertable.values())
    }

    fn insert_query_values(
        &self,
        values: Vec<sql_orm_core::ColumnValue>,
    ) -> Result<InsertQuery, OrmError>
    where
        E: AuditEntity + TenantScopedEntity,
    {
        let active_tenant = self.active_tenant();
        let audit_provider = self
            .connection
            .as_ref()
            .and_then(SharedConnection::audit_provider);
        let audit_request_values = self
            .connection
            .as_ref()
            .and_then(SharedConnection::audit_request_values);
        let values = apply_audit_values::<E>(
            AuditOperation::Insert,
            values,
            audit_provider.as_deref(),
            audit_request_values.as_deref(),
        )?;
        let values = self.tenant_insert_values(values, active_tenant.as_ref())?;
        Ok(InsertQuery::for_entity::<E, _>(&RawInsertable(values)))
    }

    #[cfg(test)]
    fn insert_query_values_with_runtime_for_test(
        &self,
        values: Vec<sql_orm_core::ColumnValue>,
        audit_provider: Option<&dyn AuditProvider>,
        audit_request_values: Option<&AuditRequestValues>,
    ) -> Result<InsertQuery, OrmError>
    where
        E: AuditEntity + TenantScopedEntity,
    {
        let active_tenant = self.active_tenant();
        let values = apply_audit_values::<E>(
            AuditOperation::Insert,
            values,
            audit_provider,
            audit_request_values,
        )?;
        let values = self.tenant_insert_values(values, active_tenant.as_ref())?;
        Ok(InsertQuery::for_entity::<E, _>(&RawInsertable(values)))
    }

    fn tenant_insert_values(
        &self,
        mut values: Vec<sql_orm_core::ColumnValue>,
        active_tenant: Option<&ActiveTenant>,
    ) -> Result<Vec<sql_orm_core::ColumnValue>, OrmError>
    where
        E: TenantScopedEntity,
    {
        let Some(policy) = E::tenant_policy() else {
            return Ok(values);
        };

        if policy.columns.len() != 1 {
            return Err(OrmError::compile(
                "tenant insert requires exactly one tenant policy column",
            ));
        }

        let tenant_column = &policy.columns[0];
        let active_tenant = active_tenant.ok_or_else(|| {
            OrmError::execution("tenant-scoped insert requires an active tenant in the DbContext")
        })?;

        if active_tenant.column_name != tenant_column.column_name {
            return Err(OrmError::compile(format!(
                "active tenant column `{}` does not match entity tenant column `{}`",
                active_tenant.column_name, tenant_column.column_name
            )));
        }

        if !tenant_value_matches_column_type(&active_tenant.value, tenant_column) {
            return Err(OrmError::compile(format!(
                "active tenant value is not compatible with entity tenant column `{}`",
                tenant_column.column_name
            )));
        }

        let mut tenant_value_position = None;
        for (index, value) in values.iter().enumerate() {
            if value.column_name == tenant_column.column_name {
                if tenant_value_position.is_some() {
                    return Err(OrmError::compile(format!(
                        "tenant-scoped insert contains duplicate tenant column `{}`",
                        tenant_column.column_name
                    )));
                }

                tenant_value_position = Some(index);
            }
        }

        if let Some(index) = tenant_value_position {
            if values[index].value != active_tenant.value {
                return Err(OrmError::compile(format!(
                    "tenant-scoped insert value for column `{}` does not match the active tenant",
                    tenant_column.column_name
                )));
            }

            return Ok(values);
        }

        values.push(sql_orm_core::ColumnValue::new(
            tenant_column.column_name,
            active_tenant.value.clone(),
        ));
        Ok(values)
    }

    #[cfg(test)]
    fn update_query<K, C>(&self, key: K, changeset: &C) -> Result<UpdateQuery, OrmError>
    where
        E: TenantScopedEntity,
        K: SqlTypeMapping,
        C: Changeset<E>,
    {
        let active_tenant = self.active_tenant();
        let mut query =
            UpdateQuery::for_entity::<E, C>(changeset).filter(self.primary_key_predicate(key)?);

        if let Some(predicate) = self.tenant_write_predicate(active_tenant.as_ref())? {
            query = query.filter(predicate);
        }

        if let Some(token) = changeset.concurrency_token()? {
            query = query.filter(self.rowversion_predicate_value(token)?);
        }

        Ok(query)
    }

    fn update_query_sql_value_audited(
        &self,
        key: SqlValue,
        changes: Vec<sql_orm_core::ColumnValue>,
        concurrency_token: Option<SqlValue>,
    ) -> Result<UpdateQuery, OrmError>
    where
        E: AuditEntity + TenantScopedEntity,
    {
        let active_tenant = self.active_tenant();
        let audit_provider = self
            .connection
            .as_ref()
            .and_then(SharedConnection::audit_provider);
        let audit_request_values = self
            .connection
            .as_ref()
            .and_then(SharedConnection::audit_request_values);

        self.update_query_sql_value_with_audit_runtime(
            key,
            changes,
            concurrency_token,
            active_tenant.as_ref(),
            audit_provider.as_deref(),
            audit_request_values.as_deref(),
        )
    }

    fn update_query_sql_value_with_audit_runtime(
        &self,
        key: SqlValue,
        changes: Vec<sql_orm_core::ColumnValue>,
        concurrency_token: Option<SqlValue>,
        active_tenant: Option<&ActiveTenant>,
        audit_provider: Option<&dyn AuditProvider>,
        audit_request_values: Option<&AuditRequestValues>,
    ) -> Result<UpdateQuery, OrmError>
    where
        E: AuditEntity + TenantScopedEntity,
    {
        let changes = apply_audit_values::<E>(
            AuditOperation::Update,
            changes,
            audit_provider,
            audit_request_values,
        )?;

        self.update_query_sql_value_with_active_tenant(
            key,
            changes,
            concurrency_token,
            active_tenant,
        )
    }

    fn update_query_sql_value_with_active_tenant(
        &self,
        key: SqlValue,
        changes: Vec<sql_orm_core::ColumnValue>,
        concurrency_token: Option<SqlValue>,
        active_tenant: Option<&ActiveTenant>,
    ) -> Result<UpdateQuery, OrmError>
    where
        E: TenantScopedEntity,
    {
        let mut query = UpdateQuery::for_entity::<E, _>(&RawChangeset(changes))
            .filter(self.primary_key_predicate_value(key)?);

        if let Some(predicate) = self.tenant_write_predicate(active_tenant)? {
            query = query.filter(predicate);
        }

        if let Some(token) = concurrency_token {
            query = query.filter(self.rowversion_predicate_value(token)?);
        }

        Ok(query)
    }

    #[cfg(test)]
    fn delete_query<K>(&self, key: K) -> Result<DeleteQuery, OrmError>
    where
        E: TenantScopedEntity,
        K: SqlTypeMapping,
    {
        let active_tenant = self.active_tenant();
        let mut query = DeleteQuery::from_entity::<E>().filter(self.primary_key_predicate(key)?);

        if let Some(predicate) = self.tenant_write_predicate(active_tenant.as_ref())? {
            query = query.filter(predicate);
        }

        Ok(query)
    }

    #[cfg(test)]
    fn delete_query_sql_value(
        &self,
        key: SqlValue,
        concurrency_token: Option<SqlValue>,
    ) -> Result<DeleteQuery, OrmError>
    where
        E: TenantScopedEntity,
    {
        let active_tenant = self.active_tenant();
        self.delete_query_sql_value_with_active_tenant(
            key,
            concurrency_token,
            active_tenant.as_ref(),
        )
    }

    fn delete_query_sql_value_with_active_tenant(
        &self,
        key: SqlValue,
        concurrency_token: Option<SqlValue>,
        active_tenant: Option<&ActiveTenant>,
    ) -> Result<DeleteQuery, OrmError>
    where
        E: TenantScopedEntity,
    {
        let mut query =
            DeleteQuery::from_entity::<E>().filter(self.primary_key_predicate_value(key)?);

        if let Some(predicate) = self.tenant_write_predicate(active_tenant)? {
            query = query.filter(predicate);
        }

        if let Some(token) = concurrency_token {
            query = query.filter(self.rowversion_predicate_value(token)?);
        }

        Ok(query)
    }

    fn delete_compiled_query_sql_value(
        &self,
        key: SqlValue,
        concurrency_token: Option<SqlValue>,
        soft_delete_provider: Option<&dyn SoftDeleteProvider>,
        request_values: Option<&SoftDeleteRequestValues>,
    ) -> Result<sql_orm_query::CompiledQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
    {
        let active_tenant = self.active_tenant();
        self.delete_compiled_query_sql_value_with_active_tenant(
            key,
            concurrency_token,
            soft_delete_provider,
            request_values,
            active_tenant.as_ref(),
        )
    }

    fn delete_compiled_query_sql_value_with_active_tenant(
        &self,
        key: SqlValue,
        concurrency_token: Option<SqlValue>,
        soft_delete_provider: Option<&dyn SoftDeleteProvider>,
        request_values: Option<&SoftDeleteRequestValues>,
        active_tenant: Option<&ActiveTenant>,
    ) -> Result<sql_orm_query::CompiledQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
    {
        if E::soft_delete_policy().is_some() {
            let changes = apply_soft_delete_values::<E>(
                SoftDeleteOperation::Delete,
                Vec::new(),
                soft_delete_provider,
                request_values,
            )?;

            if changes.is_empty() {
                return Err(OrmError::compile(
                    "soft_delete delete requires at least one runtime change",
                ));
            }

            SqlServerCompiler::compile_update(&self.update_query_sql_value_with_active_tenant(
                key,
                changes,
                concurrency_token,
                active_tenant,
            )?)
        } else {
            SqlServerCompiler::compile_delete(&self.delete_query_sql_value_with_active_tenant(
                key,
                concurrency_token,
                active_tenant,
            )?)
        }
    }

    fn tenant_write_predicate(
        &self,
        active_tenant: Option<&ActiveTenant>,
    ) -> Result<Option<Predicate>, OrmError>
    where
        E: TenantScopedEntity,
    {
        let Some(policy) = E::tenant_policy() else {
            return Ok(None);
        };

        if policy.columns.len() != 1 {
            return Err(OrmError::compile(
                "tenant write filter requires exactly one tenant policy column",
            ));
        }

        let tenant_column = &policy.columns[0];
        let active_tenant = active_tenant.ok_or_else(|| {
            OrmError::execution("tenant-scoped write requires an active tenant in the DbContext")
        })?;

        if active_tenant.column_name != tenant_column.column_name {
            return Err(OrmError::compile(format!(
                "active tenant column `{}` does not match entity tenant column `{}`",
                active_tenant.column_name, tenant_column.column_name
            )));
        }

        if !tenant_value_matches_column_type(&active_tenant.value, tenant_column) {
            return Err(OrmError::compile(format!(
                "active tenant value is not compatible with entity tenant column `{}`",
                tenant_column.column_name
            )));
        }

        Ok(Some(Predicate::eq(
            Expr::Column(ColumnRef::new(
                TableRef::for_entity::<E>(),
                tenant_column.rust_field,
                tenant_column.column_name,
            )),
            Expr::Value(active_tenant.value.clone()),
        )))
    }

    fn primary_key_predicate<K>(&self, key: K) -> Result<Predicate, OrmError>
    where
        K: SqlTypeMapping,
    {
        self.primary_key_predicate_value(key.to_sql_value())
    }

    fn primary_key_predicate_value(&self, key: SqlValue) -> Result<Predicate, OrmError> {
        let metadata = E::metadata();
        let primary_key = metadata.primary_key_columns();

        if primary_key.len() != 1 {
            return Err(OrmError::compile(
                "DbSet currently supports this operation only for entities with a single primary key column",
            ));
        }

        let column = primary_key[0];

        Ok(Predicate::eq(
            Expr::Column(ColumnRef::new(
                TableRef::for_entity::<E>(),
                column.rust_field,
                column.column_name,
            )),
            Expr::Value(key),
        ))
    }

    fn ensure_tracking_primary_key_scope(&self) -> Result<(), OrmError> {
        if E::metadata().primary_key_columns().len() == 1 {
            return Ok(());
        }

        Err(OrmError::compile(
            "change tracking currently supports only entities with a single primary key column",
        ))
    }

    fn rowversion_predicate_value(&self, token: SqlValue) -> Result<Predicate, OrmError> {
        let metadata = E::metadata();
        let column = metadata.rowversion_column().ok_or_else(|| {
            OrmError::compile("DbSet concurrency checks require an entity rowversion column")
        })?;

        Ok(Predicate::eq(
            Expr::Column(ColumnRef::new(
                TableRef::for_entity::<E>(),
                column.rust_field,
                column.column_name,
            )),
            Expr::Value(token),
        ))
    }
}

struct RawInsertable(Vec<sql_orm_core::ColumnValue>);

impl<E: Entity> Insertable<E> for RawInsertable {
    fn values(&self) -> Vec<sql_orm_core::ColumnValue> {
        self.0.clone()
    }
}

struct RawChangeset(Vec<sql_orm_core::ColumnValue>);

impl<E: Entity> Changeset<E> for RawChangeset {
    fn changes(&self) -> Vec<sql_orm_core::ColumnValue> {
        self.0.clone()
    }
}

/// Opens a direct SQL Server connection and wraps it in a `SharedConnection`.
///
/// Derived contexts use this helper behind their generated `connect(...)`
/// constructors.
pub async fn connect_shared(connection_string: &str) -> Result<SharedConnection, OrmError> {
    let connection = MssqlConnection::connect(connection_string).await?;
    Ok(SharedConnection::from_connection(connection))
}

/// Opens a direct SQL Server connection with explicit operational options.
pub async fn connect_shared_with_options(
    connection_string: &str,
    options: MssqlOperationalOptions,
) -> Result<SharedConnection, OrmError> {
    let config =
        MssqlConnectionConfig::from_connection_string_with_options(connection_string, options)?;
    connect_shared_with_config(config).await
}

/// Opens a direct SQL Server connection from a fully parsed configuration.
pub async fn connect_shared_with_config(
    config: MssqlConnectionConfig,
) -> Result<SharedConnection, OrmError> {
    let connection = MssqlConnection::connect_with_config(config).await?;
    Ok(SharedConnection::from_connection(connection))
}

#[cfg(feature = "pool-bb8")]
/// Wraps an existing SQL Server pool in a `SharedConnection`.
pub fn connect_shared_from_pool(pool: MssqlPool) -> SharedConnection {
    SharedConnection::from_pool(pool)
}

#[cfg(test)]
mod tests {
    use super::{
        ActiveTenant, DbContext, DbContextEntitySet, DbSet, SharedConnectionKind,
        SharedConnectionRuntime,
    };
    #[cfg(feature = "pool-bb8")]
    use super::{
        PooledTransactionCleanupPhase, PooledTransactionCleanupPlan, ensure_transactions_supported,
    };
    use crate::tracking::{
        ReconciledRelationshipOperation, ReconciledRelationshipOperationKind,
        RelationshipReconciliationPlan,
    };
    use crate::{
        AuditEntity, AuditOperation, AuditProvider, AuditRequestValues, EntityPersist,
        EntityPersistMode, EntityPrimaryKey, IncludeCollection, IncludeNavigation,
        RelationshipCollectionIdentityChange, RelationshipMutationIdentityChange,
        RelationshipMutationSource, RelationshipNavigationIdentityChange,
        RelationshipTrackedIdentity, SoftDeleteContext, SoftDeleteEntity, SoftDeleteOperation,
        SoftDeleteProvider, SoftDeleteRequestValues, TenantScopedEntity, Tracked,
    };
    use sql_orm_core::{
        ColumnMetadata, ColumnValue, Entity, EntityMetadata, EntityPolicyMetadata,
        ForeignKeyMetadata, FromRow, Insertable, NavigationKind, NavigationMetadata, OrmError,
        OrmErrorKind, PrimaryKeyMetadata, ReferentialAction, Row, SqlServerType, SqlValue,
    };
    use sql_orm_migrate::{
        ColumnSnapshot, MigrationOperation, ModelSnapshot, SchemaSnapshot, TableSnapshot,
        diff_column_operations, diff_schema_and_table_operations,
    };
    use sql_orm_query::{
        ColumnRef, DeleteQuery, Expr, InsertQuery, Predicate, SelectQuery, TableRef, UpdateQuery,
    };
    use std::marker::PhantomData;

    #[derive(Debug, Clone)]
    struct TestEntity;
    struct VersionedEntity;
    struct TenantWriteEntity;
    struct AuditedWriteEntity;
    struct SoftDeleteEntityUnderTest;
    struct SoftDeleteVersionedEntity;
    #[derive(Debug, Clone)]
    struct CompositeKeyEntity;
    #[derive(Debug, Clone)]
    struct ExplicitLoadRoot {
        id: i64,
        children_loaded: usize,
    }
    #[derive(Clone)]
    struct ExplicitLoadChild;
    #[derive(Clone)]
    struct BelongsToRelationshipChild {
        id: i64,
        navigation: &'static NavigationMetadata,
        changes: Vec<RelationshipMutationIdentityChange>,
    }
    #[derive(Clone)]
    struct HasManyRelationshipRoot {
        id: i64,
        navigation: &'static NavigationMetadata,
        changes: Vec<RelationshipMutationIdentityChange>,
    }
    #[derive(Clone)]
    struct RelationshipGuardEntity {
        pending_relationship_changes: usize,
    }
    #[derive(Debug, Clone)]
    struct SingleNavigationRoot {
        navigation_loaded: bool,
    }
    #[derive(Debug, Clone)]
    struct SingleNavigationTarget;
    struct DummyContext {
        entities: DbSet<TestEntity>,
    }
    struct CompositeDummyContext {
        entities: DbSet<CompositeKeyEntity>,
    }
    struct NewTestEntity {
        name: String,
        active: bool,
    }
    struct NewTenantWriteEntity {
        name: String,
        tenant_id: Option<i64>,
    }
    struct UpdateTestEntity {
        name: Option<String>,
        active: Option<bool>,
    }
    struct UpdateVersionedEntity {
        name: Option<String>,
        version: Option<Vec<u8>>,
    }
    struct TestSoftDeleteProvider;
    struct TestAuditProvider;

    static RELATIONSHIP_GUARD_NAVIGATION: NavigationMetadata = NavigationMetadata::new(
        "children",
        NavigationKind::HasMany,
        "RelationshipGuardChild",
        "dbo",
        "relationship_guard_children",
        &["id"],
        &["guard_id"],
        Some("fk_relationship_guard_children_guard_id"),
    );

    static TEST_ENTITY_COLUMNS: [ColumnMetadata; 3] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "name",
            column_name: "name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "active",
            column_name: "active",
            renamed_from: None,
            sql_type: SqlServerType::Bit,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static TEST_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TestEntity",
        schema: "dbo",
        table: "test_entities",
        renamed_from: None,
        columns: &TEST_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static EXPLICIT_LOAD_ROOT_COLUMNS: [ColumnMetadata; 1] = [ColumnMetadata {
        rust_field: "id",
        column_name: "id",
        renamed_from: None,
        sql_type: SqlServerType::BigInt,
        nullable: false,
        primary_key: true,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: true,
        updatable: false,
        max_length: None,
        precision: None,
        scale: None,
    }];

    static EXPLICIT_LOAD_CHILD_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "root_id",
            column_name: "root_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static EXPLICIT_LOAD_NAVIGATIONS: [NavigationMetadata; 1] = [NavigationMetadata::new(
        "children",
        NavigationKind::HasMany,
        "ExplicitLoadChild",
        "dbo",
        "explicit_load_children",
        &["id"],
        &["root_id"],
        Some("fk_explicit_load_children_root"),
    )];

    static EXPLICIT_LOAD_CHILD_FOREIGN_KEYS: [ForeignKeyMetadata; 1] = [ForeignKeyMetadata {
        name: "fk_explicit_load_children_root",
        columns: &["root_id"],
        referenced_schema: "dbo",
        referenced_table: "explicit_load_roots",
        referenced_columns: &["id"],
        on_delete: ReferentialAction::NoAction,
        on_update: ReferentialAction::NoAction,
    }];

    static EXPLICIT_LOAD_ROOT_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "ExplicitLoadRoot",
        schema: "dbo",
        table: "explicit_load_roots",
        renamed_from: None,
        columns: &EXPLICIT_LOAD_ROOT_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &EXPLICIT_LOAD_NAVIGATIONS,
    };

    static HAS_MANY_RELATIONSHIP_ROOT_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "HasManyRelationshipRoot",
        schema: "dbo",
        table: "explicit_load_roots",
        renamed_from: None,
        columns: &EXPLICIT_LOAD_ROOT_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &EXPLICIT_LOAD_NAVIGATIONS,
    };

    static BELONGS_TO_RELATIONSHIP_CHILD_NAVIGATIONS: [NavigationMetadata; 1] =
        [NavigationMetadata::new(
            "parent",
            NavigationKind::BelongsTo,
            "HasManyRelationshipRoot",
            "dbo",
            "explicit_load_roots",
            &["root_id"],
            &["id"],
            Some("fk_explicit_load_children_root"),
        )];

    static UNSUPPORTED_MANY_TO_MANY_NAVIGATION: NavigationMetadata = NavigationMetadata::new(
        "tags",
        NavigationKind::ManyToMany,
        "Tag",
        "dbo",
        "tags",
        &["id"],
        &["id"],
        None,
    );

    static EXPLICIT_LOAD_CHILD_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "ExplicitLoadChild",
        schema: "dbo",
        table: "explicit_load_children",
        renamed_from: None,
        columns: &EXPLICIT_LOAD_CHILD_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &EXPLICIT_LOAD_CHILD_FOREIGN_KEYS,
        navigations: &[],
    };

    static BELONGS_TO_RELATIONSHIP_CHILD_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "BelongsToRelationshipChild",
        schema: "dbo",
        table: "explicit_load_children",
        renamed_from: None,
        columns: &EXPLICIT_LOAD_CHILD_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &EXPLICIT_LOAD_CHILD_FOREIGN_KEYS,
        navigations: &BELONGS_TO_RELATIONSHIP_CHILD_NAVIGATIONS,
    };

    static RELATIONSHIP_GUARD_ENTITY_COLUMNS: [ColumnMetadata; 1] = [ColumnMetadata {
        rust_field: "id",
        column_name: "id",
        renamed_from: None,
        sql_type: SqlServerType::BigInt,
        nullable: false,
        primary_key: true,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: true,
        updatable: false,
        max_length: None,
        precision: None,
        scale: None,
    }];

    static RELATIONSHIP_GUARD_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "RelationshipGuardEntity",
        schema: "dbo",
        table: "relationship_guard_entities",
        renamed_from: None,
        columns: &RELATIONSHIP_GUARD_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static SINGLE_NAVIGATION_ROOT_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "SingleNavigationRoot",
        schema: "dbo",
        table: "single_navigation_roots",
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

    static SINGLE_NAVIGATION_TARGET_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "SingleNavigationTarget",
        schema: "dbo",
        table: "single_navigation_targets",
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

    static COMPOSITE_KEY_ENTITY_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "tenant_id",
            column_name: "tenant_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static COMPOSITE_KEY_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "CompositeKeyEntity",
        schema: "dbo",
        table: "composite_entities",
        renamed_from: None,
        columns: &COMPOSITE_KEY_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["tenant_id", "id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static VERSIONED_ENTITY_COLUMNS: [ColumnMetadata; 3] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "name",
            column_name: "name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "version",
            column_name: "version",
            renamed_from: None,
            sql_type: SqlServerType::RowVersion,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: true,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static VERSIONED_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "VersionedEntity",
        schema: "dbo",
        table: "versioned_entities",
        renamed_from: None,
        columns: &VERSIONED_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static TENANT_WRITE_ENTITY_COLUMNS: [ColumnMetadata; 5] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "name",
            column_name: "name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "tenant_id",
            column_name: "tenant_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "version",
            column_name: "version",
            renamed_from: None,
            sql_type: SqlServerType::RowVersion,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: true,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "deleted_at",
            column_name: "deleted_at",
            renamed_from: None,
            sql_type: SqlServerType::DateTime2,
            nullable: true,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static TENANT_WRITE_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TenantWriteEntity",
        schema: "dbo",
        table: "tenant_write_entities",
        renamed_from: None,
        columns: &TENANT_WRITE_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static SOFT_DELETE_ENTITY_COLUMNS: [ColumnMetadata; 3] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "name",
            column_name: "name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "deleted_at",
            column_name: "deleted_at",
            renamed_from: None,
            sql_type: SqlServerType::DateTime2,
            nullable: true,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static SOFT_DELETE_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "SoftDeleteEntityUnderTest",
        schema: "dbo",
        table: "soft_delete_entities",
        renamed_from: None,
        columns: &SOFT_DELETE_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static SOFT_DELETE_VERSIONED_ENTITY_COLUMNS: [ColumnMetadata; 4] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "name",
            column_name: "name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "deleted_at",
            column_name: "deleted_at",
            renamed_from: None,
            sql_type: SqlServerType::DateTime2,
            nullable: true,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "version",
            column_name: "version",
            renamed_from: None,
            sql_type: SqlServerType::RowVersion,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: true,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static SOFT_DELETE_VERSIONED_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "SoftDeleteVersionedEntity",
        schema: "dbo",
        table: "soft_delete_versioned_entities",
        renamed_from: None,
        columns: &SOFT_DELETE_VERSIONED_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static SOFT_DELETE_POLICY_COLUMNS: [ColumnMetadata; 1] = [ColumnMetadata {
        rust_field: "deleted_at",
        column_name: "deleted_at",
        renamed_from: None,
        sql_type: SqlServerType::DateTime2,
        nullable: true,
        primary_key: false,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: false,
        updatable: true,
        max_length: None,
        precision: None,
        scale: None,
    }];

    static AUDITED_WRITE_ENTITY_COLUMNS: [ColumnMetadata; 3] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "name",
            column_name: "name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "updated_by",
            column_name: "updated_by",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
    ];

    static AUDITED_WRITE_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "AuditedWriteEntity",
        schema: "dbo",
        table: "audited_write_entities",
        renamed_from: None,
        columns: &AUDITED_WRITE_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static AUDITED_WRITE_POLICY_COLUMNS: [ColumnMetadata; 1] = [AUDITED_WRITE_ENTITY_COLUMNS[2]];

    impl Entity for TestEntity {
        fn metadata() -> &'static EntityMetadata {
            &TEST_ENTITY_METADATA
        }
    }

    impl Entity for CompositeKeyEntity {
        fn metadata() -> &'static EntityMetadata {
            &COMPOSITE_KEY_ENTITY_METADATA
        }
    }

    impl Entity for VersionedEntity {
        fn metadata() -> &'static EntityMetadata {
            &VERSIONED_ENTITY_METADATA
        }
    }

    impl Entity for TenantWriteEntity {
        fn metadata() -> &'static EntityMetadata {
            &TENANT_WRITE_ENTITY_METADATA
        }
    }

    impl Entity for AuditedWriteEntity {
        fn metadata() -> &'static EntityMetadata {
            &AUDITED_WRITE_ENTITY_METADATA
        }
    }

    impl Entity for SoftDeleteEntityUnderTest {
        fn metadata() -> &'static EntityMetadata {
            &SOFT_DELETE_ENTITY_METADATA
        }
    }

    impl Entity for SoftDeleteVersionedEntity {
        fn metadata() -> &'static EntityMetadata {
            &SOFT_DELETE_VERSIONED_ENTITY_METADATA
        }
    }

    impl Entity for ExplicitLoadRoot {
        fn metadata() -> &'static EntityMetadata {
            &EXPLICIT_LOAD_ROOT_METADATA
        }
    }

    impl Entity for ExplicitLoadChild {
        fn metadata() -> &'static EntityMetadata {
            &EXPLICIT_LOAD_CHILD_METADATA
        }
    }

    impl Entity for BelongsToRelationshipChild {
        fn metadata() -> &'static EntityMetadata {
            &BELONGS_TO_RELATIONSHIP_CHILD_METADATA
        }
    }

    impl Entity for HasManyRelationshipRoot {
        fn metadata() -> &'static EntityMetadata {
            &HAS_MANY_RELATIONSHIP_ROOT_METADATA
        }
    }

    impl Entity for RelationshipGuardEntity {
        fn metadata() -> &'static EntityMetadata {
            &RELATIONSHIP_GUARD_ENTITY_METADATA
        }
    }

    impl Entity for SingleNavigationRoot {
        fn metadata() -> &'static EntityMetadata {
            &SINGLE_NAVIGATION_ROOT_METADATA
        }
    }

    impl Entity for SingleNavigationTarget {
        fn metadata() -> &'static EntityMetadata {
            &SINGLE_NAVIGATION_TARGET_METADATA
        }
    }

    impl SoftDeleteEntity for TestEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl AuditEntity for TestEntity {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for CompositeKeyEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl AuditEntity for CompositeKeyEntity {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl EntityPrimaryKey for CompositeKeyEntity {
        fn primary_key_value(&self) -> Result<SqlValue, OrmError> {
            Err(OrmError::compile(
                "change tracking currently supports only entities with a single primary key column",
            ))
        }
    }

    impl EntityPersist for CompositeKeyEntity {
        fn persist_mode(&self) -> Result<EntityPersistMode, OrmError> {
            Err(OrmError::compile(
                "change tracking currently supports only entities with a single primary key column",
            ))
        }

        fn insert_values(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn update_changes(&self) -> Vec<ColumnValue> {
            vec![ColumnValue::new(
                "name",
                SqlValue::String("changed".to_string()),
            )]
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError> {
            Ok(None)
        }

        fn sync_persisted(&mut self, persisted: Self) {
            *self = persisted;
        }
    }

    impl SoftDeleteEntity for VersionedEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl AuditEntity for VersionedEntity {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for TenantWriteEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new(
                "soft_delete",
                &TENANT_WRITE_ENTITY_COLUMNS[4..5],
            ))
        }
    }

    impl AuditEntity for TenantWriteEntity {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl AuditEntity for AuditedWriteEntity {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new(
                "audit",
                &AUDITED_WRITE_POLICY_COLUMNS,
            ))
        }
    }

    impl SoftDeleteEntity for SoftDeleteEntityUnderTest {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new(
                "soft_delete",
                &SOFT_DELETE_POLICY_COLUMNS,
            ))
        }
    }

    impl AuditEntity for SoftDeleteEntityUnderTest {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for SoftDeleteVersionedEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new(
                "soft_delete",
                &SOFT_DELETE_POLICY_COLUMNS,
            ))
        }
    }

    impl AuditEntity for SoftDeleteVersionedEntity {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for ExplicitLoadChild {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl AuditEntity for ExplicitLoadChild {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl EntityPrimaryKey for ExplicitLoadChild {
        fn primary_key_value(&self) -> Result<SqlValue, OrmError> {
            Ok(SqlValue::I64(11))
        }
    }

    impl EntityPersist for ExplicitLoadChild {
        fn persist_mode(&self) -> Result<EntityPersistMode, OrmError> {
            Ok(EntityPersistMode::Update(SqlValue::I64(11)))
        }

        fn insert_values(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn update_changes(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError> {
            Ok(None)
        }

        fn sync_persisted(&mut self, persisted: Self) {
            *self = persisted;
        }
    }

    impl SoftDeleteEntity for BelongsToRelationshipChild {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl AuditEntity for BelongsToRelationshipChild {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for BelongsToRelationshipChild {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl EntityPrimaryKey for BelongsToRelationshipChild {
        fn primary_key_value(&self) -> Result<SqlValue, OrmError> {
            Ok(SqlValue::I64(self.id))
        }
    }

    impl EntityPersist for BelongsToRelationshipChild {
        fn persist_mode(&self) -> Result<EntityPersistMode, OrmError> {
            Ok(EntityPersistMode::Update(SqlValue::I64(self.id)))
        }

        fn insert_values(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn update_changes(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError> {
            Ok(None)
        }

        fn sync_persisted(&mut self, persisted: Self) {
            *self = persisted;
        }
    }

    impl FromRow for BelongsToRelationshipChild {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self {
                id: 11,
                navigation: &BELONGS_TO_RELATIONSHIP_CHILD_NAVIGATIONS[0],
                changes: Vec::new(),
            })
        }
    }

    impl RelationshipMutationSource for BelongsToRelationshipChild {
        fn relationship_change_batches(&self) -> Vec<crate::RelationshipMutationBatch> {
            if self.changes.is_empty() {
                return Vec::new();
            }

            vec![crate::RelationshipMutationBatch::new(
                self.navigation,
                self.changes.clone(),
            )]
        }
    }

    impl AuditEntity for RelationshipGuardEntity {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for RelationshipGuardEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for RelationshipGuardEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl EntityPrimaryKey for RelationshipGuardEntity {
        fn primary_key_value(&self) -> Result<SqlValue, OrmError> {
            Ok(SqlValue::I64(1))
        }
    }

    impl EntityPersist for RelationshipGuardEntity {
        fn persist_mode(&self) -> Result<EntityPersistMode, OrmError> {
            Ok(EntityPersistMode::Update(SqlValue::I64(1)))
        }

        fn insert_values(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn update_changes(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError> {
            Ok(None)
        }

        fn sync_persisted(&mut self, persisted: Self) {
            *self = persisted;
        }
    }

    impl RelationshipMutationSource for RelationshipGuardEntity {
        fn relationship_change_batches(&self) -> Vec<crate::RelationshipMutationBatch> {
            if self.pending_relationship_changes == 0 {
                return Vec::new();
            }

            vec![crate::RelationshipMutationBatch::new(
                &RELATIONSHIP_GUARD_NAVIGATION,
                vec![
                    crate::RelationshipMutationIdentityChange::Collection(
                        crate::RelationshipCollectionIdentityChange::Added(None),
                    );
                    self.pending_relationship_changes
                ],
            )]
        }

        fn pending_relationship_change_count(&self) -> usize {
            self.pending_relationship_changes
        }
    }

    impl FromRow for RelationshipGuardEntity {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self {
                pending_relationship_changes: 0,
            })
        }
    }

    impl TenantScopedEntity for TestEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for CompositeKeyEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for VersionedEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for TenantWriteEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new(
                "tenant",
                &TENANT_WRITE_ENTITY_COLUMNS[2..3],
            ))
        }
    }

    impl TenantScopedEntity for AuditedWriteEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for SoftDeleteEntityUnderTest {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for SoftDeleteVersionedEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for ExplicitLoadChild {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl FromRow for TestEntity {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self)
        }
    }

    impl FromRow for CompositeKeyEntity {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self)
        }
    }

    impl FromRow for ExplicitLoadChild {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self)
        }
    }

    impl EntityPrimaryKey for HasManyRelationshipRoot {
        fn primary_key_value(&self) -> Result<SqlValue, OrmError> {
            Ok(SqlValue::I64(self.id))
        }
    }

    impl EntityPersist for HasManyRelationshipRoot {
        fn persist_mode(&self) -> Result<EntityPersistMode, OrmError> {
            Ok(EntityPersistMode::Update(SqlValue::I64(self.id)))
        }

        fn insert_values(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn update_changes(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError> {
            Ok(None)
        }

        fn sync_persisted(&mut self, persisted: Self) {
            *self = persisted;
        }
    }

    impl FromRow for HasManyRelationshipRoot {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self {
                id: 7,
                navigation: &EXPLICIT_LOAD_NAVIGATIONS[0],
                changes: Vec::new(),
            })
        }
    }

    impl AuditEntity for HasManyRelationshipRoot {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for HasManyRelationshipRoot {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for HasManyRelationshipRoot {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl RelationshipMutationSource for HasManyRelationshipRoot {
        fn relationship_change_batches(&self) -> Vec<crate::RelationshipMutationBatch> {
            if self.changes.is_empty() {
                return Vec::new();
            }

            vec![crate::RelationshipMutationBatch::new(
                self.navigation,
                self.changes.clone(),
            )]
        }
    }

    impl EntityPrimaryKey for ExplicitLoadRoot {
        fn primary_key_value(&self) -> Result<SqlValue, OrmError> {
            Ok(SqlValue::I64(self.id))
        }
    }

    impl EntityPersist for ExplicitLoadRoot {
        fn persist_mode(&self) -> Result<EntityPersistMode, OrmError> {
            Ok(EntityPersistMode::Update(SqlValue::I64(self.id)))
        }

        fn insert_values(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn update_changes(&self) -> Vec<ColumnValue> {
            Vec::new()
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError> {
            Ok(None)
        }

        fn sync_persisted(&mut self, persisted: Self) {
            *self = persisted;
        }
    }

    impl FromRow for ExplicitLoadRoot {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self {
                id: 7,
                children_loaded: 0,
            })
        }
    }

    impl AuditEntity for ExplicitLoadRoot {
        fn audit_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for ExplicitLoadRoot {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for ExplicitLoadRoot {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl IncludeCollection<ExplicitLoadChild> for ExplicitLoadRoot {
        fn set_included_collection(
            &mut self,
            navigation: &str,
            values: Vec<ExplicitLoadChild>,
        ) -> Result<(), OrmError> {
            if navigation != "children" {
                return Err(OrmError::mapping("unexpected navigation"));
            }

            self.children_loaded = values.len();
            Ok(())
        }
    }

    impl IncludeNavigation<SingleNavigationTarget> for SingleNavigationRoot {
        fn set_included_navigation(
            &mut self,
            navigation: &str,
            value: Option<SingleNavigationTarget>,
        ) -> Result<(), OrmError> {
            if navigation != "target" {
                return Err(OrmError::mapping("unexpected navigation"));
            }

            self.navigation_loaded = value.is_some();
            Ok(())
        }
    }

    impl DbContext for DummyContext {
        fn from_shared_connection(_connection: super::SharedConnection) -> Self {
            unreachable!("DummyContext is only used in disconnected unit tests")
        }

        fn shared_connection(&self) -> super::SharedConnection {
            panic!("DummyContext is only used in disconnected unit tests")
        }

        fn tracking_registry(&self) -> crate::TrackingRegistryHandle {
            self.entities.tracking_registry()
        }
    }

    impl DbContextEntitySet<TestEntity> for DummyContext {
        fn db_set(&self) -> &DbSet<TestEntity> {
            &self.entities
        }
    }

    impl DbContext for CompositeDummyContext {
        fn from_shared_connection(_connection: super::SharedConnection) -> Self {
            unreachable!("CompositeDummyContext is only used in disconnected unit tests")
        }

        fn shared_connection(&self) -> super::SharedConnection {
            panic!("CompositeDummyContext is only used in disconnected unit tests")
        }

        fn tracking_registry(&self) -> crate::TrackingRegistryHandle {
            self.entities.tracking_registry()
        }
    }

    impl DbContextEntitySet<CompositeKeyEntity> for CompositeDummyContext {
        fn db_set(&self) -> &DbSet<CompositeKeyEntity> {
            &self.entities
        }
    }

    impl sql_orm_core::Insertable<TestEntity> for NewTestEntity {
        fn values(&self) -> Vec<ColumnValue> {
            vec![
                ColumnValue::new("name", SqlValue::String(self.name.clone())),
                ColumnValue::new("active", SqlValue::Bool(self.active)),
            ]
        }
    }

    impl sql_orm_core::Insertable<TenantWriteEntity> for NewTenantWriteEntity {
        fn values(&self) -> Vec<ColumnValue> {
            let mut values = vec![ColumnValue::new(
                "name",
                SqlValue::String(self.name.clone()),
            )];

            if let Some(tenant_id) = self.tenant_id {
                values.push(ColumnValue::new("tenant_id", SqlValue::I64(tenant_id)));
            }

            values
        }
    }

    impl sql_orm_core::Changeset<TestEntity> for UpdateTestEntity {
        fn changes(&self) -> Vec<ColumnValue> {
            let mut values = Vec::new();

            if let Some(name) = &self.name {
                values.push(ColumnValue::new("name", SqlValue::String(name.clone())));
            }

            if let Some(active) = self.active {
                values.push(ColumnValue::new("active", SqlValue::Bool(active)));
            }

            values
        }
    }

    impl sql_orm_core::Changeset<CompositeKeyEntity> for UpdateTestEntity {
        fn changes(&self) -> Vec<ColumnValue> {
            <Self as sql_orm_core::Changeset<TestEntity>>::changes(self)
        }
    }

    impl sql_orm_core::Changeset<VersionedEntity> for UpdateVersionedEntity {
        fn changes(&self) -> Vec<ColumnValue> {
            let mut values = Vec::new();

            if let Some(name) = &self.name {
                values.push(ColumnValue::new("name", SqlValue::String(name.clone())));
            }

            values
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, sql_orm_core::OrmError> {
            Ok(self.version.clone().map(SqlValue::Bytes))
        }
    }

    impl sql_orm_core::Changeset<TenantWriteEntity> for UpdateVersionedEntity {
        fn changes(&self) -> Vec<ColumnValue> {
            <Self as sql_orm_core::Changeset<VersionedEntity>>::changes(self)
        }

        fn concurrency_token(&self) -> Result<Option<SqlValue>, sql_orm_core::OrmError> {
            <Self as sql_orm_core::Changeset<VersionedEntity>>::concurrency_token(self)
        }
    }

    impl SoftDeleteProvider for TestSoftDeleteProvider {
        fn apply(
            &self,
            context: SoftDeleteContext<'_>,
            changes: &mut Vec<ColumnValue>,
        ) -> Result<(), OrmError> {
            assert_eq!(context.operation, SoftDeleteOperation::Delete);
            changes.push(ColumnValue::new(
                "deleted_at",
                SqlValue::String("2026-04-25T00:00:00".to_string()),
            ));
            Ok(())
        }
    }

    impl AuditProvider for TestAuditProvider {
        fn values(&self, context: crate::AuditContext<'_>) -> Result<Vec<ColumnValue>, OrmError> {
            assert_eq!(context.operation, AuditOperation::Update);
            Ok(vec![ColumnValue::new(
                "updated_by",
                SqlValue::String("audit-provider".to_string()),
            )])
        }
    }

    #[test]
    fn direct_shared_connections_support_transactions() {
        assert_eq!(
            super::ensure_transactions_supported(SharedConnectionKind::Direct),
            Ok(())
        );
    }

    #[test]
    fn transaction_depth_is_shared_across_runtime_clones() {
        let runtime = SharedConnectionRuntime::default();
        let cloned_runtime = runtime.clone();

        runtime
            .transaction_depth
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        assert_eq!(
            cloned_runtime
                .transaction_depth
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn pooled_shared_connections_support_transaction_boundary() {
        assert_eq!(
            ensure_transactions_supported(SharedConnectionKind::Pool),
            Ok(())
        );
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn pooled_begin_error_cleanup_plan_clears_pinned_slot_without_transaction_state() {
        let plan =
            PooledTransactionCleanupPlan::for_phase(PooledTransactionCleanupPhase::BeginError);

        assert_eq!(
            plan,
            PooledTransactionCleanupPlan {
                restore_retry: false,
                exit_transaction_scope: false,
                clear_pinned_connection: true,
            }
        );
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn pooled_commit_error_cleanup_plan_restores_runtime_and_clears_pinned_slot() {
        let plan = PooledTransactionCleanupPlan::for_phase(
            PooledTransactionCleanupPhase::AfterCommitAttempt,
        );

        assert_eq!(
            plan,
            PooledTransactionCleanupPlan {
                restore_retry: true,
                exit_transaction_scope: true,
                clear_pinned_connection: true,
            }
        );
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn pooled_rollback_error_cleanup_plan_restores_runtime_and_clears_pinned_slot() {
        let plan = PooledTransactionCleanupPlan::for_phase(
            PooledTransactionCleanupPhase::AfterRollbackAttempt,
        );

        assert_eq!(
            plan,
            PooledTransactionCleanupPlan {
                restore_retry: true,
                exit_transaction_scope: true,
                clear_pinned_connection: true,
            }
        );
    }

    #[test]
    fn transaction_depth_detects_active_transaction() {
        let runtime = SharedConnectionRuntime::default();

        assert_eq!(
            runtime
                .transaction_depth
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        runtime
            .transaction_depth
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        assert_eq!(
            runtime
                .transaction_depth
                .load(std::sync::atomic::Ordering::SeqCst),
            1
        );
    }

    #[test]
    fn active_transaction_scope_only_allows_matching_transaction_handle() {
        assert!(super::transaction_scope_allows_lock(None, None));
        assert!(super::transaction_scope_allows_lock(None, Some(7)));
        assert!(super::transaction_scope_allows_lock(Some(7), Some(7)));
        assert!(!super::transaction_scope_allows_lock(Some(7), None));
        assert!(!super::transaction_scope_allows_lock(Some(7), Some(8)));
    }

    #[test]
    fn active_transaction_scope_rejection_is_transaction_error() {
        let error = super::ensure_transaction_scope_allows_lock(Some(7), None).unwrap_err();

        assert_eq!(error.kind(), sql_orm_core::OrmErrorKind::Transaction);
        assert_eq!(
            error.message(),
            "a transaction is active on this shared connection; use the transaction context passed to db.transaction(...)"
        );
    }

    #[test]
    fn transaction_scope_lifecycle_sets_and_clears_runtime_scope() {
        let runtime = SharedConnectionRuntime::default();

        runtime
            .active_transaction_scope
            .store(11, std::sync::atomic::Ordering::SeqCst);
        runtime
            .transaction_depth
            .fetch_add(1, std::sync::atomic::Ordering::SeqCst);

        assert_eq!(
            runtime
                .active_transaction_scope
                .load(std::sync::atomic::Ordering::SeqCst),
            11
        );

        let _ = runtime.transaction_depth.fetch_update(
            std::sync::atomic::Ordering::SeqCst,
            std::sync::atomic::Ordering::SeqCst,
            |depth| Some(depth.saturating_sub(1)),
        );
        runtime
            .active_transaction_scope
            .store(0, std::sync::atomic::Ordering::SeqCst);

        assert_eq!(
            runtime
                .active_transaction_scope
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
        assert_eq!(
            runtime
                .transaction_depth
                .load(std::sync::atomic::Ordering::SeqCst),
            0
        );
    }

    #[test]
    fn dbset_exposes_entity_metadata() {
        let dbset = DbSet::<TestEntity>::disconnected();

        assert_eq!(dbset.entity_metadata().table, "test_entities");
    }

    #[test]
    fn dbcontext_entity_set_trait_returns_typed_dbset() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };

        let dbset = <DummyContext as DbContextEntitySet<TestEntity>>::db_set(&context);

        assert_eq!(dbset.entity_metadata().rust_name, "TestEntity");
        assert_eq!(dbset.entity_metadata().table, "test_entities");
    }

    #[test]
    fn dbset_debug_includes_entity_name() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let rendered = format!("{dbset:?}");

        assert!(rendered.contains("TestEntity"));
        assert!(rendered.contains("test_entities"));
    }

    #[test]
    fn dbset_query_uses_entity_select_query_by_default() {
        let dbset = DbSet::<TestEntity>::disconnected();

        assert_eq!(
            dbset.query().into_select_query(),
            SelectQuery::from_entity::<TestEntity>()
        );
    }

    #[test]
    fn dbset_query_with_accepts_custom_select_query() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let custom = SelectQuery::from_entity::<TestEntity>();

        assert_eq!(dbset.query_with(custom.clone()).into_select_query(), custom);
    }

    #[test]
    fn dbset_internal_query_visibility_bypasses_soft_delete_filter() {
        let dbset = DbSet::<SoftDeleteEntityUnderTest>::disconnected();
        let select = SelectQuery::from_entity::<SoftDeleteEntityUnderTest>();

        assert_eq!(
            dbset
                .query_with_internal_visibility(select.clone())
                .into_select_query(),
            select
        );
    }

    #[test]
    fn dbset_find_builds_select_query_for_single_primary_key() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset.find_select_query(7_i64).unwrap();

        assert_eq!(
            query,
            SelectQuery::from_entity::<TestEntity>().filter(Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::new("dbo", "test_entities"),
                    "id",
                    "id",
                )),
                Expr::Value(sql_orm_core::SqlValue::I64(7)),
            ))
        );
    }

    #[test]
    fn dbset_find_rejects_composite_primary_keys() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();

        let error = dbset.find_select_query(7_i64).unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet currently supports this operation only for entities with a single primary key column"
        );
        assert_eq!(error.kind(), OrmErrorKind::Compile);
    }

    #[test]
    fn dbset_require_connection_reports_execution_error() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let error = match dbset.require_connection() {
            Ok(_) => panic!("disconnected DbSet unexpectedly returned a connection"),
            Err(error) => error,
        };

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(error.kind(), OrmErrorKind::Execution);
    }

    #[test]
    fn explicit_collection_loading_builds_related_entity_query() {
        let dbset = DbSet::<ExplicitLoadRoot>::disconnected();
        let root = ExplicitLoadRoot {
            id: 7,
            children_loaded: 0,
        };

        let query = dbset
            .explicit_collection_query::<ExplicitLoadChild>(&root, "children")
            .unwrap()
            .into_select_query();

        assert_eq!(
            query,
            SelectQuery::from_entity::<ExplicitLoadChild>().filter(Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::new("dbo", "explicit_load_children"),
                    "root_id",
                    "root_id",
                )),
                Expr::Value(SqlValue::I64(7)),
            ))
        );
    }

    #[test]
    fn explicit_collection_loading_rejects_unknown_navigation() {
        let dbset = DbSet::<ExplicitLoadRoot>::disconnected();
        let root = ExplicitLoadRoot {
            id: 7,
            children_loaded: 0,
        };

        let error = dbset
            .explicit_collection_query::<ExplicitLoadChild>(&root, "missing")
            .unwrap_err();

        assert!(error.message().contains("does not declare navigation"));
    }

    #[test]
    fn explicit_collection_loading_tracked_assignment_does_not_mark_modified() {
        let dbset = DbSet::<ExplicitLoadRoot>::disconnected();
        let mut tracked = Tracked::from_loaded(ExplicitLoadRoot {
            id: 7,
            children_loaded: 0,
        });

        tracked
            .current_mut_without_state_change()
            .set_included_collection("children", vec![ExplicitLoadChild])
            .unwrap();

        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(tracked.current().children_loaded, 1);
        drop(dbset);
    }

    #[test]
    fn tracked_navigation_assignment_does_not_register_related_graph() {
        let dbset = DbSet::<ExplicitLoadRoot>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(ExplicitLoadRoot {
            id: 7,
            children_loaded: 0,
        });
        tracked.attach_registry(registry.clone());

        tracked
            .current_mut_without_state_change()
            .set_included_collection("children", vec![ExplicitLoadChild])
            .unwrap();

        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(tracked.current().children_loaded, 1);
        assert_eq!(registry.tracked_for::<ExplicitLoadRoot>().len(), 1);
        assert_eq!(registry.tracked_for::<ExplicitLoadChild>().len(), 0);
        assert_eq!(registry.entry_count(), 1);
    }

    #[test]
    fn tracked_navigation_values_reuse_identity_map_snapshots_when_available() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked_related = Tracked::from_loaded(ExplicitLoadRoot {
            id: 7,
            children_loaded: 1,
        });
        tracked_related
            .attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();
        tracked_related.current_mut().children_loaded = 3;

        let values = dbset
            .identity_mapped_navigation_values(vec![
                ExplicitLoadRoot {
                    id: 7,
                    children_loaded: 0,
                },
                ExplicitLoadRoot {
                    id: 8,
                    children_loaded: 2,
                },
            ])
            .unwrap();

        assert_eq!(values[0].id, 7);
        assert_eq!(values[0].children_loaded, 3);
        assert_eq!(values[1].id, 8);
        assert_eq!(values[1].children_loaded, 2);
        assert_eq!(registry.tracked_for::<ExplicitLoadRoot>().len(), 1);
    }

    #[test]
    fn tracked_single_navigation_assignment_does_not_register_related_graph() {
        let dbset = DbSet::<SingleNavigationRoot>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(SingleNavigationRoot {
            navigation_loaded: false,
        });
        tracked.attach_registry(registry.clone());

        tracked
            .current_mut_without_state_change()
            .set_included_navigation("target", Some(SingleNavigationTarget))
            .unwrap();

        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert!(tracked.current().navigation_loaded);
        assert_eq!(registry.tracked_for::<SingleNavigationRoot>().len(), 1);
        assert_eq!(registry.tracked_for::<SingleNavigationTarget>().len(), 0);
        assert_eq!(registry.entry_count(), 1);
    }

    #[tokio::test]
    async fn dbset_find_tracked_reuses_find_connection_path() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let error = dbset.find_tracked(7_i64).await.unwrap_err();

        assert_eq!(
            error.message(),
            "DbSetQuery requires an initialized shared connection"
        );
    }

    #[tokio::test]
    async fn dbset_find_tracked_rejects_composite_primary_keys_with_stable_error() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();

        let error = dbset.find_tracked(7_i64).await.unwrap_err();

        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
        assert_eq!(error.kind(), OrmErrorKind::Compile);
    }

    #[test]
    fn dbset_add_tracked_registers_added_entity_in_registry() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let registry = dbset.tracking_registry();

        let tracked = dbset.add_tracked(TestEntity);

        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, crate::EntityState::Added);
    }

    #[tokio::test]
    async fn save_tracked_added_rejects_composite_primary_keys_before_sql() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let tracked = dbset.add_tracked(CompositeKeyEntity);

        let error = dbset.save_tracked_added().await.unwrap_err();

        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn save_tracked_added_returns_zero_without_pending_added_entries() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());

        let saved = dbset.save_tracked_added().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(registry.entry_count(), 1);
    }

    #[tokio::test]
    async fn mark_unchanged_on_added_entry_discards_pending_insert_before_validation() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = dbset.add_tracked(CompositeKeyEntity);

        tracked.mark_unchanged();
        let saved = dbset.save_tracked_added().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Unchanged
        );
    }

    #[tokio::test]
    async fn dropping_added_entry_keeps_pending_insert_for_registry_owned_entry() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();

        {
            let tracked = dbset.add_tracked(CompositeKeyEntity);

            assert_eq!(tracked.state(), crate::EntityState::Added);
            assert_eq!(registry.entry_count(), 1);
        }

        let error = dbset.save_tracked_added().await.unwrap_err();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn into_current_on_added_entry_keeps_pending_insert_for_registry_owned_entry() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let tracked = dbset.add_tracked(CompositeKeyEntity);

        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 1);

        let _current = tracked.into_current();
        let error = dbset.save_tracked_added().await.unwrap_err();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn dropping_clone_of_added_entry_does_not_cancel_original_pending_insert() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let tracked = dbset.add_tracked(CompositeKeyEntity);

        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 1);

        let clone = tracked.clone();
        assert_eq!(clone.state(), crate::EntityState::Added);
        drop(clone);

        let error = dbset.save_tracked_added().await.unwrap_err();

        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[test]
    fn dbset_remove_tracked_marks_loaded_entity_as_deleted() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(TestEntity);
        tracked.attach_registry(registry.clone());

        dbset.remove_tracked(&mut tracked);

        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Deleted
        );
    }

    #[test]
    fn dbset_remove_tracked_marks_modified_entity_as_deleted_without_detaching() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(TestEntity);
        tracked.attach_registry(registry.clone());
        tracked.current_mut();

        dbset.remove_tracked(&mut tracked);

        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Deleted
        );
    }

    #[tokio::test]
    async fn save_tracked_deleted_rejects_composite_primary_keys_before_sql() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());

        dbset.remove_tracked(&mut tracked);
        let error = dbset.save_tracked_deleted().await.unwrap_err();

        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn mark_modified_on_deleted_entry_keeps_pending_delete_without_update() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());

        dbset.remove_tracked(&mut tracked);
        tracked.mark_modified();
        let modified_saved = dbset.save_tracked_modified().await.unwrap();
        let delete_error = dbset.save_tracked_deleted().await.unwrap_err();

        assert_eq!(modified_saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Deleted
        );
        assert_eq!(
            delete_error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn mark_unchanged_on_deleted_entry_discards_pending_delete_before_validation() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());

        dbset.remove_tracked(&mut tracked);
        tracked.mark_unchanged();
        let deleted_saved = dbset.save_tracked_deleted().await.unwrap();

        assert_eq!(deleted_saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Unchanged
        );
    }

    #[tokio::test]
    async fn dropping_deleted_entry_keeps_pending_delete_for_registry_owned_entry() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();

        {
            let mut deleted = Tracked::from_loaded(CompositeKeyEntity);
            deleted.attach_registry(registry.clone());
            dbset.remove_tracked(&mut deleted);

            assert_eq!(deleted.state(), crate::EntityState::Deleted);
            assert_eq!(registry.entry_count(), 1);
        }

        let error = dbset.save_tracked_deleted().await.unwrap_err();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn into_current_on_deleted_entry_keeps_pending_delete_for_registry_owned_entry() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());
        dbset.remove_tracked(&mut tracked);

        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);

        let _current = tracked.into_current();
        let error = dbset.save_tracked_deleted().await.unwrap_err();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn dropping_clone_of_deleted_entry_does_not_cancel_original_pending_delete() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());
        dbset.remove_tracked(&mut tracked);

        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);

        let clone = tracked.clone();
        assert_eq!(clone.state(), crate::EntityState::Deleted);
        drop(clone);

        let error = dbset.save_tracked_deleted().await.unwrap_err();

        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[test]
    fn dbset_remove_tracked_cancels_pending_added_entity() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = dbset.add_tracked(TestEntity);

        dbset.remove_tracked(&mut tracked);

        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);
    }

    #[test]
    fn dbset_remove_tracked_is_idempotent_after_added_entry_was_cancelled() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = dbset.add_tracked(TestEntity);

        dbset.remove_tracked(&mut tracked);
        dbset.remove_tracked(&mut tracked);

        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);
    }

    #[tokio::test]
    async fn save_tracked_deleted_returns_zero_after_added_entry_was_cancelled() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = dbset.add_tracked(CompositeKeyEntity);

        dbset.remove_tracked(&mut tracked);
        let saved = dbset.save_tracked_deleted().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);
    }

    #[tokio::test]
    async fn detach_tracked_added_entry_prevents_later_insert_without_resetting_state() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = dbset.add_tracked(CompositeKeyEntity);

        dbset.detach_tracked(&mut tracked);
        let saved = dbset.save_tracked_added().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 0);
    }

    #[test]
    fn dbset_detach_tracked_discards_pending_modified_entry() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(TestEntity);
        tracked.attach_registry(registry.clone());
        tracked.current_mut();

        dbset.detach_tracked(&mut tracked);

        assert_eq!(tracked.state(), crate::EntityState::Modified);
        assert_eq!(registry.entry_count(), 0);
    }

    #[tokio::test]
    async fn detach_tracked_deleted_entry_prevents_later_delete_without_resetting_state() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());

        dbset.remove_tracked(&mut tracked);
        dbset.detach_tracked(&mut tracked);
        let saved = dbset.save_tracked_deleted().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);
    }

    #[test]
    fn dbcontext_clear_tracker_removes_entries_without_resetting_wrappers() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let registry = <DummyContext as DbContext>::tracking_registry(&context);
        let added = context.entities.add_tracked(TestEntity);
        let mut modified = Tracked::from_loaded(TestEntity);
        modified.attach_registry(registry.clone());
        modified.mark_modified();

        assert_eq!(registry.entry_count(), 2);

        <DummyContext as DbContext>::clear_tracker(&context);

        assert_eq!(registry.entry_count(), 0);
        assert_eq!(added.state(), crate::EntityState::Added);
        assert_eq!(modified.state(), crate::EntityState::Modified);
    }

    #[tokio::test]
    async fn clear_tracker_discards_added_and_deleted_entries_before_save_phase_validation() {
        let context = CompositeDummyContext {
            entities: DbSet::<CompositeKeyEntity>::disconnected(),
        };
        let registry = <CompositeDummyContext as DbContext>::tracking_registry(&context);
        let added = context.entities.add_tracked(CompositeKeyEntity);
        let mut deleted = Tracked::from_loaded(CompositeKeyEntity);
        deleted.attach_registry(registry.clone());
        context.entities.remove_tracked(&mut deleted);

        assert_eq!(registry.entry_count(), 2);

        <CompositeDummyContext as DbContext>::clear_tracker(&context);

        let added_saved = context.entities.save_tracked_added().await.unwrap();
        let deleted_saved = context.entities.save_tracked_deleted().await.unwrap();

        assert_eq!(added_saved, 0);
        assert_eq!(deleted_saved, 0);
        assert_eq!(registry.entry_count(), 0);
        assert_eq!(added.state(), crate::EntityState::Added);
        assert_eq!(deleted.state(), crate::EntityState::Deleted);
    }

    #[tokio::test]
    async fn clear_tracker_discards_modified_entries_before_save_phase_validation() {
        let context = CompositeDummyContext {
            entities: DbSet::<CompositeKeyEntity>::disconnected(),
        };
        let registry = <CompositeDummyContext as DbContext>::tracking_registry(&context);
        let mut modified = Tracked::from_loaded(CompositeKeyEntity);
        modified.attach_registry(registry.clone());
        modified.mark_modified();

        assert_eq!(registry.entry_count(), 1);

        <CompositeDummyContext as DbContext>::clear_tracker(&context);

        let modified_saved = context.entities.save_tracked_modified().await.unwrap();

        assert_eq!(modified_saved, 0);
        assert_eq!(registry.entry_count(), 0);
        assert_eq!(modified.state(), crate::EntityState::Modified);
    }

    #[tokio::test]
    async fn dropping_modified_entry_keeps_pending_update_for_registry_owned_entry() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();

        {
            let mut modified = Tracked::from_loaded(CompositeKeyEntity);
            modified.attach_registry(registry.clone());
            modified.mark_modified();

            assert_eq!(registry.entry_count(), 1);
        }

        let error = dbset.save_tracked_modified().await.unwrap_err();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn into_current_on_modified_entry_keeps_pending_update_for_registry_owned_entry() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());
        tracked.mark_modified();

        assert_eq!(tracked.state(), crate::EntityState::Modified);
        assert_eq!(registry.entry_count(), 1);

        let _current = tracked.into_current();
        let error = dbset.save_tracked_modified().await.unwrap_err();

        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn dropped_modified_entry_without_persisted_changes_accepts_registry_snapshot() {
        let dbset = DbSet::<ExplicitLoadRoot>::disconnected();
        let registry = dbset.tracking_registry();

        {
            let mut tracked = Tracked::from_loaded(ExplicitLoadRoot {
                id: 7,
                children_loaded: 0,
            });
            tracked
                .attach_registry_loaded(registry.clone(), SqlValue::I64(7))
                .unwrap();
            tracked.current_mut().children_loaded = 1;

            assert_eq!(tracked.state(), crate::EntityState::Modified);
            assert_eq!(registry.entry_count(), 1);
        }

        let saved = dbset.save_tracked_modified().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Unchanged
        );
    }

    #[tokio::test]
    async fn dropping_clone_of_modified_entry_does_not_cancel_original_pending_update() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());
        tracked.mark_modified();

        assert_eq!(tracked.state(), crate::EntityState::Modified);
        assert_eq!(registry.entry_count(), 1);

        let clone = tracked.clone();
        assert_eq!(clone.state(), crate::EntityState::Modified);
        drop(clone);

        let error = dbset.save_tracked_modified().await.unwrap_err();

        assert_eq!(tracked.state(), crate::EntityState::Modified);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn save_tracked_modified_skips_update_when_persisted_snapshot_is_unchanged() {
        let dbset = DbSet::<ExplicitLoadRoot>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(ExplicitLoadRoot {
            id: 7,
            children_loaded: 0,
        });
        tracked
            .attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();

        tracked.current_mut().children_loaded = 1;

        let saved = dbset.save_tracked_modified().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(tracked.original().children_loaded, 1);
        assert_eq!(registry.entry_count(), 1);
    }

    #[tokio::test]
    async fn save_tracked_modified_rejects_composite_primary_keys_before_sql() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());
        tracked.current_mut();

        let error = dbset.save_tracked_modified().await.unwrap_err();

        assert_eq!(tracked.state(), crate::EntityState::Modified);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            error.message(),
            "change tracking currently supports only entities with a single primary key column"
        );
    }

    #[tokio::test]
    async fn mark_unchanged_on_modified_entry_discards_pending_update_before_validation() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(CompositeKeyEntity);
        tracked.attach_registry(registry.clone());
        tracked.current_mut();

        tracked.mark_unchanged();
        let saved = dbset.save_tracked_modified().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Unchanged
        );
    }

    #[tokio::test]
    async fn save_tracked_modified_returns_zero_without_pending_modified_entries() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let tracked = dbset.add_tracked(CompositeKeyEntity);

        let saved = dbset.save_tracked_modified().await.unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 1);
    }

    #[test]
    fn reconciled_relationship_value_merge_appends_new_fk_values() {
        let values = super::merge_relationship_values(
            vec![ColumnValue::new(
                "name",
                SqlValue::String("child".to_string()),
            )],
            &[
                ColumnValue::new("root_id", SqlValue::I64(7)),
                ColumnValue::new("root_id", SqlValue::I64(7)),
            ],
        )
        .unwrap();

        assert_eq!(
            values,
            vec![
                ColumnValue::new("name", SqlValue::String("child".to_string())),
                ColumnValue::new("root_id", SqlValue::I64(7)),
            ]
        );
    }

    #[test]
    fn reconciled_relationship_value_merge_rejects_manual_fk_conflict() {
        let error = super::merge_relationship_values(
            vec![ColumnValue::new("root_id", SqlValue::I64(1))],
            &[ColumnValue::new("root_id", SqlValue::I64(2))],
        )
        .unwrap_err();

        assert_eq!(error.kind(), OrmErrorKind::Compile);
        assert_eq!(
            error.message(),
            "relationship operation assigns column `root_id` but the tracked entity already has a different value"
        );
    }

    #[tokio::test]
    async fn save_reconciled_relationship_operations_returns_zero_without_matching_operations() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let registry = dbset.tracking_registry();
        let tracked = dbset.add_tracked(CompositeKeyEntity);
        let plan = RelationshipReconciliationPlan::default();

        let saved = dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap();

        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 1);
    }

    #[tokio::test]
    async fn save_reconciled_relationship_operations_routes_insert_through_dbset_values() {
        let dbset = DbSet::<ExplicitLoadChild>::disconnected();
        let registry = dbset.tracking_registry();
        let tracked = dbset.add_tracked(ExplicitLoadChild);
        let registration_id = registry.registrations()[0].entry_id;
        let plan = RelationshipReconciliationPlan::from_operations_for_test(vec![
            ReconciledRelationshipOperation::for_test(
                registration_id,
                ReconciledRelationshipOperationKind::Insert,
                vec![ColumnValue::new("root_id", SqlValue::I64(7))],
            ),
        ]);

        let error = dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(tracked.state(), crate::EntityState::Added);
        assert_eq!(registry.entry_count(), 1);
    }

    #[tokio::test]
    async fn save_reconciled_relationship_operations_routes_update_through_dbset_values() {
        let dbset = DbSet::<ExplicitLoadChild>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(ExplicitLoadChild);
        tracked
            .attach_registry_loaded(registry.clone(), SqlValue::I64(11))
            .unwrap();
        tracked.mark_modified();
        let registration_id = registry.registrations()[0].entry_id;
        let plan = RelationshipReconciliationPlan::from_operations_for_test(vec![
            ReconciledRelationshipOperation::for_test(
                registration_id,
                ReconciledRelationshipOperationKind::Update,
                vec![ColumnValue::new("root_id", SqlValue::I64(7))],
            ),
        ]);

        let error = dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(tracked.state(), crate::EntityState::Modified);
        assert_eq!(registry.entry_count(), 1);
    }

    #[tokio::test]
    async fn save_reconciled_relationship_operations_routes_delete_through_dbset_values() {
        let dbset = DbSet::<ExplicitLoadChild>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(ExplicitLoadChild);
        tracked
            .attach_registry_loaded(registry.clone(), SqlValue::I64(11))
            .unwrap();
        tracked.mark_deleted();
        let registration_id = registry.registrations()[0].entry_id;
        let plan = RelationshipReconciliationPlan::from_operations_for_test(vec![
            ReconciledRelationshipOperation::for_test(
                registration_id,
                ReconciledRelationshipOperationKind::Delete,
                Vec::new(),
            ),
        ]);

        let error = dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);
    }

    #[tokio::test]
    async fn save_reconciled_relationship_operations_rejects_state_mismatch_before_sql() {
        let dbset = DbSet::<ExplicitLoadChild>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(ExplicitLoadChild);
        tracked
            .attach_registry_loaded(registry.clone(), SqlValue::I64(11))
            .unwrap();
        let registration_id = registry.registrations()[0].entry_id;
        let plan = RelationshipReconciliationPlan::from_operations_for_test(vec![
            ReconciledRelationshipOperation::for_test(
                registration_id,
                ReconciledRelationshipOperationKind::Insert,
                vec![ColumnValue::new("root_id", SqlValue::I64(7))],
            ),
        ]);

        let error = dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap_err();

        assert_eq!(error.kind(), OrmErrorKind::Compile);
        assert_eq!(
            error.message(),
            "reconciled relationship insert for entity `ExplicitLoadChild` requires an Added tracked entry"
        );
        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(registry.entry_count(), 1);
    }

    #[tokio::test]
    async fn has_many_relationship_commands_are_reconciled_and_dispatched_by_dependent_type() {
        let root_dbset = DbSet::<HasManyRelationshipRoot>::disconnected();
        let registry = root_dbset.tracking_registry();
        let child_dbset = DbSet::<ExplicitLoadChild> {
            connection: None,
            tracking_registry: registry.clone(),
            _entity: PhantomData,
        };
        let child = child_dbset.add_tracked(ExplicitLoadChild);
        let child_identity = RelationshipTrackedIdentity::from_tracked(&child).unwrap();
        let mut root = Tracked::from_loaded(HasManyRelationshipRoot {
            id: 7,
            navigation: &EXPLICIT_LOAD_NAVIGATIONS[0],
            changes: vec![RelationshipMutationIdentityChange::Collection(
                RelationshipCollectionIdentityChange::Added(Some(child_identity)),
            )],
        });
        root.attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();

        let commands = root_dbset
            .collect_has_many_relationship_commands(&[
                &HAS_MANY_RELATIONSHIP_ROOT_METADATA,
                &EXPLICIT_LOAD_CHILD_METADATA,
            ])
            .unwrap();
        let plan = registry.reconcile_relationship_commands(&commands).unwrap();

        assert_eq!(plan.operations().len(), 1);
        assert_eq!(
            plan.operations()[0].kind(),
            ReconciledRelationshipOperationKind::Insert
        );
        assert_eq!(
            plan.operations()[0].relationship_values(),
            &[ColumnValue::new("root_id", SqlValue::I64(7))]
        );

        let error = child_dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(child.state(), crate::EntityState::Added);
    }

    #[tokio::test]
    async fn belongs_to_relationship_move_is_reconciled_and_dispatched_by_dependent_type() {
        let dependent_dbset = DbSet::<BelongsToRelationshipChild>::disconnected();
        let registry = dependent_dbset.tracking_registry();
        let root_dbset = DbSet::<HasManyRelationshipRoot> {
            connection: None,
            tracking_registry: registry.clone(),
            _entity: PhantomData,
        };
        let mut old_root = Tracked::from_loaded(HasManyRelationshipRoot {
            id: 7,
            navigation: &EXPLICIT_LOAD_NAVIGATIONS[0],
            changes: Vec::new(),
        });
        old_root
            .attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();
        let mut new_root = Tracked::from_loaded(HasManyRelationshipRoot {
            id: 8,
            navigation: &EXPLICIT_LOAD_NAVIGATIONS[0],
            changes: Vec::new(),
        });
        new_root
            .attach_registry_loaded(registry.clone(), SqlValue::I64(8))
            .unwrap();
        let old_root_identity = RelationshipTrackedIdentity::from_tracked(&old_root).unwrap();
        let new_root_identity = RelationshipTrackedIdentity::from_tracked(&new_root).unwrap();
        let mut child = Tracked::from_loaded(BelongsToRelationshipChild {
            id: 11,
            navigation: &BELONGS_TO_RELATIONSHIP_CHILD_NAVIGATIONS[0],
            changes: vec![RelationshipMutationIdentityChange::Navigation(
                RelationshipNavigationIdentityChange::Set {
                    previous: Some(old_root_identity),
                    current: Some(new_root_identity),
                },
            )],
        });
        child
            .attach_registry_loaded(registry.clone(), SqlValue::I64(11))
            .unwrap();

        let commands = dependent_dbset
            .collect_dependent_relationship_commands::<HasManyRelationshipRoot>(&[
                &BELONGS_TO_RELATIONSHIP_CHILD_METADATA,
                &HAS_MANY_RELATIONSHIP_ROOT_METADATA,
            ])
            .unwrap();
        let plan = registry.reconcile_relationship_commands(&commands).unwrap();

        assert_eq!(child.state(), crate::EntityState::Modified);
        assert_eq!(plan.operations().len(), 1);
        assert_eq!(
            plan.operations()[0].kind(),
            ReconciledRelationshipOperationKind::Update
        );
        assert_eq!(
            plan.operations()[0].relationship_values(),
            &[ColumnValue::new("root_id", SqlValue::I64(8))]
        );

        let error = dependent_dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(root_dbset.tracking_registry().entry_count(), 3);
    }

    #[tokio::test]
    async fn belongs_to_relationship_attach_added_child_routes_insert_values() {
        let dependent_dbset = DbSet::<BelongsToRelationshipChild>::disconnected();
        let registry = dependent_dbset.tracking_registry();
        let mut root = Tracked::from_loaded(HasManyRelationshipRoot {
            id: 7,
            navigation: &EXPLICIT_LOAD_NAVIGATIONS[0],
            changes: Vec::new(),
        });
        root.attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();
        let root_identity = RelationshipTrackedIdentity::from_tracked(&root).unwrap();
        let child = dependent_dbset.add_tracked(BelongsToRelationshipChild {
            id: 11,
            navigation: &BELONGS_TO_RELATIONSHIP_CHILD_NAVIGATIONS[0],
            changes: vec![RelationshipMutationIdentityChange::Navigation(
                RelationshipNavigationIdentityChange::Set {
                    previous: None,
                    current: Some(root_identity),
                },
            )],
        });

        let commands = dependent_dbset
            .collect_dependent_relationship_commands::<HasManyRelationshipRoot>(&[
                &BELONGS_TO_RELATIONSHIP_CHILD_METADATA,
                &HAS_MANY_RELATIONSHIP_ROOT_METADATA,
            ])
            .unwrap();
        let plan = registry.reconcile_relationship_commands(&commands).unwrap();

        assert_eq!(child.state(), crate::EntityState::Added);
        assert_eq!(plan.operations().len(), 1);
        assert_eq!(
            plan.operations()[0].kind(),
            ReconciledRelationshipOperationKind::Insert
        );
        assert_eq!(
            plan.operations()[0].relationship_values(),
            &[ColumnValue::new("root_id", SqlValue::I64(7))]
        );

        let error = dependent_dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
    }

    #[test]
    fn belongs_to_relationship_collection_rejects_detached_principal_before_sql() {
        let dependent_dbset = DbSet::<BelongsToRelationshipChild>::disconnected();
        let registry = dependent_dbset.tracking_registry();
        let mut root = Tracked::from_loaded(HasManyRelationshipRoot {
            id: 7,
            navigation: &EXPLICIT_LOAD_NAVIGATIONS[0],
            changes: Vec::new(),
        });
        root.attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();
        let root_identity = RelationshipTrackedIdentity::from_tracked(&root).unwrap();
        root.detach();
        let _child = dependent_dbset.add_tracked(BelongsToRelationshipChild {
            id: 11,
            navigation: &BELONGS_TO_RELATIONSHIP_CHILD_NAVIGATIONS[0],
            changes: vec![RelationshipMutationIdentityChange::Navigation(
                RelationshipNavigationIdentityChange::Set {
                    previous: None,
                    current: Some(root_identity),
                },
            )],
        });

        let error = dependent_dbset
            .collect_dependent_relationship_commands::<HasManyRelationshipRoot>(&[
                &BELONGS_TO_RELATIONSHIP_CHILD_METADATA,
                &HAS_MANY_RELATIONSHIP_ROOT_METADATA,
            ])
            .unwrap_err();

        assert!(error.message().contains("without FK values"));
        assert_eq!(error.kind(), OrmErrorKind::Compile);
    }

    #[test]
    fn has_many_relationship_collection_rejects_required_removal_before_sql() {
        let root_dbset = DbSet::<HasManyRelationshipRoot>::disconnected();
        let registry = root_dbset.tracking_registry();
        let child_dbset = DbSet::<ExplicitLoadChild> {
            connection: None,
            tracking_registry: registry.clone(),
            _entity: PhantomData,
        };
        let mut child = Tracked::from_loaded(ExplicitLoadChild);
        child
            .attach_registry_loaded(registry.clone(), SqlValue::I64(11))
            .unwrap();
        let child_identity = RelationshipTrackedIdentity::from_tracked(&child).unwrap();
        let mut root = Tracked::from_loaded(HasManyRelationshipRoot {
            id: 7,
            navigation: &EXPLICIT_LOAD_NAVIGATIONS[0],
            changes: vec![RelationshipMutationIdentityChange::Collection(
                RelationshipCollectionIdentityChange::Removed(Some(child_identity)),
            )],
        });
        root.attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();

        let commands = root_dbset
            .collect_has_many_relationship_commands(&[
                &HAS_MANY_RELATIONSHIP_ROOT_METADATA,
                &EXPLICIT_LOAD_CHILD_METADATA,
            ])
            .unwrap();
        let error = registry
            .reconcile_relationship_commands(&commands)
            .unwrap_err();

        assert!(error.message().contains("required relationship"));
        assert_eq!(error.kind(), OrmErrorKind::Compile);
        assert_eq!(child.state(), crate::EntityState::Unchanged);
        assert_eq!(child_dbset.tracking_registry().entry_count(), 2);
    }

    #[tokio::test]
    async fn has_many_relationship_collection_routes_explicit_deleted_dependent() {
        let root_dbset = DbSet::<HasManyRelationshipRoot>::disconnected();
        let registry = root_dbset.tracking_registry();
        let child_dbset = DbSet::<ExplicitLoadChild> {
            connection: None,
            tracking_registry: registry.clone(),
            _entity: PhantomData,
        };
        let mut child = Tracked::from_loaded(ExplicitLoadChild);
        child
            .attach_registry_loaded(registry.clone(), SqlValue::I64(11))
            .unwrap();
        let child_identity = RelationshipTrackedIdentity::from_tracked(&child).unwrap();
        child.mark_deleted();
        let mut root = Tracked::from_loaded(HasManyRelationshipRoot {
            id: 7,
            navigation: &EXPLICIT_LOAD_NAVIGATIONS[0],
            changes: vec![RelationshipMutationIdentityChange::Collection(
                RelationshipCollectionIdentityChange::Removed(Some(child_identity)),
            )],
        });
        root.attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();

        let commands = root_dbset
            .collect_has_many_relationship_commands(&[
                &HAS_MANY_RELATIONSHIP_ROOT_METADATA,
                &EXPLICIT_LOAD_CHILD_METADATA,
            ])
            .unwrap();
        let plan = registry.reconcile_relationship_commands(&commands).unwrap();

        assert_eq!(plan.operations().len(), 1);
        assert_eq!(
            plan.operations()[0].kind(),
            ReconciledRelationshipOperationKind::Delete
        );

        let error = child_dbset
            .save_reconciled_relationship_operations(&plan)
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(child.state(), crate::EntityState::Deleted);
    }

    #[test]
    fn unsupported_relationship_guard_still_rejects_non_has_many_batches() {
        let dbset = DbSet::<HasManyRelationshipRoot>::disconnected();
        let registry = dbset.tracking_registry();
        let mut tracked = Tracked::from_loaded(HasManyRelationshipRoot {
            id: 7,
            navigation: &UNSUPPORTED_MANY_TO_MANY_NAVIGATION,
            changes: vec![RelationshipMutationIdentityChange::Navigation(
                RelationshipNavigationIdentityChange::Set {
                    previous: None,
                    current: None,
                },
            )],
        });
        tracked
            .attach_registry_loaded(registry.clone(), SqlValue::I64(7))
            .unwrap();

        let error = dbset
            .reject_pending_unsupported_relationship_changes()
            .unwrap_err();

        assert_eq!(error.kind(), OrmErrorKind::Compile);
        assert!(error.message().contains("unsupported relationship"));
    }

    #[test]
    fn reject_pending_relationship_changes_allows_tracked_entities_without_wrapper_mutations() {
        let dbset = DbSet::<RelationshipGuardEntity>::disconnected();
        let _tracked = dbset.add_tracked(RelationshipGuardEntity {
            pending_relationship_changes: 0,
        });

        dbset.reject_pending_relationship_changes().unwrap();
    }

    #[test]
    fn reject_pending_relationship_changes_fails_before_connection_is_required() {
        let dbset = DbSet::<RelationshipGuardEntity>::disconnected();
        let mut tracked = dbset.add_tracked(RelationshipGuardEntity {
            pending_relationship_changes: 0,
        });
        tracked.current_mut().pending_relationship_changes = 1;

        let error = dbset.reject_pending_relationship_changes().unwrap_err();

        assert_eq!(error.kind(), OrmErrorKind::Compile);
        assert_eq!(
            error.message(),
            "save_changes cannot persist relationship wrapper mutations for entity `RelationshipGuardEntity` yet because relationship command collection is not connected to execution"
        );
    }

    #[test]
    fn dbset_insert_builds_insert_query_for_entity() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let insertable = NewTestEntity {
            name: "ana".to_string(),
            active: true,
        };

        let query = dbset.insert_query(&insertable).unwrap();

        assert_eq!(
            query,
            InsertQuery {
                into: TableRef::new("dbo", "test_entities"),
                values: vec![
                    ColumnValue::new("name", SqlValue::String("ana".to_string())),
                    ColumnValue::new("active", SqlValue::Bool(true)),
                ],
                entity: Some(TestEntity::metadata()),
            }
        );
    }

    #[test]
    fn dbset_insert_appends_active_tenant_for_tenant_scoped_entities() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let insertable = NewTenantWriteEntity {
            name: "tenant row".to_string(),
            tenant_id: None,
        };
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let values = dbset
            .tenant_insert_values(insertable.values(), Some(&active_tenant))
            .unwrap();

        assert_eq!(
            values,
            vec![
                ColumnValue::new("name", SqlValue::String("tenant row".to_string())),
                ColumnValue::new("tenant_id", SqlValue::I64(42)),
            ]
        );
    }

    #[test]
    fn dbset_insert_accepts_matching_explicit_tenant_value() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let insertable = NewTenantWriteEntity {
            name: "tenant row".to_string(),
            tenant_id: Some(42),
        };
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let values = dbset
            .tenant_insert_values(insertable.values(), Some(&active_tenant))
            .unwrap();

        assert_eq!(
            values,
            vec![
                ColumnValue::new("name", SqlValue::String("tenant row".to_string())),
                ColumnValue::new("tenant_id", SqlValue::I64(42)),
            ]
        );
    }

    #[test]
    fn dbset_insert_rejects_mismatched_explicit_tenant_value() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let insertable = NewTenantWriteEntity {
            name: "tenant row".to_string(),
            tenant_id: Some(7),
        };
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let error = dbset
            .tenant_insert_values(insertable.values(), Some(&active_tenant))
            .unwrap_err();

        assert!(error.message().contains("does not match the active tenant"));
    }

    #[test]
    fn dbset_insert_fails_closed_without_active_tenant_for_tenant_scoped_entities() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let insertable = NewTenantWriteEntity {
            name: "tenant row".to_string(),
            tenant_id: None,
        };

        let error = dbset
            .tenant_insert_values(insertable.values(), None)
            .unwrap_err();

        assert!(
            error
                .message()
                .contains("tenant-scoped insert requires an active tenant")
        );
        assert_eq!(error.kind(), OrmErrorKind::Execution);
    }

    #[test]
    fn tenant_security_guardrail_keeps_write_sql_tenant_scoped() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let provider = TestSoftDeleteProvider;
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let insert_values = dbset
            .tenant_insert_values(
                vec![ColumnValue::new(
                    "name",
                    SqlValue::String("tenant row".to_string()),
                )],
                Some(&active_tenant),
            )
            .unwrap();
        let insert = super::SqlServerCompiler::compile_insert(&InsertQuery {
            into: TableRef::for_entity::<TenantWriteEntity>(),
            values: insert_values,
            entity: Some(TenantWriteEntity::metadata()),
        })
        .unwrap();
        let update = super::SqlServerCompiler::compile_update(
            &dbset
                .update_query_sql_value_with_active_tenant(
                    SqlValue::I64(7),
                    vec![ColumnValue::new(
                        "name",
                        SqlValue::String("tenant row updated".to_string()),
                    )],
                    None,
                    Some(&active_tenant),
                )
                .unwrap(),
        )
        .unwrap();
        let delete = super::SqlServerCompiler::compile_delete(
            &dbset
                .delete_query_sql_value_with_active_tenant(
                    SqlValue::I64(7),
                    None,
                    Some(&active_tenant),
                )
                .unwrap(),
        )
        .unwrap();
        let soft_delete = dbset
            .delete_compiled_query_sql_value_with_active_tenant(
                SqlValue::I64(7),
                Some(SqlValue::Bytes(vec![9, 8, 7])),
                Some(&provider),
                None,
                Some(&active_tenant),
            )
            .unwrap();

        assert_eq!(
            insert.sql,
            "INSERT INTO [dbo].[tenant_write_entities] ([name], [tenant_id]) OUTPUT INSERTED.* VALUES (@P1, @P2)"
        );
        assert_eq!(
            insert.params,
            vec![
                SqlValue::String("tenant row".to_string()),
                SqlValue::I64(42),
            ]
        );

        for compiled in [&update, &delete, &soft_delete] {
            assert!(
                compiled
                    .sql
                    .contains("[dbo].[tenant_write_entities].[tenant_id] = @P"),
                "tenant-scoped write SQL must include tenant predicate: {}",
                compiled.sql
            );
            assert!(
                compiled.params.contains(&SqlValue::I64(42)),
                "tenant-scoped write params must include active tenant value: {:?}",
                compiled.params
            );
        }

        assert!(
            !delete.sql.contains("OUTPUT INSERTED.*"),
            "physical delete should stay a DELETE statement while still tenant-scoped"
        );
        assert!(
            soft_delete.sql.starts_with("UPDATE "),
            "soft_delete route should remain logical UPDATE while tenant-scoped"
        );
    }

    #[test]
    fn dbset_update_builds_update_query_for_entity_and_primary_key() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let changeset = UpdateTestEntity {
            name: Some("ana maria".to_string()),
            active: Some(false),
        };

        let query = dbset.update_query(7_i64, &changeset).unwrap();

        assert_eq!(
            query,
            UpdateQuery::for_entity::<TestEntity, _>(&changeset).filter(Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::new("dbo", "test_entities"),
                    "id",
                    "id",
                )),
                Expr::Value(SqlValue::I64(7)),
            ))
        );
    }

    #[test]
    fn dbset_update_rejects_composite_primary_keys() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();
        let changeset = UpdateTestEntity {
            name: Some("ana".to_string()),
            active: None,
        };

        let error = dbset.update_query(7_i64, &changeset).unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet currently supports this operation only for entities with a single primary key column"
        );
        assert_eq!(error.kind(), OrmErrorKind::Compile);
    }

    #[test]
    fn dbset_update_appends_rowversion_predicate_when_changeset_has_token() {
        let dbset = DbSet::<VersionedEntity>::disconnected();
        let changeset = UpdateVersionedEntity {
            name: Some("ana maria".to_string()),
            version: Some(vec![1, 2, 3, 4]),
        };

        let query = dbset.update_query(7_i64, &changeset).unwrap();

        assert_eq!(
            query,
            UpdateQuery::for_entity::<VersionedEntity, _>(&changeset).filter(Predicate::and(vec![
                Predicate::eq(
                    Expr::Column(ColumnRef::new(
                        TableRef::new("dbo", "versioned_entities"),
                        "id",
                        "id",
                    )),
                    Expr::Value(SqlValue::I64(7)),
                ),
                Predicate::eq(
                    Expr::Column(ColumnRef::new(
                        TableRef::new("dbo", "versioned_entities"),
                        "version",
                        "version",
                    )),
                    Expr::Value(SqlValue::Bytes(vec![1, 2, 3, 4])),
                ),
            ]))
        );
    }

    #[test]
    fn dbset_update_appends_tenant_filter_before_rowversion_for_tenant_scoped_entities() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let changes = vec![ColumnValue::new(
            "name",
            SqlValue::String("tenant row".to_string()),
        )];
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let query = dbset
            .update_query_sql_value_with_active_tenant(
                SqlValue::I64(7),
                changes,
                Some(SqlValue::Bytes(vec![1, 2, 3, 4])),
                Some(&active_tenant),
            )
            .unwrap();
        let compiled = super::SqlServerCompiler::compile_update(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [dbo].[tenant_write_entities] SET [name] = @P1 OUTPUT INSERTED.* WHERE ((([dbo].[tenant_write_entities].[id] = @P2) AND ([dbo].[tenant_write_entities].[tenant_id] = @P3)) AND ([dbo].[tenant_write_entities].[version] = @P4))"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("tenant row".to_string()),
                SqlValue::I64(7),
                SqlValue::I64(42),
                SqlValue::Bytes(vec![1, 2, 3, 4]),
            ]
        );
    }

    #[test]
    fn save_changes_modified_route_preserves_audit_request_values_before_provider_values() {
        let dbset = DbSet::<AuditedWriteEntity>::disconnected();
        let request_values = AuditRequestValues::new(vec![ColumnValue::new(
            "updated_by",
            SqlValue::String("request-user".to_string()),
        )]);

        let query = dbset
            .update_query_sql_value_with_audit_runtime(
                SqlValue::I64(7),
                vec![ColumnValue::new(
                    "name",
                    SqlValue::String("tracked audited row".to_string()),
                )],
                None,
                None,
                Some(&TestAuditProvider),
                Some(&request_values),
            )
            .unwrap();
        let compiled = super::SqlServerCompiler::compile_update(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [dbo].[audited_write_entities] SET [name] = @P1, [updated_by] = @P2 OUTPUT INSERTED.* WHERE ([dbo].[audited_write_entities].[id] = @P3)"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("tracked audited row".to_string()),
                SqlValue::String("request-user".to_string()),
                SqlValue::I64(7),
            ]
        );
    }

    #[test]
    fn save_changes_modified_route_preserves_tenant_and_rowversion_predicates() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let query = dbset
            .update_query_sql_value_with_active_tenant(
                SqlValue::I64(7),
                vec![ColumnValue::new(
                    "name",
                    SqlValue::String("tracked tenant row".to_string()),
                )],
                Some(SqlValue::Bytes(vec![1, 2, 3, 4])),
                Some(&active_tenant),
            )
            .unwrap();
        let compiled = super::SqlServerCompiler::compile_update(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [dbo].[tenant_write_entities] SET [name] = @P1 OUTPUT INSERTED.* WHERE ((([dbo].[tenant_write_entities].[id] = @P2) AND ([dbo].[tenant_write_entities].[tenant_id] = @P3)) AND ([dbo].[tenant_write_entities].[version] = @P4))"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("tracked tenant row".to_string()),
                SqlValue::I64(7),
                SqlValue::I64(42),
                SqlValue::Bytes(vec![1, 2, 3, 4]),
            ]
        );
    }

    #[test]
    fn dbset_update_applies_audit_provider_values_before_compiling_update() {
        let dbset = DbSet::<AuditedWriteEntity>::disconnected();
        let provider = TestAuditProvider;

        let query = dbset
            .update_query_sql_value_with_audit_runtime(
                SqlValue::I64(7),
                vec![ColumnValue::new(
                    "name",
                    SqlValue::String("audited row".to_string()),
                )],
                None,
                None,
                Some(&provider),
                None,
            )
            .unwrap();
        let compiled = super::SqlServerCompiler::compile_update(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [dbo].[audited_write_entities] SET [name] = @P1, [updated_by] = @P2 OUTPUT INSERTED.* WHERE ([dbo].[audited_write_entities].[id] = @P3)"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("audited row".to_string()),
                SqlValue::String("audit-provider".to_string()),
                SqlValue::I64(7),
            ]
        );
    }

    #[test]
    fn save_changes_added_route_preserves_audit_request_values_before_provider_values() {
        struct InsertAuditProvider;

        impl AuditProvider for InsertAuditProvider {
            fn values(
                &self,
                context: crate::AuditContext<'_>,
            ) -> Result<Vec<ColumnValue>, OrmError> {
                assert_eq!(context.entity.table, "audited_write_entities");
                assert_eq!(context.operation, AuditOperation::Insert);
                assert!(context.request_values.is_some());

                Ok(vec![ColumnValue::new(
                    "updated_by",
                    SqlValue::String("provider-user".to_string()),
                )])
            }
        }

        let dbset = DbSet::<AuditedWriteEntity>::disconnected();
        let request_values = AuditRequestValues::new(vec![ColumnValue::new(
            "updated_by",
            SqlValue::String("request-user".to_string()),
        )]);

        let query = dbset
            .insert_query_values_with_runtime_for_test(
                vec![ColumnValue::new(
                    "name",
                    SqlValue::String("tracked audited insert".to_string()),
                )],
                Some(&InsertAuditProvider),
                Some(&request_values),
            )
            .unwrap();
        let compiled = super::SqlServerCompiler::compile_insert(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "INSERT INTO [dbo].[audited_write_entities] ([name], [updated_by]) OUTPUT INSERTED.* VALUES (@P1, @P2)"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("tracked audited insert".to_string()),
                SqlValue::String("request-user".to_string()),
            ]
        );
    }

    #[test]
    fn dbset_insert_applies_audit_request_values_before_provider_values() {
        struct InsertAuditProvider;

        impl AuditProvider for InsertAuditProvider {
            fn values(
                &self,
                context: crate::AuditContext<'_>,
            ) -> Result<Vec<ColumnValue>, OrmError> {
                assert_eq!(context.entity.table, "audited_write_entities");
                assert_eq!(context.operation, AuditOperation::Insert);
                assert!(context.request_values.is_some());

                Ok(vec![ColumnValue::new(
                    "updated_by",
                    SqlValue::String("provider".to_string()),
                )])
            }
        }

        let dbset = DbSet::<AuditedWriteEntity>::disconnected();
        let request_values = AuditRequestValues::new(vec![ColumnValue::new(
            "updated_by",
            SqlValue::String("request".to_string()),
        )]);

        let query = dbset
            .insert_query_values_with_runtime_for_test(
                vec![ColumnValue::new(
                    "name",
                    SqlValue::String("audited insert".to_string()),
                )],
                Some(&InsertAuditProvider),
                Some(&request_values),
            )
            .unwrap();
        let compiled = super::SqlServerCompiler::compile_insert(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "INSERT INTO [dbo].[audited_write_entities] ([name], [updated_by]) OUTPUT INSERTED.* VALUES (@P1, @P2)"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("audited insert".to_string()),
                SqlValue::String("request".to_string()),
            ]
        );
    }

    #[test]
    fn dbset_update_fails_closed_without_active_tenant_for_tenant_scoped_entities() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();

        let error = dbset
            .update_query_sql_value_with_active_tenant(
                SqlValue::I64(7),
                vec![ColumnValue::new(
                    "name",
                    SqlValue::String("blocked".to_string()),
                )],
                None,
                None,
            )
            .unwrap_err();

        assert!(
            error
                .message()
                .contains("tenant-scoped write requires an active tenant")
        );
    }

    #[test]
    fn save_changes_deleted_route_preserves_soft_delete_request_tenant_and_rowversion() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let request_values = SoftDeleteRequestValues::new(vec![ColumnValue::new(
            "deleted_at",
            SqlValue::String("2026-05-07T00:00:00".to_string()),
        )]);
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let compiled = dbset
            .delete_compiled_query_sql_value_with_active_tenant(
                SqlValue::I64(7),
                Some(SqlValue::Bytes(vec![9, 8, 7])),
                None,
                Some(&request_values),
                Some(&active_tenant),
            )
            .unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [dbo].[tenant_write_entities] SET [deleted_at] = @P1 OUTPUT INSERTED.* WHERE ((([dbo].[tenant_write_entities].[id] = @P2) AND ([dbo].[tenant_write_entities].[tenant_id] = @P3)) AND ([dbo].[tenant_write_entities].[version] = @P4))"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("2026-05-07T00:00:00".to_string()),
                SqlValue::I64(7),
                SqlValue::I64(42),
                SqlValue::Bytes(vec![9, 8, 7]),
            ]
        );
    }

    #[test]
    fn dbset_delete_builds_delete_query_for_entity_and_primary_key() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset.delete_query(7_i64).unwrap();

        assert_eq!(
            query,
            DeleteQuery::from_entity::<TestEntity>().filter(Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::new("dbo", "test_entities"),
                    "id",
                    "id",
                )),
                Expr::Value(SqlValue::I64(7)),
            ))
        );
    }

    #[test]
    fn dbset_delete_query_sql_value_builds_delete_query_for_entity_and_primary_key() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset
            .delete_query_sql_value(SqlValue::I64(7), None)
            .unwrap();

        assert_eq!(
            query,
            DeleteQuery::from_entity::<TestEntity>().filter(Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::new("dbo", "test_entities"),
                    "id",
                    "id",
                )),
                Expr::Value(SqlValue::I64(7)),
            ))
        );
    }

    #[test]
    fn dbset_delete_query_sql_value_appends_rowversion_predicate_when_present() {
        let dbset = DbSet::<VersionedEntity>::disconnected();

        let query = dbset
            .delete_query_sql_value(SqlValue::I64(7), Some(SqlValue::Bytes(vec![9, 8, 7])))
            .unwrap();

        assert_eq!(
            query,
            DeleteQuery::from_entity::<VersionedEntity>().filter(Predicate::and(vec![
                Predicate::eq(
                    Expr::Column(ColumnRef::new(
                        TableRef::new("dbo", "versioned_entities"),
                        "id",
                        "id",
                    )),
                    Expr::Value(SqlValue::I64(7)),
                ),
                Predicate::eq(
                    Expr::Column(ColumnRef::new(
                        TableRef::new("dbo", "versioned_entities"),
                        "version",
                        "version",
                    )),
                    Expr::Value(SqlValue::Bytes(vec![9, 8, 7])),
                ),
            ]))
        );
    }

    #[test]
    fn dbset_delete_appends_tenant_filter_for_tenant_scoped_entities() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let query = dbset
            .delete_query_sql_value_with_active_tenant(SqlValue::I64(7), None, Some(&active_tenant))
            .unwrap();
        let compiled = super::SqlServerCompiler::compile_delete(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "DELETE FROM [dbo].[tenant_write_entities] WHERE (([dbo].[tenant_write_entities].[id] = @P1) AND ([dbo].[tenant_write_entities].[tenant_id] = @P2))"
        );
        assert_eq!(compiled.params, vec![SqlValue::I64(7), SqlValue::I64(42)]);
    }

    #[test]
    fn dbset_delete_compiled_query_uses_physical_delete_for_plain_entities() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let compiled = dbset
            .delete_compiled_query_sql_value(SqlValue::I64(7), None, None, None)
            .unwrap();

        assert_eq!(
            compiled.sql,
            "DELETE FROM [dbo].[test_entities] WHERE ([dbo].[test_entities].[id] = @P1)"
        );
        assert_eq!(compiled.params, vec![SqlValue::I64(7)]);
    }

    #[test]
    fn dbset_delete_compiled_query_uses_update_for_soft_delete_entities() {
        let dbset = DbSet::<SoftDeleteEntityUnderTest>::disconnected();

        let provider = TestSoftDeleteProvider;
        let compiled = dbset
            .delete_compiled_query_sql_value(SqlValue::I64(7), None, Some(&provider), None)
            .unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [dbo].[soft_delete_entities] SET [deleted_at] = @P1 OUTPUT INSERTED.* WHERE ([dbo].[soft_delete_entities].[id] = @P2)"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("2026-04-25T00:00:00".to_string()),
                SqlValue::I64(7),
            ]
        );
    }

    #[test]
    fn dbset_delete_compiled_query_appends_rowversion_for_soft_delete_entities() {
        let dbset = DbSet::<SoftDeleteVersionedEntity>::disconnected();

        let provider = TestSoftDeleteProvider;
        let compiled = dbset
            .delete_compiled_query_sql_value(
                SqlValue::I64(7),
                Some(SqlValue::Bytes(vec![9, 8, 7])),
                Some(&provider),
                None,
            )
            .unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [dbo].[soft_delete_versioned_entities] SET [deleted_at] = @P1 OUTPUT INSERTED.* WHERE (([dbo].[soft_delete_versioned_entities].[id] = @P2) AND ([dbo].[soft_delete_versioned_entities].[version] = @P3))"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("2026-04-25T00:00:00".to_string()),
                SqlValue::I64(7),
                SqlValue::Bytes(vec![9, 8, 7]),
            ]
        );
    }

    #[test]
    fn dbset_soft_delete_appends_tenant_filter_for_tenant_scoped_entities() {
        let dbset = DbSet::<TenantWriteEntity>::disconnected();
        let provider = TestSoftDeleteProvider;
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };

        let compiled = dbset
            .delete_compiled_query_sql_value_with_active_tenant(
                SqlValue::I64(7),
                Some(SqlValue::Bytes(vec![9, 8, 7])),
                Some(&provider),
                None,
                Some(&active_tenant),
            )
            .unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [dbo].[tenant_write_entities] SET [deleted_at] = @P1 OUTPUT INSERTED.* WHERE ((([dbo].[tenant_write_entities].[id] = @P2) AND ([dbo].[tenant_write_entities].[tenant_id] = @P3)) AND ([dbo].[tenant_write_entities].[version] = @P4))"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("2026-04-25T00:00:00".to_string()),
                SqlValue::I64(7),
                SqlValue::I64(42),
                SqlValue::Bytes(vec![9, 8, 7]),
            ]
        );
    }

    #[test]
    fn dbset_delete_compiled_query_rejects_soft_delete_without_runtime_values() {
        let dbset = DbSet::<SoftDeleteEntityUnderTest>::disconnected();

        let error = dbset
            .delete_compiled_query_sql_value(SqlValue::I64(7), None, None, None)
            .unwrap_err();

        assert_eq!(
            error.message(),
            "soft_delete delete requires at least one runtime change"
        );
        assert_eq!(error.kind(), OrmErrorKind::Compile);
    }

    #[test]
    fn soft_delete_security_guardrail_keeps_schema_and_delete_paths_logical() {
        let current = ModelSnapshot::from_entities(&[SoftDeleteEntityUnderTest::metadata()]);
        let previous = ModelSnapshot::new(vec![SchemaSnapshot::new(
            "dbo",
            vec![TableSnapshot::new(
                "soft_delete_entities",
                vec![
                    ColumnSnapshot::from(&SOFT_DELETE_ENTITY_COLUMNS[0]),
                    ColumnSnapshot::from(&SOFT_DELETE_ENTITY_COLUMNS[1]),
                ],
                None,
                vec!["id".to_string()],
                vec![],
                vec![],
            )],
        )]);
        let schema_operations =
            diff_schema_and_table_operations(&ModelSnapshot::default(), &current);
        let column_operations = diff_column_operations(&previous, &current);

        let current_schema = current.schema("dbo").expect("dbo schema should exist");
        let table = current_schema
            .table("soft_delete_entities")
            .expect("soft delete table should exist");
        let deleted_at = table
            .column("deleted_at")
            .expect("soft delete column should be ordinary snapshot metadata");

        assert_eq!(deleted_at.sql_type, SqlServerType::DateTime2);
        assert!(deleted_at.nullable);
        assert!(!deleted_at.insertable);
        assert!(deleted_at.updatable);
        assert!(
            schema_operations
                .iter()
                .any(|operation| matches!(operation, MigrationOperation::CreateTable(operation) if operation.table.name == "soft_delete_entities")),
            "soft_delete entities should create tables through the normal migration pipeline"
        );
        assert!(
            column_operations
                .iter()
                .any(|operation| matches!(operation, MigrationOperation::AddColumn(operation) if operation.column.name == "deleted_at")),
            "activating soft_delete should surface generated columns as AddColumn"
        );

        let provider = TestSoftDeleteProvider;
        let compiled = DbSet::<SoftDeleteEntityUnderTest>::disconnected()
            .delete_compiled_query_sql_value(SqlValue::I64(7), None, Some(&provider), None)
            .expect("soft delete should compile as logical update");

        assert!(
            compiled.sql.starts_with("UPDATE "),
            "soft_delete delete route must compile to UPDATE, got {}",
            compiled.sql
        );
        assert!(
            !compiled.sql.starts_with("DELETE "),
            "soft_delete delete route must never compile to physical DELETE"
        );
        assert!(compiled.sql.contains("[deleted_at] = @P1"));
    }

    #[test]
    fn dbset_delete_rejects_composite_primary_keys() {
        let dbset = DbSet::<CompositeKeyEntity>::disconnected();

        let error = dbset.delete_query(7_i64).unwrap_err();

        assert_eq!(
            error.message(),
            "DbSet currently supports this operation only for entities with a single primary key column"
        );
        assert_eq!(error.kind(), OrmErrorKind::Compile);
    }
}
