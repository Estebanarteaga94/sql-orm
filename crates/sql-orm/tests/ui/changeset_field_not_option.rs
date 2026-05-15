use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
struct Customer {
    id: i64,
    email: String,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = Customer)]
struct UpdateCustomer {
    email: String,
}

fn main() {}
