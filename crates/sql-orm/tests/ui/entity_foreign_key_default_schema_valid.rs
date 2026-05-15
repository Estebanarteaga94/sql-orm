use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "sales")]
pub struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(column = "customer_id")]
    #[orm(foreign_key = "customers.id")]
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
    assert_eq!(foreign_key.referenced_schema, "dbo");
    assert_eq!(foreign_key.referenced_table, "customers");
    assert_eq!(foreign_key.referenced_columns, &["id"]);
    assert_eq!(Order::customer_id_value.column_name(), "customer_id");
    assert_eq!(
        metadata.foreign_keys_for_column("customer_id")[0].name,
        "fk_orders_customer_id_customers"
    );
}
