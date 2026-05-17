use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "custom_types", schema = "dbo")]
pub struct CustomTypes {
    #[orm(primary_key)]
    id: i64,

    #[orm(sql_type = "geography")]
    location: String,
}

fn main() {}
