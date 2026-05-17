//! Public API surface for the SQL Server code-first ORM.
//!
//! Most applications should import [`prelude`] and define entities with the
//! derive macros re-exported there. The crate root also exposes advanced
//! modules for users who need direct access to metadata, query ASTs,
//! migrations, SQL Server compilation, or the Tiberius adapter.

extern crate self as sql_orm;

mod active_record;
mod audit_runtime;
mod context;
mod dbset_query;
mod page_request;
mod predicate_composition;
mod query_alias;
mod query_order;
mod query_predicates;
mod query_projection;
mod raw_sql;
mod soft_delete_runtime;
mod tracking;

pub use sql_orm_core as core;
pub use sql_orm_macros as macros;
pub use sql_orm_migrate as migrate;
pub use sql_orm_query as query;
pub use sql_orm_sqlserver as sqlserver;
pub use sql_orm_tiberius as tiberius;
pub use tokio;

pub use active_record::{ActiveRecord, EntityPersist, EntityPersistMode, EntityPrimaryKey};
pub use audit_runtime::{
    AuditContext, AuditOperation, AuditProvider, AuditRequestValues, AuditValues,
    resolve_audit_values,
};
#[cfg(feature = "pool-bb8")]
pub use context::connect_shared_from_pool;
pub use context::{
    ActiveTenant, DbContext, DbContextEntitySet, DbSet, SharedConnection, connect_shared,
    connect_shared_with_config, connect_shared_with_options,
};
pub use dbset_query::{
    AggregateProjections, CollectionIncludeStrategy, DbSetGroupedQuery, DbSetQuery,
    DbSetQueryIncludeMany, DbSetQueryIncludeOne, GroupByExpressions,
};
pub use page_request::PageRequest;
pub use predicate_composition::PredicateCompositionExt;
pub use query_alias::{AliasedEntityColumn, EntityColumnAliasExt};
pub use query_order::EntityColumnOrderExt;
pub use query_predicates::EntityColumnPredicateExt;
pub use query_projection::SelectProjections;
pub use raw_sql::{QueryHint, RawCommand, RawParam, RawParams, RawQuery, RawSqlExecution};
pub use soft_delete_runtime::{
    SoftDeleteContext, SoftDeleteOperation, SoftDeleteProvider, SoftDeleteRequestValues,
    SoftDeleteValues,
};
pub use sql_orm_core::{EntityMetadata, NavigationKind, NavigationMetadata};
pub use sql_orm_query::{
    AggregateExpr, AggregateOrderBy, AggregatePredicate, AggregateProjection, QueryExecution,
    SqlFunction,
};
pub use sql_orm_tiberius::{
    MssqlConnectionConfig, MssqlHealthCheckOptions, MssqlHealthCheckQuery, MssqlOperationalOptions,
    MssqlParameterLogMode, MssqlPoolBackend, MssqlPoolOptions, MssqlRetryOptions,
    MssqlSlowQueryOptions, MssqlTimeoutOptions, MssqlTracingOptions,
};
#[cfg(feature = "pool-bb8")]
pub use sql_orm_tiberius::{MssqlPool, MssqlPoolBuilder, MssqlPooledConnection};
pub use tracking::{EntityState, Tracked};
#[doc(hidden)]
pub use tracking::{
    SaveChangesOperationPlan, TrackedEntityRegistration, TrackingRegistry, TrackingRegistryHandle,
    save_changes_operation_plan,
};

/// Provides entity metadata for code-first migration snapshot generation.
///
/// `#[derive(DbContext)]` implements this trait for application contexts by
/// returning the metadata for every `DbSet<T>` declared on the context.
pub trait MigrationModelSource {
    /// Returns the static metadata for all entities owned by the context.
    fn entity_metadata() -> &'static [&'static EntityMetadata];
}

/// Runtime metadata hook for entities that declare `#[orm(audit = Audit)]`.
///
/// The derive macro implements this for every entity. Entities without audit
/// policy return `None`; audited entities return the audit-owned columns as an
/// `EntityPolicyMetadata` view without changing the normal entity metadata
/// shape used by snapshots, diffs, and DDL.
pub trait AuditEntity: core::Entity {
    /// Returns audit-owned columns for this entity when audit is enabled.
    fn audit_policy() -> Option<core::EntityPolicyMetadata>;
}

/// Runtime metadata hook for entities that declare
/// `#[orm(soft_delete = SoftDelete)]`.
///
/// The public delete/read behavior lives in the `sql-orm` crate. Lower
/// layers still see ordinary columns and ordinary query/update AST nodes.
pub trait SoftDeleteEntity: core::Entity {
    /// Returns soft-delete-owned columns for this entity when enabled.
    fn soft_delete_policy() -> Option<core::EntityPolicyMetadata>;
}

/// Runtime value shape for the active tenant configured on a context.
///
/// `#[derive(TenantContext)]` implements this trait for user-defined structs
/// with exactly one field. The field defines both the tenant column name and
/// the SQL value used by tenant-scoped reads and writes.
pub trait TenantContext: core::EntityPolicy {
    /// Physical column name used by tenant-scoped entities.
    const COLUMN_NAME: &'static str;

    /// Converts the active tenant value into the SQL value compared in queries.
    fn tenant_value(&self) -> core::SqlValue;
}

/// Runtime metadata hook for entities that opt into tenant scoping.
///
/// Entities without `#[orm(tenant = CurrentTenant)]` return `None` and remain
/// cross-tenant even when a context has an active tenant configured.
pub trait TenantScopedEntity: core::Entity {
    /// Returns tenant-owned column metadata for tenant-scoped entities.
    fn tenant_policy() -> Option<core::EntityPolicyMetadata>;
}

/// Contract generated for entities that can receive an included single
/// navigation value.
///
/// This is used by `DbSetQuery::include::<T>(...)` for `belongs_to` and
/// `has_one` navigations. Collection includes use a different loading strategy
/// and are intentionally not part of this contract.
pub trait IncludeNavigation<T>: core::Entity {
    /// Attaches a loaded navigation value to the field named by `navigation`.
    fn set_included_navigation(
        &mut self,
        navigation: &str,
        value: Option<T>,
    ) -> Result<(), core::OrmError>;
}

/// Contract generated for entities that can receive an included collection
/// navigation value.
///
/// This is used by `DbSetQuery::include_many::<T>(...)` for `has_many`
/// navigations. The query layer groups duplicate root rows before assigning
/// the loaded collection.
pub trait IncludeCollection<T>: core::Entity {
    /// Attaches loaded collection values to the field named by `navigation`.
    fn set_included_collection(
        &mut self,
        navigation: &str,
        values: Vec<T>,
    ) -> Result<(), core::OrmError>;
}

/// Marker value for a single related entity navigation.
///
/// Navigation fields are not persisted as columns. They exist so
/// `#[derive(Entity)]` can attach navigation metadata to the entity while
/// future loading APIs decide explicitly when related rows are fetched.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Navigation<T> {
    value: Option<T>,
}

impl<T> Navigation<T> {
    /// Creates an empty navigation value.
    pub const fn empty() -> Self {
        Self { value: None }
    }

    /// Creates a navigation value containing a loaded related entity.
    pub fn loaded(value: T) -> Self {
        Self { value: Some(value) }
    }

    /// Creates a navigation value from an optional related entity.
    pub fn from_option(value: Option<T>) -> Self {
        Self { value }
    }

    /// Returns the loaded related entity when one has been attached.
    pub fn as_ref(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Replaces the loaded related entity.
    pub fn set(&mut self, value: Option<T>) {
        self.value = value;
    }
}

impl<T> Default for Navigation<T> {
    fn default() -> Self {
        Self::empty()
    }
}

/// Opt-in lazy single navigation wrapper.
///
/// This type never performs I/O by itself. It only records whether a related
/// value has been explicitly loaded by an ORM operation such as `include(...)`
/// or a future explicit lazy-loading API that receives a context-bearing value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyNavigation<T> {
    value: Option<T>,
    loaded: bool,
}

impl<T> LazyNavigation<T> {
    /// Creates an unloaded lazy navigation.
    pub const fn unloaded() -> Self {
        Self {
            value: None,
            loaded: false,
        }
    }

    /// Creates a loaded lazy navigation containing a related entity.
    pub fn loaded(value: T) -> Self {
        Self {
            value: Some(value),
            loaded: true,
        }
    }

    /// Creates a loaded lazy navigation from an optional related entity.
    pub fn from_option(value: Option<T>) -> Self {
        Self {
            value,
            loaded: true,
        }
    }

    /// Returns whether a load operation has populated this wrapper.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Returns the loaded related entity when one is present.
    ///
    /// This is a memory-only accessor. It never executes SQL.
    pub fn as_ref(&self) -> Option<&T> {
        self.value.as_ref()
    }

    /// Replaces the loaded value and marks this wrapper as loaded.
    pub fn set_loaded(&mut self, value: Option<T>) {
        self.value = value;
        self.loaded = true;
    }

    /// Clears the cached value and marks this wrapper as unloaded.
    pub fn clear(&mut self) {
        self.value = None;
        self.loaded = false;
    }
}

impl<T> Default for LazyNavigation<T> {
    fn default() -> Self {
        Self::unloaded()
    }
}

/// Marker value for a collection navigation.
///
/// Collection navigation fields are ignored by column metadata and start empty
/// when an entity is materialized without an explicit include/load operation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Collection<T> {
    values: Vec<T>,
}

impl<T> Collection<T> {
    /// Creates an empty collection navigation.
    pub const fn empty() -> Self {
        Self { values: Vec::new() }
    }

    /// Creates a loaded collection navigation from existing values.
    pub fn from_vec(values: Vec<T>) -> Self {
        Self { values }
    }

    /// Returns the loaded related entities.
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }
}

impl<T> Default for Collection<T> {
    fn default() -> Self {
        Self { values: Vec::new() }
    }
}

/// Opt-in lazy collection navigation wrapper.
///
/// This type stores loaded values and load state, but it never owns a database
/// context and never performs I/O from accessors, formatting, cloning or
/// comparison. Loading must happen through an explicit ORM method.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LazyCollection<T> {
    values: Vec<T>,
    loaded: bool,
}

impl<T> LazyCollection<T> {
    /// Creates an unloaded lazy collection.
    pub const fn unloaded() -> Self {
        Self {
            values: Vec::new(),
            loaded: false,
        }
    }

    /// Creates a loaded lazy collection from existing values.
    pub fn from_vec(values: Vec<T>) -> Self {
        Self {
            values,
            loaded: true,
        }
    }

    /// Returns whether a load operation has populated this wrapper.
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }

    /// Returns loaded related entities.
    ///
    /// This is a memory-only accessor. It never executes SQL.
    pub fn as_slice(&self) -> &[T] {
        &self.values
    }

    /// Replaces the loaded values and marks this wrapper as loaded.
    pub fn set_loaded(&mut self, values: Vec<T>) {
        self.values = values;
        self.loaded = true;
    }

    /// Clears the cached values and marks this wrapper as unloaded.
    pub fn clear(&mut self) {
        self.values.clear();
        self.loaded = false;
    }
}

impl<T> Default for LazyCollection<T> {
    fn default() -> Self {
        Self::unloaded()
    }
}

/// Builds a model snapshot from a context type that exposes entity metadata.
///
/// This is the helper used by consumer snapshot-export binaries.
pub fn model_snapshot_from_source<S: MigrationModelSource>() -> migrate::ModelSnapshot {
    migrate::ModelSnapshot::from_entities(S::entity_metadata())
}

/// Serializes the current model snapshot for a context as pretty JSON.
///
/// Consumer projects can print this from a small binary and pass it to the CLI
/// through `migration add --snapshot-bin`.
pub fn model_snapshot_json_from_source<S: MigrationModelSource>() -> Result<String, core::OrmError>
{
    model_snapshot_from_source::<S>().to_json_pretty()
}

pub mod prelude {
    pub use crate::AliasedEntityColumn;
    #[cfg(feature = "pool-bb8")]
    pub use crate::connect_shared_from_pool;
    pub use crate::{
        ActiveRecord, ActiveTenant, AggregateProjections, AuditEntity, Collection,
        CollectionIncludeStrategy, DbContext, DbContextEntitySet, DbSet, DbSetGroupedQuery,
        DbSetQuery, DbSetQueryIncludeMany, DbSetQueryIncludeOne, EntityColumnAliasExt,
        EntityColumnOrderExt, EntityColumnPredicateExt, EntityState, GroupByExpressions,
        IncludeCollection, IncludeNavigation, LazyCollection, LazyNavigation, MigrationModelSource,
        MssqlConnectionConfig, MssqlHealthCheckOptions, MssqlHealthCheckQuery,
        MssqlOperationalOptions, MssqlParameterLogMode, MssqlPoolBackend, MssqlPoolOptions,
        MssqlRetryOptions, MssqlSlowQueryOptions, MssqlTimeoutOptions, MssqlTracingOptions,
        Navigation, PageRequest, PredicateCompositionExt, QueryHint, RawCommand, RawParam,
        RawParams, RawQuery, RawSqlExecution, SelectProjections, SharedConnection,
        SoftDeleteContext, SoftDeleteEntity, SoftDeleteOperation, SoftDeleteProvider,
        SoftDeleteRequestValues, SoftDeleteValues, TenantContext, TenantScopedEntity, Tracked,
        model_snapshot_from_source, model_snapshot_json_from_source,
    };
    pub use crate::{
        AuditContext, AuditOperation, AuditProvider, AuditRequestValues, AuditValues,
        resolve_audit_values,
    };
    #[cfg(feature = "pool-bb8")]
    pub use crate::{MssqlPool, MssqlPoolBuilder, MssqlPooledConnection};
    pub use sql_orm_core::{
        Changeset, ColumnMetadata, ColumnValue, Entity, EntityColumn, EntityMetadata, EntityPolicy,
        EntityPolicyMetadata, ForeignKeyMetadata, FromRow, IdentityMetadata, IndexColumnMetadata,
        IndexMetadata, Insertable, NavigationKind, NavigationMetadata, OrmError,
        PrimaryKeyMetadata, ReferentialAction, Row, SqlServerType, SqlTypeMapping, SqlValue,
    };
    pub use sql_orm_macros::{
        AuditFields, Changeset, DbContext, Entity, FromRow, Insertable, SoftDeleteFields,
        TenantContext,
    };
    pub use sql_orm_query::{
        AggregateExpr, AggregateOrderBy, AggregatePredicate, AggregateProjection, Join, JoinType,
        QueryExecution, SelectProjection, SqlFunction,
    };
}

#[cfg(test)]
mod tests {
    use super::prelude::{
        ActiveRecord, ActiveTenant, AuditContext, AuditEntity, AuditFields, AuditOperation,
        AuditProvider, AuditRequestValues, AuditValues, Changeset, ColumnValue, DbContext,
        DbContextEntitySet, DbSet, Entity, EntityColumn, EntityColumnOrderExt,
        EntityColumnPredicateExt, EntityMetadata, EntityPolicy, EntityPolicyMetadata, EntityState,
        IdentityMetadata, Insertable, LazyCollection, LazyNavigation, MssqlConnectionConfig,
        MssqlOperationalOptions, MssqlPoolBackend, MssqlPoolOptions, MssqlRetryOptions,
        MssqlTimeoutOptions, NavigationKind, NavigationMetadata, OrmError, PageRequest,
        PredicateCompositionExt, PrimaryKeyMetadata, QueryExecution, QueryHint, RawCommand,
        RawParam, RawParams, RawQuery, RawSqlExecution, SelectProjection, SelectProjections,
        SharedConnection, SoftDeleteEntity, SoftDeleteFields, SqlServerType, SqlTypeMapping,
        SqlValue, TenantContext, TenantScopedEntity, Tracked,
    };
    use sql_orm_query::{Expr, OrderBy, Predicate, SortDirection, TableRef};
    use std::time::Duration;

    struct PublicEntity;

    static PUBLIC_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "PublicEntity",
        schema: "dbo",
        table: "public_entities",
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

    impl Entity for PublicEntity {
        fn metadata() -> &'static EntityMetadata {
            &PUBLIC_ENTITY_METADATA
        }
    }

    struct PublicPolicy;

    impl EntityPolicy for PublicPolicy {
        const POLICY_NAME: &'static str = "public_policy";
        const COLUMN_NAMES: &'static [&'static str] = &[];

        fn columns() -> &'static [super::core::ColumnMetadata] {
            &[]
        }
    }

    #[allow(dead_code)]
    #[derive(SoftDeleteFields)]
    struct PublicSoftDelete {
        #[orm(sql_type = "datetime2")]
        deleted_at: Option<String>,

        #[orm(nullable)]
        #[orm(length = 120)]
        deleted_by: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(AuditFields)]
    struct PublicAudit {
        #[orm(created_at)]
        #[orm(default_sql = "SYSUTCDATETIME()")]
        #[orm(sql_type = "datetime2")]
        #[orm(updatable = false)]
        created_at: String,

        #[orm(created_by)]
        #[orm(column = "created_by_user_id")]
        created_by: Option<i64>,

        #[orm(updated_by)]
        #[orm(nullable)]
        #[orm(length = 120)]
        updated_by: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(TenantContext)]
    struct PublicTenant {
        #[orm(column = "company_id")]
        tenant_id: i64,
    }

    #[test]
    fn exposes_public_prelude() {
        let error = OrmError::new("public-api");
        let raw_query_type = core::any::type_name::<RawQuery<PublicEntity>>();
        let raw_command_type = core::any::type_name::<RawCommand>();
        let projection_type = core::any::type_name::<SelectProjection>();
        let query_hint = QueryHint::Recompile;
        let raw_execution = RawSqlExecution::ReadOnly;
        let query_execution = QueryExecution::ReadOnly;
        fn assert_raw_param<T: RawParam>() {}
        fn assert_raw_params<T: RawParams>() {}
        fn assert_select_projections<T: SelectProjections>() {}

        assert!(raw_query_type.contains("RawQuery"));
        assert!(raw_command_type.contains("RawCommand"));
        assert!(projection_type.contains("SelectProjection"));
        assert_raw_param::<i64>();
        assert_raw_param::<SqlValue>();
        assert_raw_params::<(bool, i64)>();
        assert_select_projections::<(EntityColumn<PublicEntity>,)>();
        assert_eq!(query_hint, QueryHint::Recompile);
        assert_eq!(raw_execution, RawSqlExecution::ReadOnly);
        assert_eq!(query_execution, QueryExecution::ReadOnly);
        assert_eq!(error.message(), "public-api");
        assert_eq!(
            ColumnValue::new("email", SqlValue::String("ana@example.com".to_string())),
            ColumnValue {
                column_name: "email",
                value: SqlValue::String("ana@example.com".to_string()),
            }
        );
        assert_eq!(String::SQL_SERVER_TYPE, SqlServerType::NVarChar);
        assert_eq!(PageRequest::new(2, 25).page, 2);
    }

    #[test]
    fn exposes_entity_contract_in_prelude() {
        assert_eq!(PublicEntity::metadata().table, "public_entities");
    }

    #[test]
    fn exposes_navigation_metadata_contract_in_prelude() {
        let navigation = NavigationMetadata::new(
            "owner",
            NavigationKind::BelongsTo,
            "User",
            "auth",
            "users",
            &["owner_id"],
            &["id"],
            Some("fk_posts_owner_id_users"),
        );

        assert_eq!(navigation.rust_field, "owner");
        assert_eq!(navigation.kind, NavigationKind::BelongsTo);
        assert!(navigation.targets_table("auth", "users"));
        assert!(navigation.uses_foreign_key("fk_posts_owner_id_users"));
    }

    #[test]
    fn lazy_navigation_wrappers_are_memory_only_state_containers() {
        let mut owner = LazyNavigation::unloaded();
        assert!(!owner.is_loaded());
        assert_eq!(owner.as_ref(), None);

        owner.set_loaded(Some(7_i64));
        assert!(owner.is_loaded());
        assert_eq!(owner.as_ref(), Some(&7_i64));

        let cloned = owner.clone();
        assert_eq!(
            format!("{:?}", cloned),
            "LazyNavigation { value: Some(7), loaded: true }"
        );

        owner.clear();
        assert!(!owner.is_loaded());
        assert_eq!(owner.as_ref(), None);

        let mut children = LazyCollection::unloaded();
        assert!(!children.is_loaded());
        assert!(children.as_slice().is_empty());

        children.set_loaded(vec![1_i64, 2_i64]);
        assert!(children.is_loaded());
        assert_eq!(children.as_slice(), &[1_i64, 2_i64]);

        let cloned = children.clone();
        assert_eq!(
            format!("{:?}", cloned),
            "LazyCollection { values: [1, 2], loaded: true }"
        );

        children.clear();
        assert!(!children.is_loaded());
        assert!(children.as_slice().is_empty());
    }

    #[test]
    fn exposes_entity_policy_contract_in_prelude() {
        assert_eq!(
            PublicPolicy::metadata(),
            EntityPolicyMetadata::new("public_policy", &[])
        );
    }

    #[test]
    fn exposes_audit_entity_contract_in_prelude() {
        struct PublicAuditEntity;

        impl Entity for PublicAuditEntity {
            fn metadata() -> &'static EntityMetadata {
                &PUBLIC_ENTITY_METADATA
            }
        }

        impl AuditEntity for PublicAuditEntity {
            fn audit_policy() -> Option<EntityPolicyMetadata> {
                Some(EntityPolicyMetadata::new("audit", &[]))
            }
        }

        assert_eq!(
            PublicAuditEntity::audit_policy(),
            Some(EntityPolicyMetadata::new("audit", &[]))
        );
    }

    #[test]
    fn exposes_soft_delete_contract_in_prelude() {
        struct PublicSoftDeleteEntity;

        impl Entity for PublicSoftDeleteEntity {
            fn metadata() -> &'static EntityMetadata {
                &PUBLIC_ENTITY_METADATA
            }
        }

        impl SoftDeleteEntity for PublicSoftDeleteEntity {
            fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
                Some(EntityPolicyMetadata::new("soft_delete", &[]))
            }
        }

        assert_eq!(
            PublicSoftDeleteEntity::soft_delete_policy(),
            Some(EntityPolicyMetadata::new("soft_delete", &[]))
        );
    }

    #[test]
    fn exposes_tenant_contract_in_prelude() {
        struct PublicTenantEntity;

        impl Entity for PublicTenantEntity {
            fn metadata() -> &'static EntityMetadata {
                &PUBLIC_ENTITY_METADATA
            }
        }

        impl TenantScopedEntity for PublicTenantEntity {
            fn tenant_policy() -> Option<EntityPolicyMetadata> {
                Some(EntityPolicyMetadata::new("tenant", &[]))
            }
        }

        assert_eq!(
            PublicTenantEntity::tenant_policy(),
            Some(EntityPolicyMetadata::new("tenant", &[]))
        );
    }

    #[test]
    fn exposes_audit_runtime_contract_in_prelude() {
        struct PublicAuditProvider;

        impl AuditProvider for PublicAuditProvider {
            fn values(&self, context: AuditContext<'_>) -> Result<Vec<ColumnValue>, OrmError> {
                assert_eq!(context.operation, AuditOperation::Update);
                assert!(context.request_values.is_some());

                Ok(vec![ColumnValue::new(
                    "updated_at",
                    SqlValue::String("provider-updated-at".to_string()),
                )])
            }
        }

        let request_values = AuditRequestValues::new(vec![ColumnValue::new(
            "updated_by",
            SqlValue::String("request-updated-by".to_string()),
        )]);
        let context = AuditContext {
            entity: PublicEntity::metadata(),
            operation: AuditOperation::Update,
            request_values: Some(&request_values),
        };

        let provider = PublicAuditProvider;
        let values = provider.values(context).unwrap();

        assert_eq!(request_values.values()[0].column_name, "updated_by");
        assert_eq!(values[0].column_name, "updated_at");
    }

    #[test]
    fn derives_audit_fields_policy_metadata_from_public_prelude() {
        let metadata = PublicAudit::metadata();

        assert_eq!(metadata.name, "audit");
        assert_eq!(metadata.columns.len(), 3);
        assert_eq!(metadata.columns[0].rust_field, "created_at");
        assert_eq!(metadata.columns[0].column_name, "created_at");
        assert_eq!(metadata.columns[0].sql_type, SqlServerType::DateTime2);
        assert_eq!(metadata.columns[0].default_sql, Some("SYSUTCDATETIME()"));
        assert!(metadata.columns[0].insertable);
        assert!(!metadata.columns[0].updatable);
        assert_eq!(metadata.columns[1].column_name, "created_by_user_id");
        assert!(metadata.columns[1].nullable);
        assert_eq!(metadata.columns[1].sql_type, SqlServerType::BigInt);
        assert_eq!(metadata.columns[2].max_length, Some(120));
        assert!(metadata.columns[2].updatable);
        assert_eq!(
            <PublicAudit as EntityPolicy>::COLUMN_NAMES,
            &["created_at", "created_by_user_id", "updated_by"]
        );

        let audit_values = PublicAudit {
            created_at: "2026-04-28T00:00:00Z".to_string(),
            created_by: Some(7),
            updated_by: None,
        }
        .audit_values();

        assert_eq!(
            audit_values,
            vec![
                ColumnValue::new(
                    "created_at",
                    SqlValue::String("2026-04-28T00:00:00Z".to_string())
                ),
                ColumnValue::new("created_by_user_id", SqlValue::I64(7)),
                ColumnValue::new("updated_by", SqlValue::TypedNull(SqlServerType::NVarChar)),
            ]
        );
    }

    #[test]
    fn derives_tenant_context_policy_metadata_from_public_prelude() {
        let metadata = PublicTenant::metadata();
        let tenant = PublicTenant { tenant_id: 42 };
        let active_tenant = ActiveTenant::from_context(&tenant);

        assert_eq!(metadata.name, "tenant");
        assert_eq!(metadata.columns.len(), 1);
        assert_eq!(metadata.columns[0].rust_field, "tenant_id");
        assert_eq!(metadata.columns[0].column_name, "company_id");
        assert_eq!(metadata.columns[0].sql_type, SqlServerType::BigInt);
        assert!(metadata.columns[0].insertable);
        assert!(!metadata.columns[0].updatable);
        assert_eq!(
            <PublicTenant as EntityPolicy>::COLUMN_NAMES,
            &["company_id"]
        );
        assert_eq!(PublicTenant::COLUMN_NAME, "company_id");
        assert_eq!(tenant.tenant_value(), SqlValue::I64(42));
        assert_eq!(active_tenant.column_name, "company_id");
        assert_eq!(active_tenant.value, SqlValue::I64(42));
    }

    #[test]
    fn exposes_operational_configuration_surface() {
        let options = MssqlOperationalOptions::new()
            .with_timeouts(MssqlTimeoutOptions::new().with_query_timeout(Duration::from_secs(30)))
            .with_retry(MssqlRetryOptions::enabled(
                2,
                Duration::from_millis(50),
                Duration::from_secs(1),
            ))
            .with_pool(MssqlPoolOptions::bb8(12));
        let config = MssqlConnectionConfig::from_connection_string_with_options(
            "server=tcp:localhost,1433;database=master;user=sa;password=Password123;TrustServerCertificate=true",
            options,
        )
        .unwrap();

        assert_eq!(config.options().pool.backend, MssqlPoolBackend::Bb8);
        assert_eq!(config.options().pool.max_size, 12);
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn exposes_pool_surface_when_feature_is_enabled() {
        let builder = super::MssqlPool::builder().max_size(8);

        assert_eq!(builder.options().max_size, 8);
    }

    #[cfg(feature = "pool-bb8")]
    #[test]
    fn exposes_dbcontext_pool_wiring_when_feature_is_enabled() {
        let _from_pool = DerivedDbContext::from_pool;
        let _shared_from_pool = super::connect_shared_from_pool;
    }

    #[test]
    fn exposes_dbcontext_entity_set_contract_in_prelude() {
        fn require_trait<C, E>()
        where
            C: DbContextEntitySet<E>,
            E: Entity,
        {
        }

        require_trait::<DerivedDbContext, DerivedUser>();
    }

    #[test]
    fn exposes_dbcontext_health_check_contract_in_prelude() {
        let _health_check = DerivedDbContext::health_check;
        let _trait_health_check = <DerivedDbContext as DbContext>::health_check;
    }

    #[test]
    fn exposes_dbcontext_soft_delete_runtime_helpers() {
        let _with_soft_delete_provider = DerivedDbContext::with_soft_delete_provider;
        let _with_soft_delete_request_values = DerivedDbContext::with_soft_delete_request_values;
        let _with_soft_delete_values = DerivedDbContext::with_soft_delete_values::<SoftDelete>;
        let _clear_soft_delete_request_values = DerivedDbContext::clear_soft_delete_request_values;
        let _shared_with_soft_delete_values =
            SharedConnection::with_soft_delete_values::<SoftDelete>;
    }

    #[test]
    fn exposes_dbcontext_audit_runtime_helpers() {
        let _with_audit_provider = DerivedDbContext::with_audit_provider;
        let _with_audit_request_values = DerivedDbContext::with_audit_request_values;
        let _clear_audit_request_values = DerivedDbContext::clear_audit_request_values;
        let _shared_with_audit_provider = SharedConnection::with_audit_provider;
        let _shared_with_audit_request_values = SharedConnection::with_audit_request_values;
        let _shared_clear_audit_request_values = SharedConnection::clear_audit_request_values;
    }

    #[test]
    fn exposes_dbcontext_tenant_runtime_helpers() {
        let _with_tenant = DerivedDbContext::with_tenant::<PublicTenant>;
        let _clear_tenant = DerivedDbContext::clear_tenant;
        let _shared_with_tenant = SharedConnection::with_tenant::<PublicTenant>;
        let _shared_clear_tenant = SharedConnection::clear_tenant;
    }

    #[test]
    fn exposes_migration_model_source_contract_in_prelude() {
        fn require_trait<C: super::MigrationModelSource>() {}

        require_trait::<DerivedDbContext>();
        assert_eq!(
            <DerivedDbContext as super::MigrationModelSource>::entity_metadata()
                .iter()
                .map(|metadata| metadata.table)
                .collect::<Vec<_>>(),
            vec!["users", "audit_entries"]
        );
    }

    #[test]
    fn exposes_model_snapshot_export_helpers() {
        let snapshot = super::model_snapshot_from_source::<DerivedDbContext>();
        let json = super::model_snapshot_json_from_source::<DerivedDbContext>().unwrap();

        assert_eq!(
            snapshot
                .schemas
                .iter()
                .flat_map(|schema| schema.tables.iter().map(|table| table.name.as_str()))
                .collect::<Vec<_>>(),
            vec!["users", "audit_entries"]
        );
        assert!(json.contains("\"name\": \"auth\""));
        assert!(json.contains("\"name\": \"users\""));
    }

    #[test]
    fn exposes_active_record_contract_in_prelude() {
        fn require_trait<E: ActiveRecord>() {}

        require_trait::<PublicEntity>();
    }

    #[test]
    fn exposes_tracking_surface_in_prelude() {
        let tracked = Tracked::from_loaded(String::from("tracked"));

        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert_eq!(tracked.current(), "tracked");
    }

    #[allow(dead_code)]
    #[derive(Entity, Debug, Clone)]
    #[orm(table = "users", schema = "auth")]
    #[orm(index(name = "ix_users_email_created_by", columns(email, created_by)))]
    struct DerivedUser {
        #[orm(primary_key)]
        #[orm(identity)]
        id: i64,

        #[orm(length = 180)]
        #[orm(unique)]
        email: String,

        #[orm(nullable)]
        #[orm(index(name = "ix_users_display_name"))]
        display_name: Option<String>,

        #[orm(default_sql = "'system'")]
        created_by: String,

        #[orm(rowversion)]
        version: Vec<u8>,
    }

    #[allow(dead_code)]
    #[derive(Entity, Debug, Clone)]
    struct AuditEntry {
        id: i64,
        payload: String,
    }

    #[allow(dead_code)]
    #[derive(SoftDeleteFields)]
    struct SoftDelete {
        #[orm(deleted_at)]
        deleted_at: Option<String>,
    }

    #[derive(Insertable, Debug, Clone)]
    #[orm(entity = DerivedUser)]
    struct NewDerivedUser {
        email: String,
        display_name: Option<String>,
        #[orm(column = "created_by")]
        author: String,
    }

    #[derive(Changeset, Debug, Clone)]
    #[orm(entity = DerivedUser)]
    struct UpdateDerivedUser {
        email: Option<String>,
        display_name: Option<Option<String>>,
        #[orm(column = "created_by")]
        author: Option<String>,
    }

    #[allow(dead_code)]
    #[derive(DbContext, Debug, Clone)]
    struct DerivedDbContext {
        pub users: DbSet<DerivedUser>,
        pub audit_entries: DbSet<AuditEntry>,
    }

    #[test]
    fn derives_entity_metadata_from_struct_attributes() {
        let metadata = DerivedUser::metadata();

        assert_eq!(metadata.rust_name, "DerivedUser");
        assert_eq!(metadata.schema, "auth");
        assert_eq!(metadata.table, "users");
        assert_eq!(metadata.primary_key.columns, &["id"]);
        assert_eq!(metadata.indexes.len(), 3);

        let id = metadata.field("id").expect("id column metadata");
        assert_eq!(id.sql_type, SqlServerType::BigInt);
        assert_eq!(id.identity, Some(IdentityMetadata::new(1, 1)));
        assert!(!id.insertable);
        assert!(!id.updatable);

        let email = metadata.field("email").expect("email column metadata");
        assert_eq!(email.sql_type, SqlServerType::NVarChar);
        assert_eq!(email.max_length, Some(180));
        assert!(!email.nullable);

        let display_name = metadata
            .field("display_name")
            .expect("display_name column metadata");
        assert!(display_name.nullable);
        assert_eq!(display_name.max_length, Some(255));

        let created_by = metadata
            .field("created_by")
            .expect("created_by column metadata");
        assert_eq!(created_by.default_sql, Some("'system'"));

        let version = metadata.field("version").expect("version column metadata");
        assert_eq!(version.sql_type, SqlServerType::RowVersion);
        assert!(version.rowversion);
        assert!(!version.insertable);
        assert!(!version.updatable);

        assert_eq!(metadata.indexes[0].name, "ux_users_email");
        assert!(metadata.indexes[0].unique);
        assert_eq!(metadata.indexes[1].name, "ix_users_display_name");
        assert!(!metadata.indexes[1].unique);
        assert_eq!(metadata.indexes[2].name, "ix_users_email_created_by");
        assert_eq!(metadata.indexes[2].columns.len(), 2);
        assert_eq!(metadata.indexes[2].columns[0].column_name, "email");
        assert_eq!(metadata.indexes[2].columns[1].column_name, "created_by");
        assert!(!metadata.indexes[2].columns[0].descending);
        assert!(!metadata.indexes[2].columns[1].descending);
    }

    #[test]
    fn derives_default_table_and_primary_key_convention() {
        let metadata = AuditEntry::metadata();

        assert_eq!(metadata.schema, "dbo");
        assert_eq!(metadata.table, "audit_entries");
        assert_eq!(metadata.primary_key.columns, &["id"]);

        let payload = metadata.field("payload").expect("payload column metadata");
        assert_eq!(payload.sql_type, SqlServerType::NVarChar);
        assert_eq!(payload.max_length, Some(255));
        assert!(payload.insertable);
        assert!(payload.updatable);
    }

    #[test]
    fn exposes_static_columns_for_future_query_builder() {
        let email: EntityColumn<DerivedUser> = DerivedUser::email;
        let version = DerivedUser::version;
        let payload = AuditEntry::payload;

        assert_eq!(email.rust_field(), "email");
        assert_eq!(email.column_name(), "email");
        assert_eq!(email.entity_metadata().table, "users");
        assert_eq!(email.metadata().max_length, Some(180));

        assert_eq!(version.column_name(), "version");
        assert_eq!(version.metadata().sql_type, SqlServerType::RowVersion);
        assert!(!version.metadata().insertable);

        assert_eq!(payload.entity_metadata().table, "audit_entries");
        assert_eq!(payload.metadata().column_name, "payload");
    }

    #[test]
    fn exposes_public_column_predicate_extensions() {
        assert_eq!(
            DerivedUser::email.eq("ana@example.com".to_string()),
            Predicate::eq(
                Expr::from(DerivedUser::email),
                Expr::value(SqlValue::String("ana@example.com".to_string()))
            )
        );
        assert_eq!(
            DerivedUser::display_name.is_null(),
            Predicate::is_null(Expr::from(DerivedUser::display_name))
        );
        assert_eq!(
            DerivedUser::email.contains("@example.com"),
            Predicate::like_escaped(
                Expr::from(DerivedUser::email),
                Expr::value(SqlValue::String("%@example.com%".to_string())),
                '\\'
            )
        );
        assert_eq!(
            DerivedUser::email.asc(),
            OrderBy::new(TableRef::new("auth", "users"), "email", SortDirection::Asc)
        );
        assert_eq!(
            DerivedUser::email
                .contains("@example.com")
                .and(DerivedUser::display_name.is_not_null()),
            Predicate::and(vec![
                Predicate::like_escaped(
                    Expr::from(DerivedUser::email),
                    Expr::value(SqlValue::String("%@example.com%".to_string())),
                    '\\'
                ),
                Predicate::is_not_null(Expr::from(DerivedUser::display_name))
            ])
        );
    }

    #[test]
    fn derives_insertable_values_from_named_fields() {
        let insertable = NewDerivedUser {
            email: "ana@example.com".to_string(),
            display_name: None,
            author: "system".to_string(),
        };

        let values = <NewDerivedUser as Insertable<DerivedUser>>::values(&insertable);

        assert_eq!(
            values,
            vec![
                ColumnValue::new("email", SqlValue::String("ana@example.com".to_string())),
                ColumnValue::new("display_name", SqlValue::TypedNull(SqlServerType::NVarChar)),
                ColumnValue::new("created_by", SqlValue::String("system".to_string())),
            ]
        );
    }

    #[test]
    fn derives_changeset_with_outer_option_semantics() {
        let changeset = UpdateDerivedUser {
            email: Some("ana.maria@example.com".to_string()),
            display_name: Some(None),
            author: None,
        };

        let changes = <UpdateDerivedUser as Changeset<DerivedUser>>::changes(&changeset);

        assert_eq!(
            changes,
            vec![
                ColumnValue::new(
                    "email",
                    SqlValue::String("ana.maria@example.com".to_string())
                ),
                ColumnValue::new("display_name", SqlValue::TypedNull(SqlServerType::NVarChar)),
            ]
        );
    }
}
