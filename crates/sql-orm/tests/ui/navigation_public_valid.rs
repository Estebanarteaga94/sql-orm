use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "app")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,

    #[orm(has_one(Profile, foreign_key = user_id))]
    profile: Navigation<Profile>,

    #[orm(has_many(Post, foreign_key = user_id))]
    posts: Collection<Post>,

    #[orm(has_many(UserRole, foreign_key = user_id))]
    user_roles: LazyCollection<UserRole>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "profiles", schema = "app")]
struct Profile {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    user_id: i64,

    #[orm(length = 120)]
    bio: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "posts", schema = "app")]
struct Post {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    user_id: i64,

    #[orm(length = 120)]
    title: String,

    #[orm(belongs_to(User, foreign_key = user_id))]
    user: Navigation<User>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "roles", schema = "app")]
struct Role {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 80)]
    name: String,

    #[orm(has_many(UserRole, foreign_key = role_id))]
    user_roles: Collection<UserRole>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "user_roles", schema = "app")]
struct UserRole {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(foreign_key(entity = User, column = id, name = "fk_user_roles_user"))]
    user_id: i64,

    #[orm(foreign_key(entity = Role, column = id, name = "fk_user_roles_role"))]
    role_id: i64,

    #[orm(belongs_to(User, foreign_key = user_id))]
    user: Navigation<User>,

    #[orm(belongs_to(Role, foreign_key = role_id))]
    role: Navigation<Role>,
}

#[derive(DbContext, Debug, Clone)]
struct AppDb {
    pub users: DbSet<User>,
    pub profiles: DbSet<Profile>,
    pub posts: DbSet<Post>,
    pub roles: DbSet<Role>,
    pub user_roles: DbSet<UserRole>,
}

fn main() {
    let _surface = |db: &AppDb| {
        let _single_include = db
            .posts
            .query()
            .include_as::<User>("user", "user")
            .unwrap()
            .filter(User::id.aliased("user").gt(0_i64))
            .order_by(User::id.aliased("user").asc())
            .take(10);

        let _has_one_include = db
            .users
            .query()
            .include::<Profile>("profile")
            .unwrap()
            .first();

        let _collection_include = db
            .users
            .query()
            .include_many_as::<Post>("posts", "posts")
            .unwrap()
            .max_joined_rows(500)
            .filter(Post::id.aliased("posts").gt(0_i64))
            .all();

        let _navigation_join = db
            .users
            .query()
            .try_left_join_navigation_as::<UserRole>("user_roles", "user_roles")
            .unwrap()
            .try_inner_join_navigation_as::<Post>("posts", "posts")
            .unwrap()
            .filter(Post::id.aliased("posts").gt(0_i64));

        let mut user = User {
            id: 1,
            name: "Ana".to_string(),
            profile: Navigation::empty(),
            posts: Collection::empty(),
            user_roles: LazyCollection::unloaded(),
        };
        let _explicit_load = db.users.load_collection::<Post>(&mut user, "posts");

        let mut tracked = Tracked::from_loaded(User {
            id: 2,
            name: "Bruno".to_string(),
            profile: Navigation::empty(),
            posts: Collection::empty(),
            user_roles: LazyCollection::unloaded(),
        });
        let _tracked_explicit_load = db
            .users
            .load_collection_tracked::<Post>(&mut tracked, "posts");

        let _many_to_many_through_join_entity = db
            .user_roles
            .query()
            .try_left_join_navigation_as::<Role>("role", "role")
            .unwrap()
            .include::<User>("user")
            .unwrap()
            .filter(Role::id.aliased("role").gt(0_i64));
    };
}
