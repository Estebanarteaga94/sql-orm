use sql_orm::prelude::*;

#[derive(AuditFields)]
struct Audit {
    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    #[orm(updatable = false)]
    created_at: String,

    #[orm(column = "created_by_user_id")]
    created_by: Option<i64>,

    #[orm(nullable)]
    #[orm(length = 120)]
    updated_by: Option<String>,
}

fn main() {
    let metadata = Audit::metadata();

    assert_eq!(metadata.name, "audit");
    assert_eq!(metadata.columns.len(), 3);
    assert_eq!(metadata.columns[0].column_name, "created_at");
    assert_eq!(metadata.columns[0].sql_type, SqlServerType::DateTime2);
    assert_eq!(metadata.columns[1].column_name, "created_by_user_id");
    assert!(metadata.columns[1].nullable);
    assert_eq!(metadata.columns[2].max_length, Some(120));
}
