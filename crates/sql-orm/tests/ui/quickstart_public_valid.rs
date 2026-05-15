use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "quickstart_users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,

    active: bool,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = User)]
struct NewUser {
    name: String,
    active: bool,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = User)]
struct UpdateUser {
    name: Option<String>,
    active: Option<bool>,
}

#[derive(DbContext, Debug, Clone)]
struct AppDb {
    pub users: DbSet<User>,
}

async fn quickstart(db: &AppDb) -> Result<(), OrmError> {
    let inserted = db
        .users
        .insert(NewUser {
            name: "Ana".to_string(),
            active: true,
        })
        .await?;

    let _found = db.users.find(inserted.id).await?;

    let _active_users = db
        .users
        .query()
        .filter(User::active.eq(true))
        .order_by(User::name.asc())
        .take(10)
        .all()
        .await?;

    let _updated = db
        .users
        .update(
            inserted.id,
            UpdateUser {
                name: Some("Ana Maria".to_string()),
                active: Some(false),
            },
        )
        .await?;

    let _deleted = db.users.delete(inserted.id).await?;

    Ok(())
}

fn main() {
    let _connect = AppDb::connect;
    let _quickstart = quickstart;
}
