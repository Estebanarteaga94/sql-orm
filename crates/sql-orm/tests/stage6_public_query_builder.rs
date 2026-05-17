use sql_orm::prelude::*;
use sql_orm::query::{
    Expr, JoinType, OrderBy, Pagination, Predicate, SelectQuery, SortDirection, TableRef,
};

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "query_builder_users", schema = "dbo")]
struct QueryUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 180)]
    email: String,
    active: bool,
    #[orm(has_many(QueryOrder, foreign_key = user_id))]
    orders: Collection<QueryOrder>,
}

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "query_builder_orders", schema = "dbo")]
struct QueryOrder {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(foreign_key(entity = QueryUser, column = id))]
    user_id: i64,
    total_cents: i64,
    #[orm(belongs_to(QueryUser, foreign_key = user_id))]
    user: Navigation<QueryUser>,
}

#[test]
fn public_query_builder_extensions_produce_expected_ast() {
    let query = SelectQuery::from_entity::<QueryUser>()
        .filter(
            QueryUser::active
                .eq(true)
                .and(QueryUser::email.contains("@example.com")),
        )
        .order_by(QueryUser::email.desc())
        .paginate(PageRequest::new(2, 20).to_pagination());

    assert_eq!(
        query,
        SelectQuery::from_entity::<QueryUser>()
            .filter(Predicate::and(vec![
                Predicate::eq(
                    Expr::from(QueryUser::active),
                    Expr::value(SqlValue::Bool(true)),
                ),
                Predicate::like_escaped(
                    Expr::from(QueryUser::email),
                    Expr::value(SqlValue::String("%@example.com%".to_string())),
                    '\\',
                ),
            ]))
            .order_by(OrderBy::new(
                TableRef::new("dbo", "query_builder_users"),
                "email",
                SortDirection::Desc,
            ))
            .paginate(Pagination::new(20, 20))
    );
}

#[test]
fn public_predicate_composition_flattens_logical_groups() {
    let predicate = QueryUser::active
        .eq(true)
        .and(QueryUser::email.contains("@example.com"))
        .and(QueryUser::email.is_not_null())
        .or(QueryUser::id.gt(10_i64));

    assert_eq!(
        predicate,
        Predicate::or(vec![
            Predicate::and(vec![
                Predicate::eq(
                    Expr::from(QueryUser::active),
                    Expr::value(SqlValue::Bool(true)),
                ),
                Predicate::like_escaped(
                    Expr::from(QueryUser::email),
                    Expr::value(SqlValue::String("%@example.com%".to_string())),
                    '\\',
                ),
                Predicate::is_not_null(Expr::from(QueryUser::email)),
            ]),
            Predicate::gt(Expr::from(QueryUser::id), Expr::value(SqlValue::I64(10))),
        ])
    );
}

#[test]
fn public_dbset_query_exposes_join_helpers() {
    let query = SelectQuery::from_entity::<QueryUser>()
        .inner_join::<QueryOrder>(Predicate::eq(
            Expr::from(QueryUser::id),
            Expr::from(QueryOrder::user_id),
        ))
        .left_join::<QueryOrder>(QueryOrder::total_cents.gt(0_i64))
        .order_by(QueryOrder::total_cents.desc());

    assert_eq!(query.joins.len(), 2);
    assert_eq!(query.joins[0].join_type, JoinType::Inner);
    assert_eq!(
        query.joins[0].table,
        TableRef::new("dbo", "query_builder_orders")
    );
    assert_eq!(query.joins[1].join_type, JoinType::Left);
    assert_eq!(
        query.joins[1].table,
        TableRef::new("dbo", "query_builder_orders")
    );
    assert_eq!(
        query.order_by,
        vec![OrderBy::new(
            TableRef::new("dbo", "query_builder_orders"),
            "total_cents",
            SortDirection::Desc,
        )]
    );
}
