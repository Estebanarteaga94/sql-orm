use sql_orm::prelude::*;

struct Unsupported;

#[derive(AuditFields)]
struct Audit {
    created_at: Unsupported,
}

fn main() {}
