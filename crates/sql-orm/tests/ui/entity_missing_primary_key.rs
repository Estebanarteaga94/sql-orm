use sql_orm::prelude::*;

#[derive(Entity)]
pub struct AuditLog {
    pub message: String,
}

fn main() {}
