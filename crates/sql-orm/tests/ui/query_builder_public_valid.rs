use sql_orm::prelude::*;
use sql_orm::query::{Expr, Predicate, SelectQuery};

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 180)]
    email: String,

    active: bool,

    #[orm(has_many(Order, foreign_key = user_id))]
    orders: LazyCollection<Order>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "dbo")]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    user_id: i64,
    total_cents: i64,

    #[orm(belongs_to(User, foreign_key = user_id))]
    user: LazyNavigation<User>,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub users: DbSet<User>,
    pub orders: DbSet<Order>,
}

fn accept_query(_query: DbSetQuery<User>) {}

fn main() {
    let _build_query = |db: &AppDbContext| {
        accept_query(
            db.users
                .query()
                .inner_join::<Order>(Predicate::eq(
                    Expr::from(User::id),
                    Expr::from(Order::user_id),
                ))
                .left_join::<Order>(Order::total_cents.gt(0_i64))
                .filter(User::active.eq(true).and(User::email.contains("@example.com")))
                .order_by(User::email.asc())
                .limit(10)
                .paginate(PageRequest::new(2, 10)),
        );

        let _aliased_query = SelectQuery::from_entity_as::<User>("u")
            .select([
                SelectProjection::from(User::email.aliased("u")),
                SelectProjection::from(Order::total_cents.aliased("orders")),
            ])
            .inner_join_as::<Order>(
                "orders",
                Predicate::eq(
                    Expr::from(User::id.aliased("u")),
                    Expr::from(Order::user_id.aliased("orders")),
                ),
            )
            .filter(Order::total_cents.aliased("orders").gte(1000_i64))
            .order_by(Order::total_cents.aliased("orders").desc());

        let _navigation_join_query = db
            .users
            .query()
            .try_inner_join_navigation_as::<Order>("orders", "orders")
            .unwrap()
            .filter(Order::total_cents.aliased("orders").gte(1000_i64));

        let _include_query = db
            .orders
            .query()
            .include::<User>("user")
            .unwrap()
            .filter(User::id.aliased("user").gt(0_i64))
            .order_by(User::id.aliased("user").desc())
            .take(5);

        let _include_many_query = db
            .users
            .query()
            .include_many_as::<Order>("orders", "orders")
            .unwrap()
            .join_strategy()
            .max_joined_rows(1_000)
            .filter(Order::total_cents.aliased("orders").gte(1000_i64))
            .order_by(Order::total_cents.aliased("orders").desc());

        let _include_many_split_query = db
            .users
            .query()
            .include_many::<Order>("orders")
            .unwrap()
            .split_query();

        let _include_many_unbounded_join = db
            .users
            .query()
            .include_many::<Order>("orders")
            .unwrap()
            .unbounded_join();

        let mut user = User {
            id: 1,
            email: "user@example.com".to_string(),
            active: true,
            orders: LazyCollection::unloaded(),
        };
        let _load_collection = db.users.load_collection::<Order>(&mut user, "orders");

        let mut tracked_user = Tracked::from_loaded(User {
            id: 1,
            email: "tracked@example.com".to_string(),
            active: true,
            orders: LazyCollection::unloaded(),
        });
        let _load_tracked_collection =
            db.users
                .load_collection_tracked::<Order>(&mut tracked_user, "orders");
    };
}
