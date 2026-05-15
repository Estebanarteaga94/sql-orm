use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "clients", schema = "sales", renamed_from = "customers")]
pub struct Client {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    pub email: String,
}

fn main() {
    let metadata = Client::metadata();

    assert_eq!(metadata.table, "clients");
    assert_eq!(metadata.renamed_from, Some("customers"));
}
