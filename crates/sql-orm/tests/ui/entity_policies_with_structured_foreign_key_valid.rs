use sql_orm::prelude::*;
use uuid::Uuid;

#[derive(AuditFields)]
struct AuditField {
    #[orm(unsafe_default_sql = "SYSUTCDATETIME()")]
    #[orm(sql_type = "datetime2")]
    created_at: String,

    #[orm(nullable)]
    #[orm(sql_type = "datetime2")]
    updated_at: Option<String>,
}

#[derive(SoftDeleteFields)]
struct SoftDeleteField {
    #[orm(nullable)]
    #[orm(sql_type = "datetime2")]
    deleted_at: Option<String>,
}

#[derive(Entity, Clone, Debug)]
#[orm(table = "users")]
pub struct UserModel {
    #[orm(primary_key)]
    pub id: Uuid,

    pub email: String,
}

#[derive(Entity, Clone, Debug)]
#[orm(table = "tasks", audit = AuditField, soft_delete = SoftDeleteField)]
pub struct TaskModel {
    #[orm(primary_key)]
    pub id: Uuid,

    pub description: String,

    #[orm(foreign_key(entity = UserModel, column = id))]
    pub asigned_to: Uuid,

    pub title: String,
}

fn main() {
    let metadata = TaskModel::metadata();
    let foreign_key = metadata
        .foreign_key("fk_tasks_asigned_to_user_models")
        .expect("structured foreign key metadata");

    assert_eq!(metadata.table, "tasks");
    assert!(metadata.column("created_at").is_some());
    assert!(metadata.column("deleted_at").is_some());
    assert_eq!(metadata.primary_key.columns, &["id"]);
    assert_eq!(foreign_key.columns, &["asigned_to"]);
    assert_eq!(foreign_key.referenced_schema, "dbo");
    assert_eq!(foreign_key.referenced_table, "users");
    assert_eq!(foreign_key.referenced_columns, &["id"]);
}
