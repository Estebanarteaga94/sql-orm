use insta::assert_snapshot;
use sql_orm_core::{
    Changeset, ColumnMetadata, ColumnValue, Entity, EntityColumn, EntityMetadata, IdentityMetadata,
    Insertable, PrimaryKeyMetadata, SqlServerType, SqlValue,
};
use sql_orm_query::{
    CountQuery, DeleteQuery, Expr, InsertQuery, OrderBy, Pagination, Predicate, SelectQuery,
    SortDirection, TableRef, UpdateQuery,
};
use sql_orm_sqlserver::SqlServerCompiler;

#[allow(dead_code)]
struct Customer;

#[allow(dead_code)]
struct Order;

static CUSTOMER_COLUMNS: [ColumnMetadata; 4] = [
    ColumnMetadata {
        rust_field: "id",
        column_name: "id",
        renamed_from: None,
        sql_type: SqlServerType::BigInt,
        nullable: false,
        primary_key: true,
        identity: Some(IdentityMetadata::new(1, 1)),
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: false,
        updatable: false,
        max_length: None,
        precision: None,
        scale: None,
    },
    ColumnMetadata {
        rust_field: "email",
        column_name: "email",
        renamed_from: None,
        sql_type: SqlServerType::NVarChar,
        nullable: false,
        primary_key: false,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: true,
        updatable: true,
        max_length: Some(160),
        precision: None,
        scale: None,
    },
    ColumnMetadata {
        rust_field: "active",
        column_name: "active",
        renamed_from: None,
        sql_type: SqlServerType::Bit,
        nullable: false,
        primary_key: false,
        identity: None,
        default_sql: Some("1"),
        computed_sql: None,
        rowversion: false,
        insertable: true,
        updatable: true,
        max_length: None,
        precision: None,
        scale: None,
    },
    ColumnMetadata {
        rust_field: "created_at",
        column_name: "created_at",
        renamed_from: None,
        sql_type: SqlServerType::DateTime2,
        nullable: false,
        primary_key: false,
        identity: None,
        default_sql: Some("SYSUTCDATETIME()"),
        computed_sql: None,
        rowversion: false,
        insertable: true,
        updatable: true,
        max_length: None,
        precision: None,
        scale: None,
    },
];

static CUSTOMER_METADATA: EntityMetadata = EntityMetadata {
    rust_name: "Customer",
    schema: "sales",
    table: "customers",
    renamed_from: None,
    columns: &CUSTOMER_COLUMNS,
    primary_key: PrimaryKeyMetadata::new(Some("pk_customers"), &["id"]),
    indexes: &[],
    foreign_keys: &[],
    navigations: &[],
};

impl Entity for Customer {
    fn metadata() -> &'static EntityMetadata {
        &CUSTOMER_METADATA
    }
}

static ORDER_COLUMNS: [ColumnMetadata; 3] = [
    ColumnMetadata {
        rust_field: "id",
        column_name: "id",
        renamed_from: None,
        sql_type: SqlServerType::BigInt,
        nullable: false,
        primary_key: true,
        identity: Some(IdentityMetadata::new(1, 1)),
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: false,
        updatable: false,
        max_length: None,
        precision: None,
        scale: None,
    },
    ColumnMetadata {
        rust_field: "customer_id",
        column_name: "customer_id",
        renamed_from: None,
        sql_type: SqlServerType::BigInt,
        nullable: false,
        primary_key: false,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: true,
        updatable: true,
        max_length: None,
        precision: None,
        scale: None,
    },
    ColumnMetadata {
        rust_field: "total_cents",
        column_name: "total_cents",
        renamed_from: None,
        sql_type: SqlServerType::BigInt,
        nullable: false,
        primary_key: false,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: true,
        updatable: true,
        max_length: None,
        precision: None,
        scale: None,
    },
];

static ORDER_METADATA: EntityMetadata = EntityMetadata {
    rust_name: "Order",
    schema: "sales",
    table: "orders",
    renamed_from: None,
    columns: &ORDER_COLUMNS,
    primary_key: PrimaryKeyMetadata::new(Some("pk_orders"), &["id"]),
    indexes: &[],
    foreign_keys: &[],
    navigations: &[],
};

impl Entity for Order {
    fn metadata() -> &'static EntityMetadata {
        &ORDER_METADATA
    }
}

#[allow(non_upper_case_globals)]
impl Customer {
    const id: EntityColumn<Customer> = EntityColumn::new("id", "id");
    const email: EntityColumn<Customer> = EntityColumn::new("email", "email");
    const active: EntityColumn<Customer> = EntityColumn::new("active", "active");
    const created_at: EntityColumn<Customer> = EntityColumn::new("created_at", "created_at");
}

#[allow(non_upper_case_globals)]
impl Order {
    const customer_id: EntityColumn<Order> = EntityColumn::new("customer_id", "customer_id");
    const total_cents: EntityColumn<Order> = EntityColumn::new("total_cents", "total_cents");
}

struct NewCustomer {
    email: String,
    active: bool,
}

impl Insertable<Customer> for NewCustomer {
    fn values(&self) -> Vec<ColumnValue> {
        vec![
            ColumnValue::new("email", SqlValue::String(self.email.clone())),
            ColumnValue::new("active", SqlValue::Bool(self.active)),
        ]
    }
}

struct UpdateCustomer {
    email: Option<String>,
    active: Option<bool>,
}

impl Changeset<Customer> for UpdateCustomer {
    fn changes(&self) -> Vec<ColumnValue> {
        let mut changes = Vec::new();

        if let Some(email) = &self.email {
            changes.push(ColumnValue::new("email", SqlValue::String(email.clone())));
        }

        if let Some(active) = self.active {
            changes.push(ColumnValue::new("active", SqlValue::Bool(active)));
        }

        changes
    }
}

#[test]
fn snapshots_compiled_select_sql_and_params() {
    let query = SelectQuery::from_entity::<Customer>()
        .select(vec![Expr::from(Customer::id), Expr::from(Customer::email)])
        .filter(Predicate::eq(
            Expr::from(Customer::active),
            Expr::value(SqlValue::Bool(true)),
        ))
        .filter(Predicate::like(
            Expr::from(Customer::email),
            Expr::value(SqlValue::String("%@example.com".to_string())),
        ))
        .order_by(OrderBy::desc(Customer::created_at))
        .paginate(Pagination::page(2, 20));

    let compiled = SqlServerCompiler::compile_select(&query).unwrap();

    assert_snapshot!("compiled_select", render_snapshot(&compiled));
}

#[test]
fn snapshots_compiled_joined_select_sql_and_params() {
    let query = SelectQuery::from_entity::<Customer>()
        .select(vec![
            Expr::from(Customer::email),
            Expr::from(Order::total_cents),
        ])
        .inner_join::<Order>(Predicate::eq(
            Expr::from(Customer::id),
            Expr::from(Order::customer_id),
        ))
        .filter(Predicate::gte(
            Expr::from(Order::total_cents),
            Expr::value(SqlValue::I64(5000)),
        ))
        .order_by(OrderBy::desc(Order::total_cents))
        .paginate(Pagination::page(2, 10));

    let compiled = SqlServerCompiler::compile_select(&query).unwrap();

    assert_snapshot!("compiled_select_with_join", render_snapshot(&compiled));
}

#[test]
fn snapshots_compiled_aliased_joined_select_sql_and_params() {
    let query = SelectQuery::from_entity_as::<Customer>("c")
        .select(vec![
            Expr::column_as(Customer::email, "c"),
            Expr::column_as(Order::total_cents, "created_orders"),
        ])
        .inner_join_as::<Order>(
            "created_orders",
            Predicate::eq(
                Expr::column_as(Customer::id, "c"),
                Expr::column_as(Order::customer_id, "created_orders"),
            ),
        )
        .left_join_as::<Order>(
            "completed_orders",
            Predicate::gte(
                Expr::column_as(Order::total_cents, "completed_orders"),
                Expr::value(SqlValue::I64(5000)),
            ),
        )
        .filter(Predicate::gt(
            Expr::column_as(Order::total_cents, "created_orders"),
            Expr::value(SqlValue::I64(1000)),
        ))
        .order_by(OrderBy::new(
            TableRef::for_entity_as::<Order>("completed_orders"),
            "total_cents",
            SortDirection::Desc,
        ))
        .paginate(Pagination::page(1, 10));

    let compiled = SqlServerCompiler::compile_select(&query).unwrap();

    assert_snapshot!("compiled_select_with_aliases", render_snapshot(&compiled));
}

#[test]
fn snapshots_compiled_insert_sql_and_params() {
    let query = InsertQuery::for_entity::<Customer, _>(&NewCustomer {
        email: "ana@example.com".to_string(),
        active: true,
    });

    let compiled = SqlServerCompiler::compile_insert(&query).unwrap();

    assert_snapshot!("compiled_insert", render_snapshot(&compiled));
}

#[test]
fn snapshots_compiled_update_sql_and_params() {
    let query = UpdateQuery::for_entity::<Customer, _>(&UpdateCustomer {
        email: Some("ana.maria@example.com".to_string()),
        active: Some(false),
    })
    .filter(Predicate::eq(
        Expr::from(Customer::id),
        Expr::value(SqlValue::I64(7)),
    ));

    let compiled = SqlServerCompiler::compile_update(&query).unwrap();

    assert_snapshot!("compiled_update", render_snapshot(&compiled));
}

#[test]
fn snapshots_compiled_delete_sql_and_params() {
    let query = DeleteQuery::from_entity::<Customer>().filter(Predicate::eq(
        Expr::from(Customer::id),
        Expr::value(SqlValue::I64(7)),
    ));

    let compiled = SqlServerCompiler::compile_delete(&query).unwrap();

    assert_snapshot!("compiled_delete", render_snapshot(&compiled));
}

#[test]
fn snapshots_compiled_count_sql_and_params() {
    let query = CountQuery::from_entity::<Customer>().filter(Predicate::eq(
        Expr::from(Customer::active),
        Expr::value(SqlValue::Bool(true)),
    ));

    let compiled = SqlServerCompiler::compile_count(&query).unwrap();

    assert_snapshot!("compiled_count", render_snapshot(&compiled));
}

fn render_snapshot(compiled: &sql_orm_query::CompiledQuery) -> String {
    let params = compiled
        .params
        .iter()
        .enumerate()
        .map(|(index, value)| format!("{}: {}", index + 1, render_sql_value(value)))
        .collect::<Vec<_>>();

    if params.is_empty() {
        format!("SQL: {}\nParams:\n<none>", compiled.sql)
    } else {
        format!("SQL: {}\nParams:\n{}", compiled.sql, params.join("\n"))
    }
}

fn render_sql_value(value: &SqlValue) -> String {
    match value {
        SqlValue::Null => "Null".to_string(),
        SqlValue::TypedNull(sql_type) => format!("TypedNull({sql_type:?})"),
        SqlValue::Bool(value) => format!("Bool({value})"),
        SqlValue::I32(value) => format!("I32({value})"),
        SqlValue::I64(value) => format!("I64({value})"),
        SqlValue::F64(value) => format!("F64({value})"),
        SqlValue::String(value) => format!("String({value:?})"),
        SqlValue::Bytes(value) => format!("Bytes({value:?})"),
        SqlValue::Uuid(value) => format!("Uuid({value})"),
        SqlValue::Decimal(value) => format!("Decimal({value})"),
        SqlValue::Date(value) => format!("Date({value})"),
        SqlValue::DateTime(value) => format!("DateTime({value})"),
    }
}
