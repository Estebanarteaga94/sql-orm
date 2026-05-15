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

fn main() {
    let _tracked_save = |db: &AppDbContext| {
        let mut tracked = Tracked::from_loaded(User {
            id: 1,
            email: "ana@example.com".to_string(),
        });

        let future = tracked.save(db);
        let _ = future;
    };

    let _tracked_delete = |db: &AppDbContext| {
        let mut tracked = db.users.add_tracked(User {
            id: 0,
            email: "luis@example.com".to_string(),
        });

        let future = tracked.delete(db);
        let _ = future;
    };
}
