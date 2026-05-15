use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(has_many(Order, foreign_key = user_id))]
    orders: Collection<Order>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "dbo")]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    user_id: i64,

    #[orm(belongs_to(User, foreign_key = user_id))]
    user: Navigation<User>,
}

#[derive(Debug, FromRow)]
struct UserDto {
    id: i64,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub users: DbSet<User>,
    pub orders: DbSet<Order>,
}

fn main() {
    let _build_query = |db: &AppDbContext| {
        let _single_projection = db
            .orders
            .query()
            .include::<User>("user")
            .unwrap()
            .select(User::id);

        let _single_dto = db
            .orders
            .query()
            .include::<User>("user")
            .unwrap()
            .all_as::<UserDto>();

        let _collection_projection = db
            .users
            .query()
            .include_many::<Order>("orders")
            .unwrap()
            .select(User::id);

        let _collection_dto = db
            .users
            .query()
            .include_many::<Order>("orders")
            .unwrap()
            .all_as::<UserDto>();
    };
}
