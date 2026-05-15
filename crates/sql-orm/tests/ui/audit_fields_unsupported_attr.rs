use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    #[orm(primary_key)]
    created_at: String,
}

fn main() {}
