use sql_orm::prelude::*;

#[derive(Insertable, Debug, Clone)]
struct NewCustomer {
    email: String,
}

fn main() {}
