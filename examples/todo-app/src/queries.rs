use crate::db::TodoAppDbContext;
use crate::domain::{TodoItem, TodoList, User};
use sql_orm::prelude::*;
use sql_orm::query::{Expr, Predicate};

#[cfg(test)]
use sql_orm::query::SelectQuery;

pub fn user_lists_page_query(
    db: &TodoAppDbContext,
    owner_user_id: i64,
    page: PageRequest,
) -> DbSetQuery<TodoList> {
    db.todo_lists
        .query()
        .filter(
            TodoList::owner_user_id
                .eq(owner_user_id)
                .and(TodoList::is_archived.eq(false)),
        )
        .order_by(TodoList::title.asc())
        .paginate(page)
}

pub fn list_items_page_query(
    db: &TodoAppDbContext,
    owner_user_id: i64,
    list_id: i64,
    page: PageRequest,
) -> DbSetQuery<TodoItem> {
    db.todo_items
        .query()
        .inner_join::<TodoList>(Predicate::eq(
            Expr::from(TodoItem::list_id),
            Expr::from(TodoList::id),
        ))
        .left_join::<User>(Predicate::eq(
            Expr::from(TodoItem::completed_by_user_id),
            Expr::from(User::id),
        ))
        .filter(
            TodoList::owner_user_id
                .eq(owner_user_id)
                .and(TodoItem::list_id.eq(list_id)),
        )
        .order_by(TodoItem::position.asc())
        .paginate(page)
}

pub fn open_items_preview_query(
    db: &TodoAppDbContext,
    list_id: i64,
    limit: u64,
) -> DbSetQuery<TodoItem> {
    db.todo_items
        .query()
        .filter(
            TodoItem::list_id
                .eq(list_id)
                .and(TodoItem::is_completed.eq(false)),
        )
        .order_by(TodoItem::position.asc())
        .take(limit)
}

pub fn open_items_count_query(db: &TodoAppDbContext, list_id: i64) -> DbSetQuery<TodoItem> {
    db.todo_items.query().filter(
        TodoItem::list_id
            .eq(list_id)
            .and(TodoItem::is_completed.eq(false)),
    )
}

#[cfg(test)]
fn user_lists_select_query(owner_user_id: i64) -> SelectQuery {
    SelectQuery::from_entity::<TodoList>()
        .filter(
            TodoList::owner_user_id
                .eq(owner_user_id)
                .and(TodoList::is_archived.eq(false)),
        )
        .order_by(TodoList::title.asc())
}

#[cfg(test)]
fn list_items_select_query(owner_user_id: i64, list_id: i64) -> SelectQuery {
    SelectQuery::from_entity::<TodoItem>()
        .inner_join::<TodoList>(Predicate::eq(
            Expr::from(TodoItem::list_id),
            Expr::from(TodoList::id),
        ))
        .left_join::<User>(Predicate::eq(
            Expr::from(TodoItem::completed_by_user_id),
            Expr::from(User::id),
        ))
        .filter(
            TodoList::owner_user_id
                .eq(owner_user_id)
                .and(TodoItem::list_id.eq(list_id)),
        )
        .order_by(TodoItem::position.asc())
}

#[cfg(test)]
fn open_items_select_query(list_id: i64) -> SelectQuery {
    SelectQuery::from_entity::<TodoItem>()
        .filter(
            TodoItem::list_id
                .eq(list_id)
                .and(TodoItem::is_completed.eq(false)),
        )
        .order_by(TodoItem::position.asc())
}

#[cfg(test)]
use sql_orm::query::CountQuery;

#[cfg(test)]
fn open_items_count_ast(list_id: i64) -> CountQuery {
    CountQuery::from_entity::<TodoItem>().filter(
        TodoItem::list_id
            .eq(list_id)
            .and(TodoItem::is_completed.eq(false)),
    )
}

#[cfg(test)]
mod tests {
    use super::{
        list_items_select_query, open_items_count_ast, open_items_select_query,
        user_lists_select_query,
    };
    use crate::domain::{TodoItem, TodoList};
    use sql_orm::prelude::*;
    use sql_orm::query::{
        Expr, JoinType, OrderBy, Pagination, Predicate, SelectQuery, SortDirection, TableRef,
    };
    use sql_orm::sqlserver::SqlServerCompiler;

    #[test]
    fn user_lists_select_query_builds_expected_ast() {
        let query = user_lists_select_query(7).paginate(PageRequest::new(2, 10).to_pagination());

        assert_eq!(
            query,
            SelectQuery::from_entity::<TodoList>()
                .filter(Predicate::and(vec![
                    Predicate::eq(
                        Expr::from(TodoList::owner_user_id),
                        Expr::value(SqlValue::I64(7))
                    ),
                    Predicate::eq(
                        Expr::from(TodoList::is_archived),
                        Expr::value(SqlValue::Bool(false))
                    ),
                ]))
                .order_by(OrderBy::new(
                    TableRef::new("todo", "todo_lists"),
                    "title",
                    SortDirection::Asc,
                ))
                .paginate(Pagination::new(10, 10))
        );
    }

    #[test]
    fn list_items_select_query_builds_expected_ast_with_joins() {
        let query =
            list_items_select_query(42, 9).paginate(PageRequest::new(3, 25).to_pagination());

        assert_eq!(query.joins.len(), 2);
        assert_eq!(query.joins[0].join_type, JoinType::Inner);
        assert_eq!(query.joins[0].table, TableRef::new("todo", "todo_lists"));
        assert_eq!(query.joins[1].join_type, JoinType::Left);
        assert_eq!(query.joins[1].table, TableRef::new("todo", "users"));
        assert_eq!(
            query.predicate,
            Some(Predicate::and(vec![
                Predicate::eq(
                    Expr::from(TodoList::owner_user_id),
                    Expr::value(SqlValue::I64(42))
                ),
                Predicate::eq(Expr::from(TodoItem::list_id), Expr::value(SqlValue::I64(9))),
            ]))
        );
        assert_eq!(
            query.order_by,
            vec![OrderBy::new(
                TableRef::new("todo", "todo_items"),
                "position",
                SortDirection::Asc,
            )]
        );
        assert_eq!(query.pagination, Some(Pagination::new(50, 25)));
    }

    #[test]
    fn open_items_select_query_supports_preview_and_count_shapes() {
        let preview = open_items_select_query(5).paginate(sql_orm::query::Pagination::new(0, 3));
        let count = open_items_select_query(11);

        assert_eq!(
            preview,
            SelectQuery::from_entity::<TodoItem>()
                .filter(Predicate::and(vec![
                    Predicate::eq(Expr::from(TodoItem::list_id), Expr::value(SqlValue::I64(5))),
                    Predicate::eq(
                        Expr::from(TodoItem::is_completed),
                        Expr::value(SqlValue::Bool(false)),
                    ),
                ]))
                .order_by(OrderBy::new(
                    TableRef::new("todo", "todo_items"),
                    "position",
                    SortDirection::Asc,
                ))
                .paginate(Pagination::new(0, 3))
        );

        assert_eq!(
            count,
            SelectQuery::from_entity::<TodoItem>()
                .filter(Predicate::and(vec![
                    Predicate::eq(
                        Expr::from(TodoItem::list_id),
                        Expr::value(SqlValue::I64(11))
                    ),
                    Predicate::eq(
                        Expr::from(TodoItem::is_completed),
                        Expr::value(SqlValue::Bool(false)),
                    ),
                ]))
                .order_by(OrderBy::new(
                    TableRef::new("todo", "todo_items"),
                    "position",
                    SortDirection::Asc,
                ))
        );
    }

    #[test]
    fn list_items_select_query_compiles_expected_sql_server_select() {
        let compiled = SqlServerCompiler::compile_select(
            &list_items_select_query(42, 9).paginate(PageRequest::new(2, 5).to_pagination()),
        )
        .expect("todo_app joined select compilation");

        assert_eq!(
            compiled.sql,
            "SELECT * FROM [todo].[todo_items] INNER JOIN [todo].[todo_lists] ON ([todo].[todo_items].[list_id] = [todo].[todo_lists].[id]) LEFT JOIN [todo].[users] ON ([todo].[todo_items].[completed_by_user_id] = [todo].[users].[id]) WHERE (([todo].[todo_lists].[owner_user_id] = @P1) AND ([todo].[todo_items].[list_id] = @P2)) ORDER BY [todo].[todo_items].[position] ASC OFFSET @P3 ROWS FETCH NEXT @P4 ROWS ONLY"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::I64(42),
                SqlValue::I64(9),
                SqlValue::I64(5),
                SqlValue::I64(5),
            ]
        );
    }

    #[test]
    fn open_items_select_query_compiles_expected_sql_server_count() {
        let compiled = SqlServerCompiler::compile_count(&open_items_count_ast(9))
            .expect("todo_app count compilation");

        assert_eq!(
            compiled.sql,
            "SELECT COUNT(*) AS [count] FROM [todo].[todo_items] WHERE (([todo].[todo_items].[list_id] = @P1) AND ([todo].[todo_items].[is_completed] = @P2))"
        );
        assert_eq!(
            compiled.params,
            vec![SqlValue::I64(9), SqlValue::Bool(false)]
        );
    }
}
