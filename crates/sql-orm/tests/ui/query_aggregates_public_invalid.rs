use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "sales")]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    customer_id: i64,
    total_cents: i64,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    pub orders: DbSet<Order>,
}

fn main() {
    let _invalid_surface = |db: &AppDbContext| {
        let grouped = db
            .orders
            .query()
            .group_by(Order::customer_id)
            .expect("group_by shape is valid");

        let _entity_materialization = grouped.all();

        let _missing_alias_projection = db
            .orders
            .query()
            .group_by(Order::customer_id)
            .expect("group_by shape is valid")
            .select_aggregate(AggregateExpr::sum(Order::total_cents));
    };
}
