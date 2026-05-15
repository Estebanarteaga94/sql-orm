use sql_orm::prelude::*;

#[derive(SoftDeleteFields)]
struct SoftDeleteColumns {
    #[orm(sql_type = "datetime2")]
    deleted_at: Option<String>,

    #[orm(nullable)]
    #[orm(length = 120)]
    deleted_by: Option<String>,
}

#[derive(Entity, Debug, Clone)]
#[orm(
    table = "soft_deleted_entities",
    schema = "audit",
    soft_delete = SoftDeleteColumns
)]
struct SoftDeletedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,
}

fn main() {
    let metadata = SoftDeletedEntity::metadata();
    assert_eq!(metadata.columns.len(), 4);
    assert_eq!(metadata.columns[2].column_name, "deleted_at");
    assert_eq!(metadata.columns[3].column_name, "deleted_by");

    let soft_delete = SoftDeletedEntity::soft_delete_policy().expect("soft delete policy");
    assert_eq!(soft_delete.name, "soft_delete");
    assert_eq!(soft_delete.columns.len(), 2);
    assert_eq!(soft_delete.columns[0].column_name, "deleted_at");
    assert!(!soft_delete.columns[0].insertable);
    assert!(soft_delete.columns[0].updatable);
    assert_eq!(soft_delete.columns[1].column_name, "deleted_by");

    let metadata = SoftDeleteColumns::metadata();
    assert_eq!(metadata.name, "soft_delete");
}
