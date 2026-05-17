use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "sales")]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    customer_id: i64,
    total_cents: i64,
    tax_rate: f64,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub orders: DbSet<Order>,
}

fn main() {
    let _build_query = |db: &AppDbContext| {
        let _count_future = db.orders.query().count();
        let _exists_future = db.orders.query().exists();
        let _any_future = db.orders.query().any();

        let _sum_future = db.orders.query().sum::<i64>(Order::total_cents);
        let _avg_future = db.orders.query().avg::<f64>(Order::tax_rate);
        let _min_future = db.orders.query().min::<i64>(Order::customer_id);
        let _max_future = db.orders.query().max::<i64>(Order::total_cents);
    };
}
