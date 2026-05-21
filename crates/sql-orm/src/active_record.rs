use crate::{AuditEntity, DbContextEntitySet, DbSetQuery, SoftDeleteEntity, TenantScopedEntity};
use core::future::Future;
use sql_orm_core::{ColumnValue, Entity, FromRow, OrmError, SqlTypeMapping, SqlValue};

#[doc(hidden)]
pub trait EntityPrimaryKey: Entity {
    fn primary_key_value(&self) -> Result<SqlValue, OrmError>;
}

#[doc(hidden)]
pub enum EntityPersistMode {
    Insert,
    InsertOrUpdate(SqlValue),
    Update(SqlValue),
}

#[doc(hidden)]
pub trait EntityPersist: Entity {
    fn persist_mode(&self) -> Result<EntityPersistMode, OrmError>;
    fn insert_values(&self) -> Vec<ColumnValue>;
    fn update_changes(&self) -> Vec<ColumnValue>;
    fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError>;
    fn sync_persisted(&mut self, persisted: Self);

    #[doc(hidden)]
    fn has_persisted_changes(original: &Self, current: &Self) -> bool {
        original.update_changes() != current.update_changes()
    }
}

/// Convenience Active Record style API for entities.
///
/// Every `Entity` implements this trait. The methods delegate to the `DbSet`
/// declared on a `DbContext`, so Active Record remains a thin convenience
/// layer over the same query, insert, update, delete, tenant, audit, and
/// soft-delete pipelines used by the explicit context API.
pub trait ActiveRecord: Entity + Sized {
    /// Starts a query for this entity through the context's `DbSet<Self>`.
    fn query<C>(db: &C) -> DbSetQuery<Self>
    where
        C: DbContextEntitySet<Self>,
        Self: TenantScopedEntity,
    {
        db.db_set().query()
    }

    /// Finds one entity by single-column primary key through the context.
    fn find<C, K>(db: &C, key: K) -> impl Future<Output = Result<Option<Self>, OrmError>> + Send
    where
        C: DbContextEntitySet<Self>,
        Self: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
        K: SqlTypeMapping + Send,
    {
        db.db_set().find(key)
    }

    /// Deletes this entity through the context's `DbSet<Self>`.
    ///
    /// Entities with `soft_delete` use logical delete. Entities with
    /// rowversion participate in the same concurrency-conflict detection as
    /// the explicit `DbSet` delete path.
    fn delete<C>(&self, db: &C) -> impl Future<Output = Result<bool, OrmError>> + Send
    where
        C: DbContextEntitySet<Self> + Sync,
        Self: EntityPrimaryKey
            + EntityPersist
            + FromRow
            + Send
            + SoftDeleteEntity
            + TenantScopedEntity,
    {
        let key = <Self as EntityPrimaryKey>::primary_key_value(self);
        let concurrency_token = <Self as EntityPersist>::concurrency_token(self);

        async move {
            db.db_set()
                .delete_by_sql_value(key?, concurrency_token?)
                .await
        }
    }

    /// Inserts or updates this entity according to the derived persistence
    /// strategy.
    ///
    /// The method syncs the in-memory entity with the persisted row returned
    /// by SQL Server.
    fn save<C>(&mut self, db: &C) -> impl Future<Output = Result<(), OrmError>> + Send
    where
        C: DbContextEntitySet<Self> + Sync,
        Self: AuditEntity + EntityPersist + FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        async move {
            match <Self as EntityPersist>::persist_mode(self)? {
                EntityPersistMode::Insert => {
                    let persisted = db.db_set().insert_entity(self).await?;
                    <Self as EntityPersist>::sync_persisted(self, persisted);
                    Ok(())
                }
                EntityPersistMode::InsertOrUpdate(key) => {
                    if db
                        .db_set()
                        .exists_by_sql_value_internal(key.clone())
                        .await?
                    {
                        if let Some(persisted) = db
                            .db_set()
                            .update_entity_by_sql_value(
                                key,
                                self,
                                <Self as EntityPersist>::concurrency_token(self)?,
                            )
                            .await?
                        {
                            <Self as EntityPersist>::sync_persisted(self, persisted);
                        } else {
                            return Err(OrmError::concurrency(
                                "ActiveRecord save could not update a row for the current primary key",
                            ));
                        }
                    } else {
                        let persisted = db.db_set().insert_entity(self).await?;
                        <Self as EntityPersist>::sync_persisted(self, persisted);
                    }

                    Ok(())
                }
                EntityPersistMode::Update(key) => {
                    let persisted = db
                        .db_set()
                        .update_entity_by_sql_value(
                            key,
                            self,
                            <Self as EntityPersist>::concurrency_token(self)?,
                        )
                        .await?
                        .ok_or_else(|| {
                            OrmError::concurrency(
                                "ActiveRecord save could not update a row for the current primary key",
                            )
                        })?;
                    <Self as EntityPersist>::sync_persisted(self, persisted);
                    Ok(())
                }
            }
        }
    }
}

impl<E: Entity> ActiveRecord for E {}

#[cfg(test)]
mod tests {
    use super::{ActiveRecord, EntityPersist, EntityPersistMode, EntityPrimaryKey};
    use crate::{
        AuditEntity, DbContext, DbContextEntitySet, DbSet, SoftDeleteEntity, TenantScopedEntity,
        Tracked,
    };
    use sql_orm_core::{
        ColumnMetadata, ColumnValue, Entity, EntityMetadata, EntityPolicyMetadata, FromRow,
        OrmError, OrmErrorKind, PrimaryKeyMetadata, Row, SqlServerType,
    };
    use sql_orm_query::SelectQuery;

    #[derive(Debug, Clone, PartialEq)]
    struct TestEntity {
        id: i64,
        name: String,
    }

    static TEST_ENTITY_COLUMNS: [ColumnMetadata; 2] = [
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

    impl Entity for TestEntity {
        fn metadata() -> &'static EntityMetadata {
            &TEST_ENTITY_METADATA
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

    impl TenantScopedEntity for TestEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl FromRow for TestEntity {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self {
                id: 7,
                name: "Persisted".to_string(),
            })
        }
    }

    impl EntityPrimaryKey for TestEntity {
        fn primary_key_value(&self) -> Result<sql_orm_core::SqlValue, OrmError> {
            Ok(sql_orm_core::SqlValue::I64(self.id))
        }
    }

    impl EntityPersist for TestEntity {
        fn persist_mode(&self) -> Result<EntityPersistMode, OrmError> {
            Ok(EntityPersistMode::Update(sql_orm_core::SqlValue::I64(
                self.id,
            )))
        }

        fn insert_values(&self) -> Vec<ColumnValue> {
            vec![ColumnValue::new(
                "name",
                sql_orm_core::SqlValue::String(self.name.clone()),
            )]
        }

        fn update_changes(&self) -> Vec<ColumnValue> {
            vec![ColumnValue::new(
                "name",
                sql_orm_core::SqlValue::String(self.name.clone()),
            )]
        }

        fn concurrency_token(&self) -> Result<Option<sql_orm_core::SqlValue>, OrmError> {
            Ok(None)
        }

        fn sync_persisted(&mut self, persisted: Self) {
            *self = persisted;
        }
    }

    struct DummyContext {
        entities: DbSet<TestEntity>,
    }

    impl DbContext for DummyContext {
        fn from_shared_connection(_connection: crate::SharedConnection) -> Self {
            unreachable!("DummyContext is only used in disconnected unit tests")
        }

        fn shared_connection(&self) -> crate::SharedConnection {
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

    #[test]
    fn active_record_query_delegates_to_typed_dbset() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };

        let query = TestEntity::query(&context);

        assert_eq!(
            query.into_select_query(),
            SelectQuery::from_entity::<TestEntity>()
        );
    }

    #[test]
    fn active_record_trait_is_available_for_entities() {
        fn require_active_record<E: ActiveRecord>() {}

        require_active_record::<TestEntity>();
    }

    #[tokio::test]
    async fn tracked_save_unchanged_is_noop_without_dereferencing_to_active_record() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let mut tracked = Tracked::from_loaded(TestEntity {
            id: 7,
            name: "Tracked".to_string(),
        });

        tracked.save(&context).await.unwrap();

        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(tracked.original(), tracked.current());
    }

    #[tokio::test]
    async fn tracked_save_unchanged_registered_entry_remains_tracked() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let registry = context.entities.tracking_registry();
        let mut tracked = Tracked::from_loaded(TestEntity {
            id: 7,
            name: "Tracked".to_string(),
        });
        tracked
            .attach_registry_loaded(registry.clone(), sql_orm_core::SqlValue::I64(7))
            .unwrap();

        tracked.save(&context).await.unwrap();

        assert_eq!(tracked.state(), crate::EntityState::Unchanged);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Unchanged
        );
    }

    #[tokio::test]
    async fn tracked_save_deleted_returns_stable_error_before_active_record() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let mut tracked = Tracked::from_loaded(TestEntity {
            id: 7,
            name: "Tracked".to_string(),
        });
        context.entities.remove_tracked(&mut tracked);

        let error = tracked.save(&context).await.unwrap_err();

        assert_eq!(
            error.message(),
            "tracked deleted entities cannot be saved; detach them or persist deletion"
        );
    }

    #[tokio::test]
    async fn tracked_save_deleted_registered_entry_keeps_pending_delete_after_error() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let registry = context.entities.tracking_registry();
        let mut tracked = Tracked::from_loaded(TestEntity {
            id: 7,
            name: "Tracked".to_string(),
        });
        tracked
            .attach_registry_loaded(registry.clone(), sql_orm_core::SqlValue::I64(7))
            .unwrap();
        context.entities.remove_tracked(&mut tracked);

        let error = tracked.save(&context).await.unwrap_err();

        assert_eq!(
            error.message(),
            "tracked deleted entities cannot be saved; detach them or persist deletion"
        );
        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].state,
            crate::EntityState::Deleted
        );
    }

    #[tokio::test]
    async fn tracked_delete_added_cancels_local_insert_without_active_record() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let registry = context.entities.tracking_registry();
        let mut tracked = context.entities.add_tracked(TestEntity {
            id: 0,
            name: "Pending".to_string(),
        });

        let deleted = tracked.delete(&context).await.unwrap();

        assert!(!deleted);
        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);
    }

    #[tokio::test]
    async fn tracked_delete_deleted_entry_is_idempotent_without_active_record() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let registry = context.entities.tracking_registry();
        let mut tracked = context.entities.add_tracked(TestEntity {
            id: 0,
            name: "Pending".to_string(),
        });
        tracked.delete(&context).await.unwrap();

        let deleted = tracked.delete(&context).await.unwrap();

        assert!(!deleted);
        assert_eq!(tracked.state(), crate::EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);
    }

    #[test]
    fn active_record_find_reuses_dbset_error_contract() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };

        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let error = match runtime.block_on(TestEntity::find(&context, 1_i64)) {
            Ok(value) => panic!("expected disconnected ActiveRecord::find to fail, got {value:?}"),
            Err(error) => error,
        };

        assert_eq!(
            error.message(),
            "DbSetQuery requires an initialized shared connection"
        );
        assert_eq!(error.kind(), OrmErrorKind::Execution);
    }

    #[test]
    fn active_record_delete_reuses_dbset_error_contract() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let entity = TestEntity {
            id: 7,
            name: "Ana".to_string(),
        };

        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let error = match runtime.block_on(entity.delete(&context)) {
            Ok(value) => {
                panic!("expected disconnected ActiveRecord::delete to fail, got {value:?}")
            }
            Err(error) => error,
        };

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(error.kind(), OrmErrorKind::Execution);
    }

    #[test]
    fn active_record_save_reuses_dbset_error_contract() {
        let context = DummyContext {
            entities: DbSet::<TestEntity>::disconnected(),
        };
        let mut entity = TestEntity {
            id: 7,
            name: "Ana".to_string(),
        };

        let runtime = tokio::runtime::Runtime::new().expect("tokio runtime");
        let error = match runtime.block_on(entity.save(&context)) {
            Ok(()) => panic!("expected disconnected ActiveRecord::save to fail"),
            Err(error) => error,
        };

        assert_eq!(
            error.message(),
            "DbSet requires an initialized shared connection"
        );
        assert_eq!(error.kind(), OrmErrorKind::Execution);
    }
}
