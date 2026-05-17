#[cfg(feature = "pool-bb8")]
use sql_orm::MssqlPool;
use sql_orm::prelude::*;
use sql_orm::query::{CompiledQuery, Expr, Predicate, SelectQuery};
use sql_orm::tiberius::MssqlConnection;
use std::sync::OnceLock;
#[cfg(feature = "pool-bb8")]
use std::time::Duration;
use tokio::sync::{Mutex, MutexGuard};

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const KEEP_ROWS_ENV: &str = "KEEP_TEST_ROWS";
const TEST_TABLE_NAME: &str = "dbo.sql_orm_public_crud";
const VERSIONED_TEST_TABLE_NAME: &str = "dbo.sql_orm_public_crud_versioned";

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_public_crud", schema = "dbo")]
struct PublicCrudUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
    active: bool,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = PublicCrudUser)]
struct NewPublicCrudUser {
    name: String,
    active: bool,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = PublicCrudUser)]
struct UpdatePublicCrudUser {
    name: Option<String>,
    active: Option<bool>,
}

#[derive(DbContext)]
struct PublicCrudDb {
    pub users: DbSet<PublicCrudUser>,
}

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_public_crud_versioned", schema = "dbo")]
struct VersionedPublicCrudUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
    #[orm(rowversion)]
    version: Vec<u8>,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = VersionedPublicCrudUser)]
struct NewVersionedPublicCrudUser {
    name: String,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = VersionedPublicCrudUser)]
struct UpdateVersionedPublicCrudUser {
    name: Option<String>,
    version: Option<Vec<u8>>,
}

#[derive(DbContext)]
struct VersionedPublicCrudDb {
    pub users: DbSet<VersionedPublicCrudUser>,
}

#[derive(DbContext)]
struct CombinedTrackedCrudDb {
    pub users: DbSet<PublicCrudUser>,
    pub versioned_users: DbSet<VersionedPublicCrudUser>,
}

#[tokio::test]
async fn public_dbset_crud_api_roundtrips_against_real_sql_server() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!("skipping public CRUD integration test because {TEST_CONNECTION_ENV} is not set");
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let keep_rows = keep_test_rows();

    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;
    announce_test_table(keep_tables, keep_rows);

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;

        let inserted = db
            .users
            .insert(NewPublicCrudUser {
                name: "Ana".to_string(),
                active: true,
            })
            .await?;

        assert!(inserted.id > 0);
        assert_eq!(inserted.name, "Ana");
        assert!(inserted.active);

        let found = db.users.find(inserted.id).await?;
        assert_eq!(found, Some(inserted.clone()));

        let registry = <PublicCrudDb as sql_orm::DbContext>::tracking_registry(&db);
        assert_eq!(registry.entry_count(), 0);

        let tracked = db.users.find_tracked(inserted.id).await?;
        let mut tracked = tracked.expect("tracked entity should exist");
        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert_eq!(tracked.original(), &inserted);
        assert_eq!(tracked.current(), &inserted);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(
            registry.registrations()[0].entity_rust_name,
            "PublicCrudUser"
        );
        assert_eq!(registry.registrations()[0].state, EntityState::Unchanged);

        tracked.name = "Ana Maria".to_string();

        assert_eq!(tracked.state(), EntityState::Modified);
        assert_eq!(tracked.original(), &inserted);
        assert_eq!(
            tracked.current(),
            &PublicCrudUser {
                id: inserted.id,
                name: "Ana Maria".to_string(),
                active: true,
            }
        );

        let count = db.users.query().count().await?;
        assert_eq!(count, 1);

        let all = db.users.query().all().await?;
        assert_eq!(all, vec![inserted.clone()]);

        let filtered = db
            .users
            .query_with(
                SelectQuery::from_entity::<PublicCrudUser>().filter(Predicate::eq(
                    Expr::from(PublicCrudUser::id),
                    Expr::value(SqlValue::I64(inserted.id)),
                )),
            )
            .first()
            .await?;
        assert_eq!(filtered, Some(inserted.clone()));

        let updated = db
            .users
            .update(
                inserted.id,
                UpdatePublicCrudUser {
                    name: Some("Ana Maria".to_string()),
                    active: Some(false),
                },
            )
            .await?;
        assert_eq!(
            updated,
            Some(PublicCrudUser {
                id: inserted.id,
                name: "Ana Maria".to_string(),
                active: false,
            })
        );

        let updated_found = db.users.find(inserted.id).await?;
        assert_eq!(updated_found, updated);

        if keep_rows {
            let persisted = db.users.find(inserted.id).await?;
            assert_eq!(persisted, updated);
            assert_eq!(db.users.query().count().await?, 1);
        } else {
            let deleted = db.users.delete(inserted.id).await?;
            assert!(deleted);

            let after_delete = db.users.find(inserted.id).await?;
            assert_eq!(after_delete, None);
            assert_eq!(db.users.query().count().await?, 0);

            let deleted_again = db.users.delete(inserted.id).await?;
            assert!(!deleted_again);
        }

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables || keep_rows).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_transaction_commits_on_ok() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public transaction commit test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;
    announce_test_table(keep_tables, false);

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;

        let inserted = db
            .transaction(|tx| async move {
                tx.users
                    .insert(NewPublicCrudUser {
                        name: "Committed".to_string(),
                        active: true,
                    })
                    .await
            })
            .await?;

        assert!(inserted.id > 0);

        let persisted = db.users.find(inserted.id).await?;
        assert_eq!(
            persisted,
            Some(PublicCrudUser {
                id: inserted.id,
                name: "Committed".to_string(),
                active: true,
            })
        );
        assert_eq!(db.users.query().count().await?, 1);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_transaction_rolls_back_on_err() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public transaction rollback test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;
    announce_test_table(keep_tables, false);

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;

        let error = db
            .transaction(|tx| async move {
                tx.users
                    .insert(NewPublicCrudUser {
                        name: "Rolled Back".to_string(),
                        active: false,
                    })
                    .await?;

                Err::<(), OrmError>(OrmError::new(
                    "forcing transaction rollback for integration test",
                ))
            })
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "forcing transaction rollback for integration test"
        );
        assert_eq!(db.users.query().count().await?, 0);

        let all = db.users.query().all().await?;
        assert!(all.is_empty());

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[cfg(feature = "pool-bb8")]
#[tokio::test]
async fn public_dbcontext_pool_transaction_commits_and_rolls_back() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public pooled transaction test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;
    announce_test_table(keep_tables, false);

    let result = async {
        let pool = MssqlPool::builder()
            .max_size(1)
            .acquire_timeout(Duration::from_secs(2))
            .connect(&connection_string)
            .await?;
        let db = PublicCrudDb::from_pool(pool);

        let inserted = db
            .transaction(|tx| async move {
                tx.users
                    .insert(NewPublicCrudUser {
                        name: "Pooled Commit".to_string(),
                        active: true,
                    })
                    .await
            })
            .await?;

        assert!(inserted.id > 0);
        assert_eq!(db.users.query().count().await?, 1);

        let rollback_error = db
            .transaction(|tx| async move {
                tx.users
                    .insert(NewPublicCrudUser {
                        name: "Pooled Rollback".to_string(),
                        active: false,
                    })
                    .await?;

                Err::<(), OrmError>(OrmError::new(
                    "forcing pooled transaction rollback for integration test",
                ))
            })
            .await
            .unwrap_err();

        assert_eq!(
            rollback_error.message(),
            "forcing pooled transaction rollback for integration test"
        );
        assert_eq!(db.users.query().count().await?, 1);

        db.transaction(|tx| async move {
            let nested_error = tx
                .transaction(|_| async { Ok::<(), OrmError>(()) })
                .await
                .unwrap_err();
            assert!(nested_error.message().contains("nested db.transaction"));
            Ok(())
        })
        .await?;

        let persisted = db.users.find(inserted.id).await?;
        assert_eq!(
            persisted,
            Some(PublicCrudUser {
                id: inserted.id,
                name: "Pooled Commit".to_string(),
                active: true,
            })
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbset_update_uses_rowversion_to_prevent_stale_writes() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public rowversion CRUD integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_versioned_test_table(&connection_string).await?;

    let result = async {
        let db = VersionedPublicCrudDb::connect(&connection_string).await?;

        let inserted = db
            .users
            .insert(NewVersionedPublicCrudUser {
                name: "Ana".to_string(),
            })
            .await?;

        let updated = db
            .users
            .update(
                inserted.id,
                UpdateVersionedPublicCrudUser {
                    name: Some("Ana Maria".to_string()),
                    version: Some(inserted.version.clone()),
                },
            )
            .await?;
        let updated = updated.expect("rowversion-protected update should succeed");

        assert_eq!(updated.name, "Ana Maria");
        assert_ne!(updated.version, inserted.version);

        let stale = db
            .users
            .update(
                inserted.id,
                UpdateVersionedPublicCrudUser {
                    name: Some("Should Not Persist".to_string()),
                    version: Some(inserted.version.clone()),
                },
            )
            .await
            .unwrap_err();
        assert_eq!(stale, OrmError::ConcurrencyConflict);

        let persisted = db.users.find(updated.id).await?;
        assert_eq!(persisted, Some(updated.clone()));

        Ok(())
    }
    .await;

    cleanup_versioned_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_save_changes_persists_modified_tracked_entities() -> Result<(), OrmError>
{
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public save_changes integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;

        let inserted = db
            .users
            .insert(NewPublicCrudUser {
                name: "Tracked".to_string(),
                active: true,
            })
            .await?;

        let registry = <PublicCrudDb as sql_orm::DbContext>::tracking_registry(&db);
        let mut tracked = db
            .users
            .find_tracked(inserted.id)
            .await?
            .expect("tracked entity should exist");

        tracked.name = "Tracked Saved".to_string();
        tracked.active = false;

        let saved = db.save_changes().await?;
        assert_eq!(saved, 1);
        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert_eq!(
            tracked.current(),
            &PublicCrudUser {
                id: inserted.id,
                name: "Tracked Saved".to_string(),
                active: false,
            }
        );
        assert_eq!(tracked.original(), tracked.current());

        let persisted = db.users.find(inserted.id).await?;
        assert_eq!(persisted, Some(tracked.current().clone()));

        drop(tracked);
        assert_eq!(registry.entry_count(), 0);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_save_changes_returns_zero_for_unchanged_tracked_entities()
-> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public save_changes unchanged integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;

        let inserted = db
            .users
            .insert(NewPublicCrudUser {
                name: "Tracked Unchanged".to_string(),
                active: true,
            })
            .await?;

        let tracked = db
            .users
            .find_tracked(inserted.id)
            .await?
            .expect("tracked entity should exist");

        let saved = db.save_changes().await?;
        assert_eq!(saved, 0);
        assert_eq!(tracked.state(), EntityState::Unchanged);

        let persisted = db.users.find(inserted.id).await?;
        assert_eq!(persisted, Some(inserted));

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_save_changes_persists_added_tracked_entities() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public save_changes added integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;
        let registry = <PublicCrudDb as sql_orm::DbContext>::tracking_registry(&db);

        let mut tracked = db.users.add_tracked(PublicCrudUser {
            id: 0,
            name: "Tracked Added".to_string(),
            active: true,
        });

        assert_eq!(tracked.state(), EntityState::Added);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Added);

        tracked.active = false;

        let saved = db.save_changes().await?;
        assert_eq!(saved, 1);
        assert_eq!(tracked.state(), EntityState::Unchanged);
        assert!(tracked.id > 0);
        assert_eq!(
            tracked.current(),
            &PublicCrudUser {
                id: tracked.id,
                name: "Tracked Added".to_string(),
                active: false,
            }
        );
        assert_eq!(tracked.original(), tracked.current());

        let persisted = db.users.find(tracked.id).await?;
        assert_eq!(persisted, Some(tracked.current().clone()));

        drop(tracked);
        assert_eq!(registry.entry_count(), 0);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_save_changes_propagates_rowversion_conflicts() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public save_changes rowversion integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_versioned_test_table(&connection_string).await?;

    let result = async {
        let db = VersionedPublicCrudDb::connect(&connection_string).await?;

        let inserted = db
            .users
            .insert(NewVersionedPublicCrudUser {
                name: "Tracked".to_string(),
            })
            .await?;

        let mut tracked = db
            .users
            .find_tracked(inserted.id)
            .await?
            .expect("tracked entity should exist");

        tracked.name = "Tracked Stale".to_string();

        let externally_updated = db
            .users
            .update(
                inserted.id,
                UpdateVersionedPublicCrudUser {
                    name: Some("External Update".to_string()),
                    version: Some(inserted.version.clone()),
                },
            )
            .await?
            .expect("external update should succeed");

        let error = db.save_changes().await.unwrap_err();
        assert_eq!(error, OrmError::ConcurrencyConflict);
        assert_eq!(tracked.state(), EntityState::Modified);
        assert_eq!(tracked.current().name, "Tracked Stale");

        let persisted = db.users.find(inserted.id).await?;
        assert_eq!(persisted, Some(externally_updated));

        Ok(())
    }
    .await;

    cleanup_versioned_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_save_changes_persists_deleted_tracked_entities() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public save_changes deleted integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;
        let registry = <PublicCrudDb as sql_orm::DbContext>::tracking_registry(&db);

        let inserted = db
            .users
            .insert(NewPublicCrudUser {
                name: "Tracked Deleted".to_string(),
                active: true,
            })
            .await?;

        let mut tracked = db
            .users
            .find_tracked(inserted.id)
            .await?
            .expect("tracked entity should exist");

        db.users.remove_tracked(&mut tracked);

        assert_eq!(tracked.state(), EntityState::Deleted);
        assert_eq!(registry.entry_count(), 1);
        assert_eq!(registry.registrations()[0].state, EntityState::Deleted);

        let saved = db.save_changes().await?;
        assert_eq!(saved, 1);
        assert_eq!(tracked.state(), EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);

        let persisted = db.users.find(inserted.id).await?;
        assert_eq!(persisted, None);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_remove_tracked_cancels_pending_added_entity() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public remove_tracked added-cancel integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;
        let registry = <PublicCrudDb as sql_orm::DbContext>::tracking_registry(&db);

        let mut tracked = db.users.add_tracked(PublicCrudUser {
            id: 0,
            name: "Never Persisted".to_string(),
            active: true,
        });

        db.users.remove_tracked(&mut tracked);

        assert_eq!(tracked.state(), EntityState::Deleted);
        assert_eq!(registry.entry_count(), 0);
        assert_eq!(db.save_changes().await?, 0);
        assert_eq!(db.users.query().count().await?, 0);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_save_changes_persists_dropped_pending_wrappers() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public save_changes dropped-wrapper integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;

    let result = async {
        let db = PublicCrudDb::connect(&connection_string).await?;
        let registry = <PublicCrudDb as sql_orm::DbContext>::tracking_registry(&db);

        let inserted = db
            .users
            .insert(NewPublicCrudUser {
                name: "Tracked Drop".to_string(),
                active: true,
            })
            .await?;

        let mut tracked = db
            .users
            .find_tracked(inserted.id)
            .await?
            .expect("tracked entity should exist");
        tracked.name = "Dropped Mutation".to_string();

        assert_eq!(registry.entry_count(), 1);
        drop(tracked);
        assert_eq!(registry.entry_count(), 1);

        let saved = db.save_changes().await?;
        assert_eq!(saved, 1);

        let persisted = db.users.find(inserted.id).await?;
        assert_eq!(
            persisted,
            Some(PublicCrudUser {
                id: inserted.id,
                name: "Dropped Mutation".to_string(),
                active: true,
            })
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_save_changes_deleted_propagates_rowversion_conflicts()
-> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public save_changes deleted rowversion integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_versioned_test_table(&connection_string).await?;

    let result = async {
        let db = VersionedPublicCrudDb::connect(&connection_string).await?;

        let inserted = db
            .users
            .insert(NewVersionedPublicCrudUser {
                name: "Tracked Deleted".to_string(),
            })
            .await?;

        let mut tracked = db
            .users
            .find_tracked(inserted.id)
            .await?
            .expect("tracked entity should exist");

        let externally_updated = db
            .users
            .update(
                inserted.id,
                UpdateVersionedPublicCrudUser {
                    name: Some("External Update".to_string()),
                    version: Some(inserted.version.clone()),
                },
            )
            .await?
            .expect("external update should succeed");

        db.users.remove_tracked(&mut tracked);

        let error = db.save_changes().await.unwrap_err();
        assert_eq!(error, OrmError::ConcurrencyConflict);
        assert_eq!(tracked.state(), EntityState::Deleted);

        let persisted = db.users.find(inserted.id).await?;
        assert_eq!(persisted, Some(externally_updated));

        Ok(())
    }
    .await;

    cleanup_versioned_test_table(&connection_string, keep_tables).await?;

    result
}

#[tokio::test]
async fn public_dbcontext_shares_tracking_registry_across_dbsets() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping shared tracking registry integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let _fixture_guard = public_crud_fixture_lock().await;
    reset_test_table(&connection_string).await?;
    reset_versioned_test_table(&connection_string).await?;
    announce_test_table(keep_tables, false);

    let result = async {
        let db = CombinedTrackedCrudDb::connect(&connection_string).await?;
        let registry = <CombinedTrackedCrudDb as sql_orm::DbContext>::tracking_registry(&db);

        let inserted = db
            .users
            .insert(NewPublicCrudUser {
                name: "Shared Registry".to_string(),
                active: true,
            })
            .await?;

        let versioned = db
            .versioned_users
            .insert(NewVersionedPublicCrudUser {
                name: "Shared Registry Versioned".to_string(),
            })
            .await?;

        assert_eq!(registry.entry_count(), 0);

        let tracked_user = db.users.find_tracked(inserted.id).await?;
        let tracked_versioned_user = db.versioned_users.find_tracked(versioned.id).await?;

        let registrations = registry.registrations();
        assert_eq!(registrations.len(), 2);
        assert_eq!(registrations[0].entity_rust_name, "PublicCrudUser");
        assert_eq!(registrations[0].state, EntityState::Unchanged);
        assert_eq!(registrations[1].entity_rust_name, "VersionedPublicCrudUser");
        assert_eq!(registrations[1].state, EntityState::Unchanged);

        drop(tracked_user);
        drop(tracked_versioned_user);
        assert_eq!(registry.entry_count(), 0);

        Ok(())
    }
    .await;

    cleanup_test_table(&connection_string, keep_tables).await?;
    cleanup_versioned_test_table(&connection_string, keep_tables).await?;

    result
}

fn test_connection_string() -> Option<String> {
    std::env::var(TEST_CONNECTION_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn keep_test_tables() -> bool {
    matches!(
        std::env::var(KEEP_TABLES_ENV)
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn keep_test_rows() -> bool {
    matches!(
        std::env::var(KEEP_ROWS_ENV)
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn announce_test_table(keep_tables: bool, keep_rows: bool) {
    if keep_rows {
        eprintln!(
            "keeping public CRUD integration row in `{TEST_TABLE_NAME}` because {KEEP_ROWS_ENV}=1"
        );
    } else if keep_tables {
        eprintln!(
            "keeping public CRUD integration table `{TEST_TABLE_NAME}` because {KEEP_TABLES_ENV}=1"
        );
    } else {
        eprintln!("created public CRUD integration table `{TEST_TABLE_NAME}`");
    }
}

async fn public_crud_fixture_lock() -> MutexGuard<'static, ()> {
    static FIXTURE_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    FIXTURE_LOCK.get_or_init(|| Mutex::new(())).lock().await
}

async fn reset_test_table(connection_string: &str) -> Result<(), OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "IF OBJECT_ID('{TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {TEST_TABLE_NAME}"
            ),
            vec![],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {TEST_TABLE_NAME} (\
                    id BIGINT IDENTITY(1,1) PRIMARY KEY,\
                    name NVARCHAR(120) NOT NULL,\
                    active BIT NOT NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn reset_versioned_test_table(connection_string: &str) -> Result<(), OrmError> {
    let mut connection = MssqlConnection::connect(connection_string).await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "IF OBJECT_ID('{VERSIONED_TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {VERSIONED_TEST_TABLE_NAME}"
            ),
            vec![],
        ))
        .await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {VERSIONED_TEST_TABLE_NAME} (\
                    id BIGINT IDENTITY(1,1) PRIMARY KEY,\
                    name NVARCHAR(120) NOT NULL,\
                    version ROWVERSION NOT NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn cleanup_test_table(connection_string: &str, keep_tables: bool) -> Result<(), OrmError> {
    if keep_tables {
        return Ok(());
    }

    let mut connection = MssqlConnection::connect(connection_string).await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "IF OBJECT_ID('{TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {TEST_TABLE_NAME}"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn cleanup_versioned_test_table(
    connection_string: &str,
    keep_tables: bool,
) -> Result<(), OrmError> {
    if keep_tables {
        return Ok(());
    }

    let mut connection = MssqlConnection::connect(connection_string).await?;

    connection
        .execute(CompiledQuery::new(
            format!(
                "IF OBJECT_ID('{VERSIONED_TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {VERSIONED_TEST_TABLE_NAME}"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}
