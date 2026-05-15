use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "customers", schema = "sales")]
pub struct Customer {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(column = "email_address")]
    #[orm(renamed_from = "email")]
    pub email: String,
}

fn main() {
    let metadata = Customer::metadata();
    let column = metadata.column("email_address").expect("renamed column metadata");

    assert_eq!(Customer::email.column_name(), "email_address");
    assert_eq!(column.renamed_from, Some("email"));
}
