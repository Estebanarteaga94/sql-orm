use sql_orm::prelude::*;

#[derive(Entity)]
pub struct InvalidRowVersion {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(rowversion)]
    pub version: String,
}

fn main() {}
