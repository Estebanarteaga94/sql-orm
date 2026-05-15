use sql_orm::prelude::*;

#[derive(SoftDeleteFields)]
struct SoftDelete {
    #[orm(deleted_at)]
    #[orm(sql_type = "datetime2")]
    deleted_at: String,

    #[orm(deleted_by)]
    #[orm(column = "deleted_by_user_id")]
    deleted_by: Option<i64>,

    #[orm(is_deleted)]
    #[orm(column = "deleted")]
    deleted: bool,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "soft_deleted_entities", schema = "audit", soft_delete = SoftDelete)]
struct SoftDeletedEntity {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(length = 120)]
    name: String,
}

#[derive(DbContext, Debug, Clone)]
struct AppDbContext {
    soft_deleted_entities: DbSet<SoftDeletedEntity>,
}

fn assert_soft_delete_values<T: SoftDeleteValues>(values: T) -> Vec<ColumnValue> {
    values.soft_delete_values()
}

fn main() {
    let soft_delete = SoftDelete {
        deleted_at: "2026-04-28T00:00:00Z".to_string(),
        deleted_by: Some(42),
        deleted: true,
    };

    let values = assert_soft_delete_values(soft_delete);

    assert_eq!(values[0].column_name, "deleted_at");
    assert_eq!(values[1].column_name, "deleted_by_user_id");
    assert_eq!(values[2].column_name, "deleted");

    let _with_typed_values = AppDbContext::with_soft_delete_values::<SoftDelete>;
    let _shared_with_typed_values = SharedConnection::with_soft_delete_values::<SoftDelete>;
}
