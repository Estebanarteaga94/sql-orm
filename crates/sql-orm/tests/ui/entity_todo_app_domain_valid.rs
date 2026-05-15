use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(length = 180)]
    #[orm(unique)]
    pub email: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists", schema = "todo")]
pub struct TodoList {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    #[orm(on_delete = "cascade")]
    pub owner_user_id: i64,

    #[orm(length = 160)]
    pub title: String,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_items", schema = "todo")]
#[orm(index(name = "ix_todo_items_list_position", columns(list_id, position)))]
pub struct TodoItem {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = TodoList, column = id))]
    #[orm(on_delete = "cascade")]
    pub list_id: i64,

    #[orm(foreign_key(entity = User, column = id, name = "fk_todo_items_created_by_user"))]
    pub created_by_user_id: i64,

    #[orm(nullable)]
    #[orm(foreign_key(entity = User, column = id, name = "fk_todo_items_completed_by_user"))]
    #[orm(on_delete = "set null")]
    pub completed_by_user_id: Option<i64>,

    pub position: i32,
}

fn main() {
    let list_metadata = TodoList::metadata();
    let item_metadata = TodoItem::metadata();

    assert_eq!(list_metadata.foreign_keys.len(), 1);
    assert_eq!(
        list_metadata.foreign_keys[0].name,
        "fk_todo_lists_owner_user_id_users"
    );
    assert_eq!(item_metadata.foreign_keys.len(), 3);
    assert_eq!(
        item_metadata
            .foreign_key("fk_todo_items_completed_by_user")
            .expect("completed by relationship")
            .on_delete,
        ReferentialAction::SetNull
    );
    assert_eq!(
        item_metadata.indexes[0]
            .columns
            .iter()
            .map(|column| column.column_name)
            .collect::<Vec<_>>(),
        vec!["list_id", "position"]
    );
}
