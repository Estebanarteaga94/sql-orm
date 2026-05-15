use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "sales")]
pub struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key = "customers.id")]
    #[orm(on_delete = "set null")]
    pub customer_id: i64,
}

fn main() {}
