//! Query AST foundations for the ORM.

mod aggregate;
mod delete;
mod expr;
mod insert;
mod join;
mod order;
mod pagination;
mod predicate;
mod select;
mod update;

use sql_orm_core::{CrateIdentity, SqlValue};

pub use aggregate::{
    AggregateExpr, AggregateOrderBy, AggregatePredicate, AggregateProjection, AggregateQuery,
};
pub use delete::DeleteQuery;
pub use expr::{BinaryOp, ColumnRef, Expr, SqlFunction, TableRef, UnaryOp};
pub use insert::InsertQuery;
pub use join::{Join, JoinType};
pub use order::{OrderBy, SortDirection};
pub use pagination::Pagination;
pub use predicate::Predicate;
pub use select::{CountQuery, ExistsQuery, SelectProjection, SelectQuery};
pub use update::UpdateQuery;

#[derive(Debug, Clone, PartialEq)]
pub struct CompiledQuery {
    pub sql: String,
    pub params: Vec<SqlValue>,
    pub execution: QueryExecution,
}

impl CompiledQuery {
    pub fn new(sql: impl Into<String>, params: Vec<SqlValue>) -> Self {
        Self::with_execution(sql, params, QueryExecution::RawNoRetry)
    }

    pub fn read_only(sql: impl Into<String>, params: Vec<SqlValue>) -> Self {
        Self::with_execution(sql, params, QueryExecution::ReadOnly)
    }

    pub fn write(sql: impl Into<String>, params: Vec<SqlValue>) -> Self {
        Self::with_execution(sql, params, QueryExecution::Write)
    }

    pub fn migration(sql: impl Into<String>, params: Vec<SqlValue>) -> Self {
        Self::with_execution(sql, params, QueryExecution::Migration)
    }

    pub fn raw_no_retry(sql: impl Into<String>, params: Vec<SqlValue>) -> Self {
        Self::with_execution(sql, params, QueryExecution::RawNoRetry)
    }

    pub fn with_execution(
        sql: impl Into<String>,
        params: Vec<SqlValue>,
        execution: QueryExecution,
    ) -> Self {
        Self {
            sql: sql.into(),
            params,
            execution,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum QueryExecution {
    ReadOnly,
    Write,
    Migration,
    RawNoRetry,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Query {
    Select(SelectQuery),
    Aggregate(Box<AggregateQuery>),
    Exists(Box<ExistsQuery>),
    Insert(InsertQuery),
    Update(UpdateQuery),
    Delete(DeleteQuery),
    Count(CountQuery),
}

pub const CRATE_IDENTITY: CrateIdentity = CrateIdentity {
    name: "sql-orm-query",
    responsibility: "typed AST and query builder primitives without SQL generation",
};

#[cfg(test)]
mod tests {
    use super::{
        AggregateExpr, AggregateOrderBy, AggregatePredicate, AggregateProjection, AggregateQuery,
        BinaryOp, CRATE_IDENTITY, ColumnRef, CompiledQuery, CountQuery, DeleteQuery, ExistsQuery,
        Expr, InsertQuery, Join, JoinType, OrderBy, Pagination, Predicate, Query, SelectProjection,
        SelectQuery, SortDirection, SqlFunction, TableRef, UpdateQuery,
    };
    use sql_orm_core::{
        Changeset, ColumnMetadata, ColumnValue, Entity, EntityColumn, EntityMetadata,
        IdentityMetadata, Insertable, PrimaryKeyMetadata, SqlServerType, SqlValue,
    };

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
    }

    impl Changeset<Customer> for UpdateCustomer {
        fn changes(&self) -> Vec<ColumnValue> {
            self.email
                .clone()
                .map(|email| vec![ColumnValue::new("email", SqlValue::String(email))])
                .unwrap_or_default()
        }
    }

    #[test]
    fn keeps_query_layer_sql_free() {
        assert!(
            CRATE_IDENTITY
                .responsibility
                .contains("without SQL generation")
        );
    }

    #[test]
    fn entity_columns_become_table_aware_column_refs() {
        let column = ColumnRef::for_entity_column(Customer::email);

        assert_eq!(column.table, TableRef::new("sales", "customers"));
        assert_eq!(column.rust_field, "email");
        assert_eq!(column.column_name, "email");
    }

    #[test]
    fn expr_supports_columns_values_functions_and_operations() {
        let expr = Expr::binary(
            Expr::function(SqlFunction::Lower, vec![Expr::from(Customer::email)]),
            BinaryOp::Add,
            Expr::value(SqlValue::String("@example.com".to_string())),
        );

        match expr {
            Expr::Binary { left, op, right } => {
                assert_eq!(op, BinaryOp::Add);
                assert!(matches!(*left, Expr::Function { .. }));
                assert_eq!(
                    *right,
                    Expr::Value(SqlValue::String("@example.com".to_string()))
                );
            }
            other => panic!("unexpected expr shape: {other:?}"),
        }
    }

    #[test]
    fn predicates_can_be_composed_without_sql_rendering() {
        let predicate = Predicate::and(vec![
            Predicate::eq(
                Expr::from(Customer::active),
                Expr::value(SqlValue::Bool(true)),
            ),
            Predicate::like(
                Expr::from(Customer::email),
                Expr::value(SqlValue::String("%@example.com".to_string())),
            ),
            Predicate::like_escaped(
                Expr::from(Customer::email),
                Expr::value(SqlValue::String(r"%literal\%%".to_string())),
                '\\',
            ),
        ]);

        match predicate {
            Predicate::And(parts) => assert_eq!(parts.len(), 3),
            other => panic!("unexpected predicate shape: {other:?}"),
        }
    }

    #[test]
    fn select_query_captures_projection_filters_order_and_pagination() {
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

        assert_eq!(query.from, TableRef::new("sales", "customers"));
        assert!(query.joins.is_empty());
        assert_eq!(
            query.projection,
            vec![
                SelectProjection::column(Customer::id),
                SelectProjection::column(Customer::email)
            ]
        );
        assert_eq!(
            query.order_by,
            vec![OrderBy::new(
                TableRef::new("sales", "customers"),
                "created_at",
                SortDirection::Desc,
            )]
        );
        assert_eq!(query.pagination, Some(Pagination::new(20, 20)));
        assert!(matches!(query.predicate, Some(Predicate::And(_))));
    }

    #[test]
    fn select_query_captures_explicit_joins_without_sql_rendering() {
        let query = SelectQuery::from_entity::<Customer>()
            .inner_join::<Order>(Predicate::eq(
                Expr::from(Customer::id),
                Expr::from(Order::customer_id),
            ))
            .join(Join::left(
                TableRef::new("sales", "orders"),
                Predicate::gt(
                    Expr::from(Order::total_cents),
                    Expr::value(SqlValue::I64(0)),
                ),
            ));

        assert_eq!(query.joins.len(), 2);
        assert_eq!(query.joins[0].join_type, JoinType::Inner);
        assert_eq!(query.joins[0].table, TableRef::new("sales", "orders"));
        assert!(matches!(query.joins[0].on, Predicate::Eq(_, _)));
        assert_eq!(query.joins[1].join_type, JoinType::Left);
        assert_eq!(query.joins[1].table, TableRef::new("sales", "orders"));
        assert!(matches!(query.joins[1].on, Predicate::Gt(_, _)));
    }

    #[test]
    fn table_refs_capture_optional_aliases_without_sql_rendering() {
        let table = TableRef::for_entity_as::<Customer>("root");
        let column = ColumnRef::for_entity_column_as(Customer::email, "root");
        let expr = Expr::column_as(Customer::id, "root");

        assert_eq!(table.schema, "sales");
        assert_eq!(table.table, "customers");
        assert_eq!(table.alias, Some("root"));
        assert_eq!(table.reference_name(), "root");
        assert_eq!(table.without_alias(), TableRef::new("sales", "customers"));
        assert_eq!(column.table, table);

        match expr {
            Expr::Column(column) => {
                assert_eq!(column.table.alias, Some("root"));
                assert_eq!(column.column_name, "id");
            }
            other => panic!("unexpected expr shape: {other:?}"),
        }
    }

    #[test]
    fn select_query_captures_aliased_sources_and_repeated_joins() {
        let query = SelectQuery::from_entity_as::<Customer>("c")
            .inner_join_as::<Order>(
                "created_orders",
                Predicate::eq(
                    Expr::column_as(Customer::id, "c"),
                    Expr::column_as(Order::customer_id, "created_orders"),
                ),
            )
            .left_join_as::<Order>(
                "completed_orders",
                Predicate::gt(
                    Expr::column_as(Order::total_cents, "completed_orders"),
                    Expr::value(SqlValue::I64(0)),
                ),
            );

        assert_eq!(query.from, TableRef::with_alias("sales", "customers", "c"));
        assert_eq!(query.joins.len(), 2);
        assert_eq!(
            query.joins[0].table,
            TableRef::with_alias("sales", "orders", "created_orders")
        );
        assert_eq!(
            query.joins[1].table,
            TableRef::with_alias("sales", "orders", "completed_orders")
        );
        assert_ne!(query.joins[0].table, query.joins[1].table);
    }

    #[test]
    fn select_projection_captures_default_and_explicit_aliases() {
        let column_projection = SelectProjection::column(Customer::email);
        assert_eq!(column_projection.alias.as_deref(), Some("email"));
        assert_eq!(column_projection.expr, Expr::from(Customer::email));

        let expression_projection = SelectProjection::expr_as(
            Expr::function(SqlFunction::Lower, vec![Expr::from(Customer::email)]),
            "email_lower",
        );
        assert_eq!(expression_projection.alias.as_deref(), Some("email_lower"));

        let owned_alias_projection =
            SelectProjection::expr_as(Expr::from(Customer::email), "owned_email".to_string());
        assert_eq!(owned_alias_projection.alias.as_deref(), Some("owned_email"));

        let unaliased_expression = SelectProjection::expr(Expr::function(
            SqlFunction::Lower,
            vec![Expr::from(Customer::email)],
        ));
        assert_eq!(unaliased_expression.alias.as_deref(), None);
    }

    #[test]
    fn aggregate_projection_requires_alias_without_changing_select_projection() {
        let group_key = AggregateProjection::group_key(Order::customer_id);
        let expression_group_key = AggregateProjection::group_key_as(
            Expr::function(SqlFunction::Year, vec![Expr::from(Customer::created_at)]),
            "created_year",
        );
        let aggregate = AggregateProjection::sum_as(Order::total_cents, "total_cents");

        assert_eq!(group_key.alias, "customer_id");
        assert_eq!(expression_group_key.alias, "created_year");
        assert_eq!(aggregate.alias, "total_cents");
        assert_eq!(
            aggregate,
            AggregateProjection::expr_as(
                AggregateExpr::Sum(Expr::from(Order::total_cents)),
                "total_cents"
            )
        );

        let ordinary_projection = SelectProjection::expr(Expr::function(
            SqlFunction::Lower,
            vec![Expr::from(Customer::email)],
        ));
        assert_eq!(ordinary_projection.alias.as_deref(), None);
    }

    #[test]
    fn aggregate_query_captures_grouping_having_and_projection_without_sql_rendering() {
        let query = AggregateQuery::from_entity::<Order>()
            .inner_join::<Customer>(Predicate::eq(
                Expr::from(Order::customer_id),
                Expr::from(Customer::id),
            ))
            .filter(Predicate::eq(
                Expr::from(Customer::active),
                Expr::value(SqlValue::Bool(true)),
            ))
            .group_by(vec![Expr::from(Order::customer_id)])
            .project(vec![
                AggregateProjection::group_key(Order::customer_id),
                AggregateProjection::count_as("order_count"),
                AggregateProjection::sum_as(Order::total_cents, "total_cents"),
            ])
            .having(AggregatePredicate::gt(
                AggregateExpr::count_all(),
                Expr::value(SqlValue::I64(1)),
            ))
            .order_by(AggregateOrderBy::desc(AggregateExpr::sum(Expr::from(
                Order::total_cents,
            ))))
            .paginate(Pagination::page(1, 10));

        assert_eq!(query.from, TableRef::new("sales", "orders"));
        assert_eq!(query.joins.len(), 1);
        assert!(query.predicate.is_some());
        assert_eq!(query.group_by, vec![Expr::from(Order::customer_id)]);
        assert_eq!(
            query.projection,
            vec![
                AggregateProjection::group_key(Order::customer_id),
                AggregateProjection::count_as("order_count"),
                AggregateProjection::sum_as(Order::total_cents, "total_cents")
            ]
        );
        assert!(matches!(query.having, Some(AggregatePredicate::Gt(_, _))));
        assert_eq!(
            query.order_by,
            vec![AggregateOrderBy::desc(AggregateExpr::sum(Expr::from(
                Order::total_cents
            )))]
        );
        assert_eq!(query.pagination, Some(Pagination::new(0, 10)));
        assert!(matches!(
            Query::Aggregate(Box::new(query)),
            Query::Aggregate(_)
        ));
    }

    #[test]
    fn insert_update_delete_and_count_queries_capture_operation_data() {
        let insert = InsertQuery::for_entity::<Customer, _>(&NewCustomer {
            email: "ana@example.com".to_string(),
            active: true,
        });
        let update = UpdateQuery::for_entity::<Customer, _>(&UpdateCustomer {
            email: Some("ana.maria@example.com".to_string()),
        })
        .filter(Predicate::eq(
            Expr::from(Customer::id),
            Expr::value(SqlValue::I64(7)),
        ));
        let delete = DeleteQuery::from_entity::<Customer>().filter(Predicate::eq(
            Expr::from(Customer::id),
            Expr::value(SqlValue::I64(7)),
        ));
        let count = CountQuery::from_entity::<Customer>().filter(Predicate::eq(
            Expr::from(Customer::active),
            Expr::value(SqlValue::Bool(true)),
        ));
        let exists = ExistsQuery::from_entity::<Customer>()
            .inner_join::<Order>(Predicate::eq(
                Expr::from(Customer::id),
                Expr::from(Order::customer_id),
            ))
            .filter(Predicate::eq(
                Expr::from(Customer::active),
                Expr::value(SqlValue::Bool(true)),
            ));

        assert_eq!(insert.into, TableRef::new("sales", "customers"));
        assert_eq!(insert.values.len(), 2);
        assert_eq!(insert.entity, Some(Customer::metadata()));
        assert_eq!(update.table, TableRef::new("sales", "customers"));
        assert_eq!(update.changes.len(), 1);
        assert!(update.predicate.is_some());
        assert!(!update.allow_all_rows);
        assert_eq!(update.entity, Some(Customer::metadata()));
        assert!(
            UpdateQuery::for_entity::<Customer, _>(&UpdateCustomer {
                email: Some("ana.maria@example.com".to_string()),
            })
            .allow_all_rows()
            .allow_all_rows
        );
        assert_eq!(delete.from, TableRef::new("sales", "customers"));
        assert!(delete.predicate.is_some());
        assert!(!delete.allow_all_rows);
        assert!(
            DeleteQuery::from_entity::<Customer>()
                .allow_all_rows()
                .allow_all_rows
        );
        assert_eq!(count.from, TableRef::new("sales", "customers"));
        assert!(count.predicate.is_some());
        assert_eq!(exists.from, TableRef::new("sales", "customers"));
        assert_eq!(exists.joins.len(), 1);
        assert!(exists.predicate.is_some());

        assert!(matches!(Query::Insert(insert.clone()), Query::Insert(_)));
        assert!(matches!(Query::Update(update.clone()), Query::Update(_)));
        assert!(matches!(Query::Delete(delete.clone()), Query::Delete(_)));
        assert!(matches!(Query::Count(count.clone()), Query::Count(_)));
        assert!(matches!(Query::Exists(Box::new(exists)), Query::Exists(_)));
    }

    #[test]
    fn compiled_query_keeps_sql_and_parameter_order() {
        let compiled = CompiledQuery::new(
            "SELECT [id] FROM [sales].[customers] WHERE [active] = @P1 AND [email] LIKE @P2",
            vec![
                SqlValue::Bool(true),
                SqlValue::String("%@example.com".to_string()),
            ],
        );

        assert_eq!(
            compiled.sql,
            "SELECT [id] FROM [sales].[customers] WHERE [active] = @P1 AND [email] LIKE @P2"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::Bool(true),
                SqlValue::String("%@example.com".to_string()),
            ]
        );
    }
}
