use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    #[orm(column = "")]
    created_at: String,
}

fn main() {}
