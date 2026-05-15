use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 180)]
    #[orm(unique)]
    email: String,

    #[orm(nullable)]
    display_name: Option<String>,

    #[orm(rowversion)]
    version: Vec<u8>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists", schema = "todo")]
struct TodoList {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    #[orm(on_delete = "cascade")]
    owner_user_id: i64,

    #[orm(length = 160)]
    title: String,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = User)]
struct NewUser {
    email: String,
    display_name: Option<String>,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = User)]
struct UpdateUser {
    email: Option<String>,
    display_name: Option<Option<String>>,
}

#[derive(DbContext, Debug, Clone)]
struct TodoDb {
    pub users: DbSet<User>,
    pub todo_lists: DbSet<TodoList>,
}

async fn code_first_flow(db: &TodoDb) -> Result<(), OrmError> {
    let inserted = db
        .users
        .insert(NewUser {
            email: "ana@example.com".to_string(),
            display_name: Some("Ana".to_string()),
        })
        .await?;

    let _found = db.users.find(inserted.id).await?;

    let _users = db
        .users
        .query()
        .filter(User::email.contains("@example.com"))
        .order_by(User::email.asc())
        .take(20)
        .all()
        .await?;

    let _updated = db
        .users
        .update(
            inserted.id,
            UpdateUser {
                email: None,
                display_name: Some(Some("Ana Maria".to_string())),
            },
        )
        .await?;

    let _deleted = db.users.delete(inserted.id).await?;

    Ok(())
}

fn main() {
    let user_metadata = User::metadata();
    let list_metadata = TodoList::metadata();

    assert_eq!(user_metadata.schema, "todo");
    assert_eq!(user_metadata.table, "users");
    assert_eq!(user_metadata.primary_key.columns, &["id"]);
    assert_eq!(list_metadata.foreign_keys.len(), 1);

    let _connect = TodoDb::connect;
    let _connect_with_options = TodoDb::connect_with_options;
    let _connect_with_config = TodoDb::connect_with_config;
    let _from_connection = TodoDb::from_connection;
    let _from_shared = TodoDb::from_shared_connection;
    let _transaction = TodoDb::transaction::<
        fn(TodoDb) -> std::future::Ready<Result<(), OrmError>>,
        std::future::Ready<Result<(), OrmError>>,
        (),
    >;
    let _code_first_flow = code_first_flow;
}
