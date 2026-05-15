use sql_orm::prelude::*;

#[derive(Entity)]
pub struct InvalidIdentity {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: String,
}

fn main() {}
