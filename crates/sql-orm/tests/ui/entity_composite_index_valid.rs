use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "sales")]
#[orm(index(name = "ix_orders_customer_total", columns(customer_id, total_cents)))]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    customer_id: i64,

    total_cents: i64,
}

fn main() {
    let metadata = Order::metadata();
    let index = metadata
        .indexes
        .iter()
        .find(|index| index.name == "ix_orders_customer_total")
        .expect("composite index should exist");

    assert_eq!(index.columns.len(), 2);
    assert_eq!(index.columns[0].column_name, "customer_id");
    assert_eq!(index.columns[1].column_name, "total_cents");
}
