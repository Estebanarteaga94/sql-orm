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

fn main() {
    let mut tracked = Tracked::from_loaded(User {
        id: 1,
        email: "ana@example.com".to_string(),
    });

    tracked.attach_registry(());
}
