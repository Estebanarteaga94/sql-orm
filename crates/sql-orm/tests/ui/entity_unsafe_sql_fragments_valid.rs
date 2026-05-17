use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "unsafe_fragments", schema = "dbo")]
pub struct UnsafeFragments {
    #[orm(primary_key)]
    id: i64,

    #[orm(unsafe_sql_type = "geography")]
    location: String,

    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    created_at: String,

    #[orm(unsafe_computed_sql = "[first_name] + N' ' + [last_name]")]
    full_name: String,
}

fn main() {
    let metadata = UnsafeFragments::metadata();
    assert_eq!(metadata.columns[1].sql_type, SqlServerType::Custom("geography"));
    assert_eq!(metadata.columns[2].default_sql, Some("SYSUTCDATETIME()"));
    assert_eq!(
        metadata.columns[3].computed_sql,
        Some("[first_name] + N' ' + [last_name]")
    );
}
