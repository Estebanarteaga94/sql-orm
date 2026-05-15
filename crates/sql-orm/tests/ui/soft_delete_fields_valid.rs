use sql_orm::prelude::*;

#[derive(SoftDeleteFields)]
struct SoftDelete {
    #[orm(sql_type = "datetime2")]
    deleted_at: Option<String>,

    #[orm(column = "deleted_by_user_id")]
    deleted_by: Option<i64>,
}

fn main() {
    let metadata = SoftDelete::metadata();

    assert_eq!(metadata.name, "soft_delete");
    assert_eq!(metadata.columns.len(), 2);
    assert_eq!(metadata.columns[0].column_name, "deleted_at");
    assert_eq!(metadata.columns[0].sql_type, SqlServerType::DateTime2);
    assert!(metadata.columns[0].nullable);
    assert!(!metadata.columns[0].insertable);
    assert!(metadata.columns[0].updatable);
    assert_eq!(metadata.columns[1].column_name, "deleted_by_user_id");
    assert_eq!(metadata.columns[1].sql_type, SqlServerType::BigInt);
    assert!(metadata.columns[1].nullable);
}
