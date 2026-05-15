use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "todo")]
pub struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(has_one(UserProfile, foreign_key = user_id))]
    pub profile: Navigation<UserProfile>,

    #[orm(has_many(TodoList, foreign_key = owner_id))]
    pub lists: Collection<TodoList>,

    #[orm(has_many(TodoList, foreign_key = owner_id))]
    pub lazy_lists: LazyCollection<TodoList>,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "user_profiles", schema = "todo")]
pub struct UserProfile {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub user_id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "todo_lists", schema = "todo")]
pub struct TodoList {
    #[orm(primary_key)]
    #[orm(identity)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub owner_id: i64,

    #[orm(belongs_to(User, foreign_key = owner_id))]
    pub owner: Navigation<User>,

    #[orm(belongs_to(User, foreign_key = owner_id))]
    pub lazy_owner: LazyNavigation<User>,
}

fn main() {
    let list_metadata = TodoList::metadata();
    let owner = list_metadata
        .navigation("owner")
        .expect("belongs_to navigation metadata");

    let lazy_owner = list_metadata
        .navigation("lazy_owner")
        .expect("lazy belongs_to navigation metadata");

    assert_eq!(list_metadata.columns.len(), 2);
    assert!(list_metadata.field("owner").is_none());
    assert!(list_metadata.field("lazy_owner").is_none());
    assert_eq!(owner.kind, NavigationKind::BelongsTo);
    assert_eq!(owner.target_rust_name, "User");
    assert_eq!(owner.target_schema, "todo");
    assert_eq!(owner.target_table, "users");
    assert_eq!(owner.local_columns, &["owner_id"]);
    assert_eq!(owner.target_columns, &["id"]);
    assert_eq!(owner.foreign_key_name, Some("fk_todo_lists_owner_id_users"));
    assert_eq!(lazy_owner.kind, NavigationKind::BelongsTo);
    assert_eq!(lazy_owner.target_rust_name, "User");

    let user_metadata = User::metadata();
    let lists = user_metadata
        .navigation("lists")
        .expect("has_many navigation metadata");
    let lazy_lists = user_metadata
        .navigation("lazy_lists")
        .expect("lazy has_many navigation metadata");

    assert_eq!(user_metadata.columns.len(), 1);
    assert!(user_metadata.field("lists").is_none());
    assert!(user_metadata.field("lazy_lists").is_none());
    assert!(user_metadata.field("profile").is_none());

    let profile = user_metadata
        .navigation("profile")
        .expect("has_one navigation metadata");
    assert_eq!(profile.kind, NavigationKind::HasOne);
    assert_eq!(profile.target_rust_name, "UserProfile");
    assert_eq!(profile.local_columns, &["id"]);
    assert_eq!(profile.target_columns, &["user_id"]);
    assert_eq!(
        profile.foreign_key_name,
        Some("fk_user_profiles_user_id_users")
    );

    assert_eq!(lists.kind, NavigationKind::HasMany);
    assert_eq!(lists.target_rust_name, "TodoList");
    assert_eq!(lists.local_columns, &["id"]);
    assert_eq!(lists.target_columns, &["owner_id"]);
    assert_eq!(lists.foreign_key_name, Some("fk_todo_lists_owner_id_users"));
    assert_eq!(lazy_lists.kind, NavigationKind::HasMany);
    assert_eq!(lazy_lists.target_rust_name, "TodoList");
}
