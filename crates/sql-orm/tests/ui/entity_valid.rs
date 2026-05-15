use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "customers", schema = "sales")]
pub struct Customer {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(length = 180)]
    #[orm(unique)]
    pub email: String,

    #[orm(nullable)]
    pub display_name: Option<String>,

    #[orm(default_sql = "SYSUTCDATETIME()")]
    pub created_at: String,

    #[orm(rowversion)]
    pub version: Vec<u8>,

    #[orm(foreign_key = "crm.accounts.id")]
    pub account_id: i64,
}

fn main() {
    let metadata = Customer::metadata();
    assert_eq!(metadata.schema, "sales");
    assert_eq!(metadata.table, "customers");
    assert_eq!(Customer::email.column_name(), "email");
    assert!(Customer::version.metadata().rowversion);
    assert_eq!(metadata.foreign_keys.len(), 1);
    assert_eq!(metadata.foreign_keys[0].referenced_schema, "crm");
    assert_eq!(metadata.foreign_keys[0].referenced_table, "accounts");
    assert_eq!(metadata.foreign_keys[0].referenced_columns, &["id"]);
}
