use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    created_at: String,

    #[orm(column = "created_at")]
    created_on: String,
}

fn main() {}
