use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 180)]
    email: String,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub users: DbSet<User>,
}

async fn tracking_context_surface(db: &AppDbContext) -> Result<(), OrmError> {
    if let Some(mut tracked) = db.users.find_tracked(1_i64).await? {
        db.users.remove_tracked(&mut tracked);
        db.users.detach_tracked(&mut tracked);
    }

    let mut added = db.users.add_tracked(User {
        id: 0,
        email: "luis@example.com".to_string(),
    });
    db.users.remove_tracked(&mut added);

    let _: usize = db.save_changes().await?;
    db.clear_tracker();

    Ok(())
}

fn main() {}
