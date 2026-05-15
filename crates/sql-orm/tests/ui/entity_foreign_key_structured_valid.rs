use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "customers", schema = "sales")]
pub struct Customer {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "sales")]
pub struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(column = "customer_id")]
    #[orm(foreign_key(entity = Customer, column = id))]
    pub customer_id_value: i64,
}

fn main() {
    let metadata = Order::metadata();
    let foreign_key = metadata
        .foreign_key("fk_orders_customer_id_customers")
        .expect("generated foreign key metadata");

    assert_eq!(metadata.schema, "sales");
    assert_eq!(metadata.table, "orders");
    assert_eq!(foreign_key.columns, &["customer_id"]);
    assert_eq!(foreign_key.referenced_schema, "sales");
    assert_eq!(foreign_key.referenced_table, "customers");
    assert_eq!(foreign_key.referenced_columns, &["id"]);
    assert_eq!(Order::customer_id_value.column_name(), "customer_id");
}
