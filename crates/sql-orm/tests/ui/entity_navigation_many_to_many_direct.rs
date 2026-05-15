use sql_orm::prelude::*;

#[derive(Entity, Debug, Clone)]
#[orm(table = "users")]
pub struct User {
    #[orm(primary_key)]
    pub id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "groups")]
pub struct Group {
    #[orm(primary_key)]
    pub id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "user_groups")]
pub struct UserGroup {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    pub user_id: i64,

    #[orm(foreign_key(entity = Group, column = id))]
    pub group_id: i64,
}

#[derive(Entity, Debug, Clone)]
#[orm(table = "accounts")]
pub struct Account {
    #[orm(primary_key)]
    pub id: i64,

    #[orm(many_to_many(Group, through = UserGroup))]
    pub groups: Collection<Group>,
}

fn main() {}
