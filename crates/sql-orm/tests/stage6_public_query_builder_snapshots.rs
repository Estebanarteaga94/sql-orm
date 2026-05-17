use insta::assert_snapshot;
use sql_orm::prelude::*;
use sql_orm::query::{CompiledQuery, Expr, Predicate};
use sql_orm::sqlserver::SqlServerCompiler;

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "snapshot_users", schema = "dbo")]
struct SnapshotUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 180)]
    email: String,
    active: bool,
}

#[allow(dead_code)]
#[derive(Entity, Debug, Clone)]
#[orm(table = "snapshot_orders", schema = "dbo")]
struct SnapshotOrder {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    user_id: i64,
    total_cents: i64,
}

#[test]
fn public_query_builder_snapshot_preserves_sql_and_parameter_order() {
    let compiled = SqlServerCompiler::compile_select(
        &sql_orm::query::SelectQuery::from_entity::<SnapshotUser>()
            .filter(
                SnapshotUser::active
                    .eq(true)
                    .and(SnapshotUser::email.contains("@example.com")),
            )
            .order_by(SnapshotUser::email.desc())
            .paginate(PageRequest::new(2, 20).to_pagination()),
    )
    .unwrap();

    assert_snapshot!(
        "public_query_builder_compiled_select",
        render_snapshot(&compiled)
    );
}

#[test]
fn public_query_builder_join_snapshot_preserves_sql_and_parameter_order() {
    let compiled = SqlServerCompiler::compile_select(
        &sql_orm::query::SelectQuery::from_entity::<SnapshotUser>()
            .inner_join::<SnapshotOrder>(Predicate::eq(
                Expr::from(SnapshotUser::id),
                Expr::from(SnapshotOrder::user_id),
            ))
            .filter(SnapshotOrder::total_cents.gte(1000_i64))
            .order_by(SnapshotOrder::total_cents.desc())
            .paginate(PageRequest::new(2, 10).to_pagination()),
    )
    .unwrap();

    assert_snapshot!(
        "public_query_builder_compiled_join_select",
        render_snapshot(&compiled)
    );
}

#[test]
fn public_query_builder_alias_snapshot_preserves_sql_and_parameter_order() {
    let compiled = SqlServerCompiler::compile_select(
        &sql_orm::query::SelectQuery::from_entity_as::<SnapshotUser>("u")
            .select([
                SelectProjection::from(SnapshotUser::email.aliased("u")),
                SelectProjection::from(SnapshotOrder::total_cents.aliased("created_orders")),
            ])
            .inner_join_as::<SnapshotOrder>(
                "created_orders",
                Predicate::eq(
                    Expr::from(SnapshotUser::id.aliased("u")),
                    Expr::from(SnapshotOrder::user_id.aliased("created_orders")),
                ),
            )
            .left_join_as::<SnapshotOrder>(
                "completed_orders",
                SnapshotOrder::total_cents
                    .aliased("completed_orders")
                    .gte(5000_i64),
            )
            .filter(
                SnapshotOrder::total_cents
                    .aliased("created_orders")
                    .gte(1000_i64),
            )
            .order_by(
                SnapshotOrder::total_cents
                    .aliased("completed_orders")
                    .desc(),
            )
            .paginate(PageRequest::new(1, 10).to_pagination()),
    )
    .unwrap();

    assert_snapshot!(
        "public_query_builder_compiled_alias_select",
        render_snapshot(&compiled)
    );
}

#[test]
fn public_query_builder_keeps_untrusted_values_out_of_sql_text() {
    let malicious = "'; DROP TABLE dbo.snapshot_users; -- %_[range]";

    let compiled = SqlServerCompiler::compile_select(
        &sql_orm::query::SelectQuery::from_entity::<SnapshotUser>()
            .filter(SnapshotUser::email.contains(malicious))
            .order_by(SnapshotUser::id.asc())
            .paginate(PageRequest::new(1, 5).to_pagination()),
    )
    .unwrap();

    assert!(!compiled.sql.contains(malicious));
    assert_eq!(compiled.params.len(), 3);
    assert_eq!(
        compiled.params[0],
        SqlValue::String(r"%'; DROP TABLE dbo.snapshot\_users; -- \%\_\[range\]%".to_string())
    );
    assert_eq!(compiled.params[1], SqlValue::I64(0));
    assert_eq!(compiled.params[2], SqlValue::I64(5));
}

fn render_snapshot(compiled: &CompiledQuery) -> String {
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
