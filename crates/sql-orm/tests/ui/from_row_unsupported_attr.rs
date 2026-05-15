use sql_orm::prelude::*;

#[derive(FromRow)]
struct Projection {
    #[orm(length = 120)]
    name: String,
}

fn main() {}
