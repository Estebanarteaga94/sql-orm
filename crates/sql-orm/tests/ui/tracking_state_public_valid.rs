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
    let mut tracked = Tracked::from_loaded(User {
        id: 1,
        email: "ana@example.com".to_string(),
    });

    let _: EntityState = tracked.state();
    tracked.mark_modified();
    tracked.mark_deleted();
    tracked.mark_unchanged();
    tracked.detach();

    let _context_surface = |db: &AppDbContext| {
        let mut added = db.users.add_tracked(User {
            id: 0,
            email: "luis@example.com".to_string(),
        });

        db.users.detach_tracked(&mut added);
        db.clear_tracker();
    };
}
