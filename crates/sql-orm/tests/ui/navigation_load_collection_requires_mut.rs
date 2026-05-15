use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users")]
struct User {
    #[orm(primary_key)]
    id: i64,

    #[orm(has_many(Post, foreign_key = user_id))]
    posts: Collection<Post>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "posts")]
struct Post {
    #[orm(primary_key)]
    id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    user_id: i64,
}

#[derive(DbContext)]
struct AppDb {
    pub users: DbSet<User>,
}

fn main() {
    let _load = |db: &AppDb| {
        let user = User {
            id: 1,
            posts: Collection::empty(),
        };

        let _future = db.users.load_collection::<Post>(&user, "posts");
    };
}
