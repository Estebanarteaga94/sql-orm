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

#[derive(Debug, Clone, PartialEq)]
struct OrderTotals {
    customer_id: i64,
    order_count: i64,
    total_cents: Option<i64>,
}

impl FromRow for OrderTotals {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            customer_id: row.get_required_typed::<i64>("customer_id")?,
            order_count: row.get_required_typed::<i64>("order_count")?,
            total_cents: row.try_get_typed::<i64>("total_cents")?,
        })
    }
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

        let _grouped_future = db
            .orders
            .query()
            .group_by(Order::customer_id)
            .expect("group_by should accept one group key")
            .select_aggregate((
                AggregateProjection::group_key(Order::customer_id),
                AggregateProjection::count_as("order_count"),
                AggregateProjection::sum_as(Order::total_cents, "total_cents"),
            ))
            .having(AggregatePredicate::gt(
                AggregateExpr::count_all(),
                SqlValue::I64(1),
            ))
            .order_by(AggregateOrderBy::desc(AggregateExpr::sum(
                Order::total_cents,
            )))
            .limit(10)
            .all_as::<OrderTotals>();

        let _first_group_future = db
            .orders
            .query()
            .group_by([Order::customer_id])
            .expect("group_by should accept arrays")
            .select_aggregate([
                AggregateProjection::group_key(Order::customer_id),
                AggregateProjection::max_as(Order::total_cents, "total_cents"),
            ])
            .first_as::<OrderTotals>();
    };
}
