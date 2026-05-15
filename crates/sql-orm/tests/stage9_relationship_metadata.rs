use sql_orm::prelude::*;

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "customers", schema = "sales")]
struct Customer {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "users", schema = "dbo")]
struct User {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
}

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "orders", schema = "sales")]
struct Order {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(foreign_key(entity = Customer, column = id))]
    customer_id: i64,

    #[orm(column = "approver_user_id")]
    #[orm(foreign_key(entity = User, column = id))]
    approved_by: i64,

    total_cents: i64,
}

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "order_notes", schema = "sales")]
struct OrderNote {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(foreign_key(entity = Order, column = id))]
    #[orm(on_delete = "cascade")]
    order_id: i64,

    #[orm(foreign_key(entity = User, column = id))]
    #[orm(on_delete = "set null")]
    reviewer_id: Option<i64>,
}

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "projects", schema = "sales")]
struct Project {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    name: String,

    #[orm(has_many(ProjectTask, foreign_key = project_id))]
    tasks: Collection<ProjectTask>,
}

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "project_tasks", schema = "sales")]
struct ProjectTask {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,

    #[orm(column = "project_ref")]
    #[orm(foreign_key(entity = Project, column = id, name = "fk_tasks_project_ref"))]
    project_id: i64,

    #[orm(belongs_to(Project, foreign_key = project_id))]
    project: Navigation<Project>,
}

#[test]
fn derives_relationship_metadata_for_multiple_foreign_keys() {
    let metadata = Order::metadata();

    assert_eq!(metadata.foreign_keys.len(), 2);

    let customer_fk = metadata
        .foreign_key("fk_orders_customer_id_customers")
        .expect("customer foreign key metadata");
    assert_eq!(customer_fk.columns, &["customer_id"]);
    assert_eq!(customer_fk.referenced_schema, "sales");
    assert_eq!(customer_fk.referenced_table, "customers");
    assert_eq!(customer_fk.referenced_columns, &["id"]);
    assert_eq!(customer_fk.on_delete, ReferentialAction::NoAction);
    assert_eq!(customer_fk.on_update, ReferentialAction::NoAction);

    let approver_fk = metadata
        .foreign_key("fk_orders_approver_user_id_users")
        .expect("approver foreign key metadata");
    assert_eq!(approver_fk.columns, &["approver_user_id"]);
    assert_eq!(approver_fk.referenced_schema, "dbo");
    assert_eq!(approver_fk.referenced_table, "users");
    assert_eq!(approver_fk.referenced_columns, &["id"]);

    assert_eq!(Order::approved_by.column_name(), "approver_user_id");
    assert_eq!(
        Order::approved_by.metadata().column_name,
        approver_fk.columns[0]
    );
}

#[test]
fn relationship_metadata_helpers_filter_generated_foreign_keys() {
    let metadata = Order::metadata();

    let customer_column_matches = metadata.foreign_keys_for_column("customer_id");
    assert_eq!(customer_column_matches.len(), 1);
    assert_eq!(
        customer_column_matches[0].name,
        "fk_orders_customer_id_customers"
    );

    let approver_column_matches = metadata.foreign_keys_for_column("approver_user_id");
    assert_eq!(approver_column_matches.len(), 1);
    assert_eq!(
        approver_column_matches[0].name,
        "fk_orders_approver_user_id_users"
    );

    let sales_customer_refs = metadata.foreign_keys_referencing("sales", "customers");
    assert_eq!(sales_customer_refs.len(), 1);
    assert_eq!(
        sales_customer_refs[0].name,
        "fk_orders_customer_id_customers"
    );

    let dbo_user_refs = metadata.foreign_keys_referencing("dbo", "users");
    assert_eq!(dbo_user_refs.len(), 1);
    assert_eq!(dbo_user_refs[0].name, "fk_orders_approver_user_id_users");

    assert!(metadata.foreign_keys_for_column("total_cents").is_empty());
    assert!(
        metadata
            .foreign_keys_referencing("dbo", "accounts")
            .is_empty()
    );
}

#[test]
fn derives_delete_behavior_metadata_for_foreign_keys() {
    let metadata = OrderNote::metadata();

    let order_fk = metadata
        .foreign_key("fk_order_notes_order_id_orders")
        .expect("order foreign key metadata");
    assert_eq!(order_fk.on_delete, ReferentialAction::Cascade);
    assert_eq!(order_fk.on_update, ReferentialAction::NoAction);

    let reviewer_fk = metadata
        .foreign_key("fk_order_notes_reviewer_id_users")
        .expect("reviewer foreign key metadata");
    assert_eq!(reviewer_fk.on_delete, ReferentialAction::SetNull);
    assert_eq!(reviewer_fk.on_update, ReferentialAction::NoAction);
    assert_eq!(reviewer_fk.columns, &["reviewer_id"]);
    assert_eq!(reviewer_fk.referenced_schema, "dbo");
    assert_eq!(reviewer_fk.referenced_table, "users");
}

#[test]
fn belongs_to_navigation_metadata_uses_structured_foreign_key_columns() {
    let metadata = ProjectTask::metadata();

    assert_eq!(metadata.columns.len(), 2);
    assert!(metadata.field("project").is_none());

    let foreign_key = metadata
        .foreign_key("fk_tasks_project_ref")
        .expect("structured project foreign key");
    assert_eq!(foreign_key.columns, &["project_ref"]);
    assert_eq!(foreign_key.referenced_schema, "sales");
    assert_eq!(foreign_key.referenced_table, "projects");
    assert_eq!(foreign_key.referenced_columns, &["id"]);

    let navigation = metadata
        .navigation("project")
        .expect("belongs_to project navigation");
    assert_eq!(navigation.kind, NavigationKind::BelongsTo);
    assert_eq!(navigation.target_rust_name, "Project");
    assert_eq!(navigation.target_schema, "sales");
    assert_eq!(navigation.target_table, "projects");
    assert_eq!(navigation.local_columns, &["project_ref"]);
    assert_eq!(navigation.target_columns, &["id"]);
    assert_eq!(navigation.foreign_key_name, Some("fk_tasks_project_ref"));
    assert!(navigation.uses_foreign_key("fk_tasks_project_ref"));

    let by_foreign_key = metadata.navigations_for_foreign_key("fk_tasks_project_ref");
    assert_eq!(by_foreign_key.len(), 1);
    assert_eq!(by_foreign_key[0].rust_field, "project");
}

#[test]
fn belongs_to_navigation_can_receive_included_entity() {
    let mut task = ProjectTask {
        id: 10,
        project_id: 7,
        project: Navigation::empty(),
    };

    task.set_included_navigation(
        "project",
        Some(Project {
            id: 7,
            name: "Roadmap".to_string(),
            tasks: Collection::empty(),
        }),
    )
    .unwrap();

    assert_eq!(task.project.as_ref().map(|project| project.id), Some(7));
}

#[test]
fn has_many_navigation_can_receive_included_collection() {
    let mut project = Project {
        id: 7,
        name: "Roadmap".to_string(),
        tasks: Collection::empty(),
    };

    project
        .set_included_collection(
            "tasks",
            vec![
                ProjectTask {
                    id: 10,
                    project_id: 7,
                    project: Navigation::empty(),
                },
                ProjectTask {
                    id: 11,
                    project_id: 7,
                    project: Navigation::empty(),
                },
            ],
        )
        .unwrap();

    assert_eq!(project.tasks.as_slice().len(), 2);
    assert_eq!(project.tasks.as_slice()[0].id, 10);
    assert_eq!(project.tasks.as_slice()[1].id, 11);
}

#[test]
fn inverse_navigation_metadata_reuses_target_foreign_key_metadata() {
    let metadata = Project::metadata();

    assert_eq!(metadata.columns.len(), 2);
    assert!(metadata.field("tasks").is_none());

    let navigation = metadata
        .navigation("tasks")
        .expect("has_many project tasks navigation");
    assert_eq!(navigation.kind, NavigationKind::HasMany);
    assert_eq!(navigation.target_rust_name, "ProjectTask");
    assert_eq!(navigation.target_schema, "sales");
    assert_eq!(navigation.target_table, "project_tasks");
    assert_eq!(navigation.local_columns, &["id"]);
    assert_eq!(navigation.target_columns, &["project_ref"]);
    assert_eq!(navigation.foreign_key_name, Some("fk_tasks_project_ref"));

    let by_foreign_key = metadata.navigations_for_foreign_key("fk_tasks_project_ref");
    assert_eq!(by_foreign_key.len(), 1);
    assert_eq!(by_foreign_key[0].rust_field, "tasks");
}
