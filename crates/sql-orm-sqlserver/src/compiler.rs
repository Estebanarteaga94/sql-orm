use crate::quoting::{
    quote_column_ref, quote_identifier, quote_table_ref, quote_table_reference, quote_table_source,
};
use sql_orm_core::{ColumnValue, OrmError, SqlValue};
use sql_orm_query::{
    BinaryOp, CompiledQuery, CountQuery, DeleteQuery, Expr, InsertQuery, Join, JoinType, OrderBy,
    Pagination, Predicate, Query, SelectProjection, SelectQuery, SortDirection, TableRef, UnaryOp,
    UpdateQuery,
};
use std::collections::BTreeSet;

#[derive(Debug, Default)]
struct ParameterBuilder {
    params: Vec<SqlValue>,
}

impl ParameterBuilder {
    fn push(&mut self, value: SqlValue) -> String {
        self.params.push(value);
        format!("@P{}", self.params.len())
    }

    fn finish(self, sql: String) -> CompiledQuery {
        CompiledQuery::new(sql, self.params)
    }
}

impl crate::SqlServerCompiler {
    pub fn compile_query(query: &Query) -> Result<CompiledQuery, OrmError> {
        match query {
            Query::Select(query) => Self::compile_select(query),
            Query::Insert(query) => Self::compile_insert(query),
            Query::Update(query) => Self::compile_update(query),
            Query::Delete(query) => Self::compile_delete(query),
            Query::Count(query) => Self::compile_count(query),
        }
    }

    pub fn compile_select(query: &SelectQuery) -> Result<CompiledQuery, OrmError> {
        let mut parameters = ParameterBuilder::default();
        let projection = compile_projection(&query.projection, &mut parameters)?;
        let mut sql = format!(
            "SELECT {projection} FROM {}",
            quote_table_source(&query.from)?
        );
        sql.push_str(&compile_joins(&query.from, &query.joins, &mut parameters)?);

        if let Some(predicate) = &query.predicate {
            let predicate = compile_predicate(predicate, &mut parameters)?;
            sql.push_str(" WHERE ");
            sql.push_str(&predicate);
        }

        if !query.order_by.is_empty() {
            sql.push_str(" ORDER BY ");
            sql.push_str(&compile_order_by(&query.order_by)?);
        }

        if let Some(pagination) = query.pagination {
            if query.order_by.is_empty() {
                return Err(OrmError::new(
                    "SQL Server pagination requires ORDER BY before OFFSET/FETCH",
                ));
            }

            sql.push(' ');
            sql.push_str(&compile_pagination(pagination, &mut parameters));
        }

        Ok(parameters.finish(sql))
    }

    pub fn compile_insert(query: &InsertQuery) -> Result<CompiledQuery, OrmError> {
        if query.values.is_empty() {
            return Err(OrmError::new(
                "SQL Server insert compilation requires at least one value",
            ));
        }

        let mut parameters = ParameterBuilder::default();
        let (columns, values) = compile_column_values(&query.values, &mut parameters)?;
        let sql = format!(
            "INSERT INTO {} ({columns}) OUTPUT INSERTED.* VALUES ({values})",
            quote_table_ref(&query.into)?,
        );

        Ok(parameters.finish(sql))
    }

    pub fn compile_update(query: &UpdateQuery) -> Result<CompiledQuery, OrmError> {
        if query.changes.is_empty() {
            return Err(OrmError::new(
                "SQL Server update compilation requires at least one change",
            ));
        }

        let mut parameters = ParameterBuilder::default();
        let assignments = compile_assignments(&query.changes, &mut parameters)?;
        let mut sql = format!(
            "UPDATE {} SET {assignments} OUTPUT INSERTED.*",
            quote_table_ref(&query.table)?,
        );

        if let Some(predicate) = &query.predicate {
            let predicate = compile_predicate(predicate, &mut parameters)?;
            sql.push_str(" WHERE ");
            sql.push_str(&predicate);
        }

        Ok(parameters.finish(sql))
    }

    pub fn compile_delete(query: &DeleteQuery) -> Result<CompiledQuery, OrmError> {
        let mut parameters = ParameterBuilder::default();
        let mut sql = format!("DELETE FROM {}", quote_table_ref(&query.from)?);

        if let Some(predicate) = &query.predicate {
            let predicate = compile_predicate(predicate, &mut parameters)?;
            sql.push_str(" WHERE ");
            sql.push_str(&predicate);
        }

        Ok(parameters.finish(sql))
    }

    pub fn compile_count(query: &CountQuery) -> Result<CompiledQuery, OrmError> {
        let mut parameters = ParameterBuilder::default();
        let mut sql = format!(
            "SELECT COUNT(*) AS {} FROM {}",
            quote_identifier("count")?,
            quote_table_source(&query.from)?,
        );

        if let Some(predicate) = &query.predicate {
            let predicate = compile_predicate(predicate, &mut parameters)?;
            sql.push_str(" WHERE ");
            sql.push_str(&predicate);
        }

        Ok(parameters.finish(sql))
    }
}

fn compile_joins(
    from: &TableRef,
    joins: &[Join],
    parameters: &mut ParameterBuilder,
) -> Result<String, OrmError> {
    let mut compiled = String::new();
    let mut seen_tables = vec![*from];

    for join in joins {
        if seen_tables.contains(&join.table) {
            return Err(OrmError::new(
                "SQL Server join compilation requires aliases for repeated table sources",
            ));
        }

        seen_tables.push(join.table);
        compiled.push(' ');
        compiled.push_str(match join.join_type {
            JoinType::Inner => "INNER JOIN ",
            JoinType::Left => "LEFT JOIN ",
        });
        compiled.push_str(&quote_table_source(&join.table)?);
        compiled.push_str(" ON ");
        compiled.push_str(&compile_predicate(&join.on, parameters)?);
    }

    Ok(compiled)
}

fn compile_projection(
    projection: &[SelectProjection],
    parameters: &mut ParameterBuilder,
) -> Result<String, OrmError> {
    if projection.is_empty() {
        return Ok("*".to_string());
    }

    let mut aliases = BTreeSet::new();
    let parts = projection
        .iter()
        .map(|projection| {
            let alias = projection.alias.ok_or_else(|| {
                OrmError::new("SQL Server projection expressions require an explicit alias")
            })?;
            if alias.trim().is_empty() {
                return Err(OrmError::new("SQL Server projection alias cannot be empty"));
            }
            if !aliases.insert(alias) {
                return Err(OrmError::new(format!(
                    "SQL Server projection alias `{alias}` is duplicated"
                )));
            }

            Ok(format!(
                "{} AS {}",
                compile_expr(&projection.expr, parameters)?,
                quote_identifier(alias)?
            ))
        })
        .collect::<Result<Vec<_>, _>>()?;
    Ok(parts.join(", "))
}

fn compile_expr(expr: &Expr, parameters: &mut ParameterBuilder) -> Result<String, OrmError> {
    match expr {
        Expr::Column(column) => quote_column_ref(column),
        Expr::Value(value) => Ok(parameters.push(value.clone())),
        Expr::Binary { left, op, right } => Ok(format!(
            "({} {} {})",
            compile_expr(left, parameters)?,
            compile_binary_op(*op),
            compile_expr(right, parameters)?,
        )),
        Expr::Unary { op, expr } => Ok(format!(
            "({} {})",
            compile_unary_op(*op),
            compile_expr(expr, parameters)?,
        )),
        Expr::Function { name, args } => {
            if name.trim().is_empty() {
                return Err(OrmError::new("SQL function name cannot be empty"));
            }

            let args = args
                .iter()
                .map(|arg| compile_expr(arg, parameters))
                .collect::<Result<Vec<_>, _>>()?;

            Ok(format!("{name}({})", args.join(", ")))
        }
    }
}

fn compile_predicate(
    predicate: &Predicate,
    parameters: &mut ParameterBuilder,
) -> Result<String, OrmError> {
    match predicate {
        Predicate::Eq(left, right) => compile_comparison(left, "=", right, parameters),
        Predicate::Ne(left, right) => compile_comparison(left, "<>", right, parameters),
        Predicate::Gt(left, right) => compile_comparison(left, ">", right, parameters),
        Predicate::Gte(left, right) => compile_comparison(left, ">=", right, parameters),
        Predicate::Lt(left, right) => compile_comparison(left, "<", right, parameters),
        Predicate::Lte(left, right) => compile_comparison(left, "<=", right, parameters),
        Predicate::Like(left, right) => compile_comparison(left, "LIKE", right, parameters),
        Predicate::IsNull(expr) => Ok(format!("({} IS NULL)", compile_expr(expr, parameters)?)),
        Predicate::IsNotNull(expr) => {
            Ok(format!("({} IS NOT NULL)", compile_expr(expr, parameters)?))
        }
        Predicate::And(predicates) => compile_logical("AND", predicates, parameters),
        Predicate::Or(predicates) => compile_logical("OR", predicates, parameters),
        Predicate::Not(predicate) => Ok(format!(
            "(NOT {})",
            compile_predicate(predicate, parameters)?
        )),
    }
}

fn compile_comparison(
    left: &Expr,
    operator: &str,
    right: &Expr,
    parameters: &mut ParameterBuilder,
) -> Result<String, OrmError> {
    Ok(format!(
        "({} {operator} {})",
        compile_expr(left, parameters)?,
        compile_expr(right, parameters)?,
    ))
}

fn compile_logical(
    operator: &str,
    predicates: &[Predicate],
    parameters: &mut ParameterBuilder,
) -> Result<String, OrmError> {
    if predicates.is_empty() {
        return Err(OrmError::new(
            "logical predicate compilation requires at least one child predicate",
        ));
    }

    let compiled = predicates
        .iter()
        .map(|predicate| compile_predicate(predicate, parameters))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(format!("({})", compiled.join(&format!(" {operator} "))))
}

fn compile_order_by(order_by: &[OrderBy]) -> Result<String, OrmError> {
    let parts = order_by
        .iter()
        .map(|order| {
            Ok(format!(
                "{}.{} {}",
                quote_table_reference(&order.table)?,
                quote_identifier(order.column_name)?,
                match order.direction {
                    SortDirection::Asc => "ASC",
                    SortDirection::Desc => "DESC",
                },
            ))
        })
        .collect::<Result<Vec<_>, OrmError>>()?;

    Ok(parts.join(", "))
}

fn compile_pagination(pagination: Pagination, parameters: &mut ParameterBuilder) -> String {
    let offset = parameters.push(SqlValue::I64(pagination.offset as i64));
    let limit = parameters.push(SqlValue::I64(pagination.limit as i64));

    format!("OFFSET {offset} ROWS FETCH NEXT {limit} ROWS ONLY")
}

fn compile_column_values(
    values: &[ColumnValue],
    parameters: &mut ParameterBuilder,
) -> Result<(String, String), OrmError> {
    let mut columns = Vec::with_capacity(values.len());
    let mut placeholders = Vec::with_capacity(values.len());

    for value in values {
        columns.push(quote_identifier(value.column_name)?);
        placeholders.push(parameters.push(value.value.clone()));
    }

    Ok((columns.join(", "), placeholders.join(", ")))
}

fn compile_assignments(
    changes: &[ColumnValue],
    parameters: &mut ParameterBuilder,
) -> Result<String, OrmError> {
    let assignments = changes
        .iter()
        .map(|change| {
            Ok(format!(
                "{} = {}",
                quote_identifier(change.column_name)?,
                parameters.push(change.value.clone()),
            ))
        })
        .collect::<Result<Vec<_>, OrmError>>()?;

    Ok(assignments.join(", "))
}

fn compile_binary_op(op: BinaryOp) -> &'static str {
    match op {
        BinaryOp::Add => "+",
        BinaryOp::Subtract => "-",
        BinaryOp::Multiply => "*",
        BinaryOp::Divide => "/",
    }
}

fn compile_unary_op(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Negate => "-",
    }
}

#[cfg(test)]
mod tests {
    use super::super::SqlServerCompiler;
    use sql_orm_core::{
        Changeset, ColumnMetadata, ColumnValue, Entity, EntityColumn, EntityMetadata,
        IdentityMetadata, Insertable, PrimaryKeyMetadata, SqlServerType, SqlValue,
    };
    use sql_orm_query::{
        BinaryOp, CountQuery, DeleteQuery, Expr, InsertQuery, OrderBy, Pagination, Predicate,
        Query, SelectProjection, SelectQuery, TableRef, UnaryOp, UpdateQuery,
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
    fn compiles_select_with_predicates_order_and_pagination() {
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

        assert_eq!(
            compiled.sql,
            "SELECT [sales].[customers].[id] AS [id], [sales].[customers].[email] AS [email] FROM [sales].[customers] WHERE (([sales].[customers].[active] = @P1) AND ([sales].[customers].[email] LIKE @P2)) ORDER BY [sales].[customers].[created_at] DESC OFFSET @P3 ROWS FETCH NEXT @P4 ROWS ONLY"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::Bool(true),
                SqlValue::String("%@example.com".to_string()),
                SqlValue::I64(20),
                SqlValue::I64(20),
            ]
        );
    }

    #[test]
    fn compiles_select_without_projection_as_star() {
        let compiled =
            SqlServerCompiler::compile_select(&SelectQuery::from_entity::<Customer>()).unwrap();

        assert_eq!(compiled.sql, "SELECT * FROM [sales].[customers]");
        assert!(compiled.params.is_empty());
    }

    #[test]
    fn rejects_pagination_without_order_by() {
        let error = SqlServerCompiler::compile_select(
            &SelectQuery::from_entity::<Customer>().paginate(Pagination::page(1, 10)),
        )
        .unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server pagination requires ORDER BY before OFFSET/FETCH"
        );
    }

    #[test]
    fn compiles_explicit_joins_to_sql() {
        let query = SelectQuery::from_entity::<Customer>()
            .select(vec![
                Expr::from(Customer::email),
                Expr::from(Order::total_cents),
            ])
            .inner_join::<Order>(Predicate::eq(
                Expr::from(Customer::id),
                Expr::from(Order::customer_id),
            ))
            .filter(Predicate::gt(
                Expr::from(Order::total_cents),
                Expr::value(SqlValue::I64(1000)),
            ))
            .order_by(OrderBy::desc(Order::total_cents))
            .paginate(Pagination::page(1, 10));

        let compiled = SqlServerCompiler::compile_select(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "SELECT [sales].[customers].[email] AS [email], [sales].[orders].[total_cents] AS [total_cents] FROM [sales].[customers] INNER JOIN [sales].[orders] ON ([sales].[customers].[id] = [sales].[orders].[customer_id]) WHERE ([sales].[orders].[total_cents] > @P1) ORDER BY [sales].[orders].[total_cents] DESC OFFSET @P2 ROWS FETCH NEXT @P3 ROWS ONLY"
        );
        assert_eq!(
            compiled.params,
            vec![SqlValue::I64(1000), SqlValue::I64(0), SqlValue::I64(10)]
        );
    }

    #[test]
    fn rejects_duplicate_unaliased_joined_tables() {
        let error = SqlServerCompiler::compile_select(
            &SelectQuery::from_entity::<Customer>().inner_join::<Customer>(Predicate::eq(
                Expr::from(Customer::id),
                Expr::from(Customer::id),
            )),
        )
        .unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server join compilation requires aliases for repeated table sources"
        );
    }

    #[test]
    fn compiles_aliased_selects_with_repeated_joined_tables() {
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
                sql_orm_query::SortDirection::Desc,
            ))
            .paginate(Pagination::page(1, 10));

        let compiled = SqlServerCompiler::compile_select(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "SELECT [c].[email] AS [email], [created_orders].[total_cents] AS [total_cents] FROM [sales].[customers] AS [c] INNER JOIN [sales].[orders] AS [created_orders] ON ([c].[id] = [created_orders].[customer_id]) LEFT JOIN [sales].[orders] AS [completed_orders] ON ([completed_orders].[total_cents] >= @P1) WHERE ([created_orders].[total_cents] > @P2) ORDER BY [completed_orders].[total_cents] DESC OFFSET @P3 ROWS FETCH NEXT @P4 ROWS ONLY"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::I64(5000),
                SqlValue::I64(1000),
                SqlValue::I64(0),
                SqlValue::I64(10),
            ]
        );
    }

    #[test]
    fn compiles_aliased_count_query() {
        let query = CountQuery::from_entity_as::<Customer>("c").filter(Predicate::eq(
            Expr::column_as(Customer::active, "c"),
            Expr::value(SqlValue::Bool(true)),
        ));

        let compiled = SqlServerCompiler::compile_count(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "SELECT COUNT(*) AS [count] FROM [sales].[customers] AS [c] WHERE ([c].[active] = @P1)"
        );
        assert_eq!(compiled.params, vec![SqlValue::Bool(true)]);
    }

    #[test]
    fn rejects_empty_table_aliases() {
        let error = SqlServerCompiler::compile_select(
            &SelectQuery::from_entity_as::<Customer>("").inner_join_as::<Order>(
                "o",
                Predicate::eq(
                    Expr::column_as(Customer::id, ""),
                    Expr::column_as(Order::customer_id, "o"),
                ),
            ),
        )
        .unwrap_err();

        assert_eq!(error.message(), "SQL Server identifier cannot be empty");
    }

    #[test]
    fn compiles_insert_with_output_inserted_and_parameter_order() {
        let query = InsertQuery::for_entity::<Customer, _>(&NewCustomer {
            email: "ana@example.com".to_string(),
            active: true,
        });

        let compiled = SqlServerCompiler::compile_insert(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "INSERT INTO [sales].[customers] ([email], [active]) OUTPUT INSERTED.* VALUES (@P1, @P2)"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("ana@example.com".to_string()),
                SqlValue::Bool(true),
            ]
        );
    }

    #[test]
    fn compiles_update_with_output_inserted_and_where_clause() {
        let query = UpdateQuery::for_entity::<Customer, _>(&UpdateCustomer {
            email: Some("ana.maria@example.com".to_string()),
            active: Some(false),
        })
        .filter(Predicate::eq(
            Expr::from(Customer::id),
            Expr::value(SqlValue::I64(7)),
        ));

        let compiled = SqlServerCompiler::compile_update(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "UPDATE [sales].[customers] SET [email] = @P1, [active] = @P2 OUTPUT INSERTED.* WHERE ([sales].[customers].[id] = @P3)"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("ana.maria@example.com".to_string()),
                SqlValue::Bool(false),
                SqlValue::I64(7),
            ]
        );
    }

    #[test]
    fn compiles_delete_and_count_queries() {
        let delete = DeleteQuery::from_entity::<Customer>().filter(Predicate::eq(
            Expr::from(Customer::id),
            Expr::value(SqlValue::I64(7)),
        ));
        let count = CountQuery::from_entity::<Customer>().filter(Predicate::eq(
            Expr::from(Customer::active),
            Expr::value(SqlValue::Bool(true)),
        ));

        let compiled_delete = SqlServerCompiler::compile_delete(&delete).unwrap();
        let compiled_count = SqlServerCompiler::compile_count(&count).unwrap();

        assert_eq!(
            compiled_delete.sql,
            "DELETE FROM [sales].[customers] WHERE ([sales].[customers].[id] = @P1)"
        );
        assert_eq!(compiled_delete.params, vec![SqlValue::I64(7)]);
        assert_eq!(
            compiled_count.sql,
            "SELECT COUNT(*) AS [count] FROM [sales].[customers] WHERE ([sales].[customers].[active] = @P1)"
        );
        assert_eq!(compiled_count.params, vec![SqlValue::Bool(true)]);
    }

    #[test]
    fn compiles_query_enum_through_single_entry_point() {
        let query = Query::Count(CountQuery::from_entity::<Customer>().filter(Predicate::eq(
            Expr::from(Customer::active),
            Expr::value(SqlValue::Bool(true)),
        )));

        let compiled = SqlServerCompiler::compile_query(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "SELECT COUNT(*) AS [count] FROM [sales].[customers] WHERE ([sales].[customers].[active] = @P1)"
        );
        assert_eq!(compiled.params, vec![SqlValue::Bool(true)]);
    }

    #[test]
    fn compiles_functions_null_checks_and_unary_binary_exprs() {
        let query = SelectQuery {
            from: TableRef::new("sales", "customers"),
            joins: vec![],
            projection: vec![SelectProjection::expr_as(
                Expr::function(
                    "LOWER",
                    vec![Expr::binary(
                        Expr::from(Customer::email),
                        BinaryOp::Add,
                        Expr::value(SqlValue::String("@example.com".to_string())),
                    )],
                ),
                "email_lower",
            )],
            predicate: Some(Predicate::and(vec![
                Predicate::is_not_null(Expr::from(Customer::email)),
                Predicate::negate(Predicate::is_null(Expr::unary(
                    UnaryOp::Negate,
                    Expr::value(SqlValue::I64(1)),
                ))),
            ])),
            order_by: vec![],
            pagination: None,
        };

        let compiled = SqlServerCompiler::compile_select(&query).unwrap();

        assert_eq!(
            compiled.sql,
            "SELECT LOWER(([sales].[customers].[email] + @P1)) AS [email_lower] FROM [sales].[customers] WHERE (([sales].[customers].[email] IS NOT NULL) AND (NOT ((- @P2) IS NULL)))"
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("@example.com".to_string()),
                SqlValue::I64(1),
            ]
        );
    }

    #[test]
    fn rejects_projection_expression_without_alias() {
        let error = SqlServerCompiler::compile_select(
            &SelectQuery::from_entity::<Customer>().select(vec![SelectProjection::expr(
                Expr::function("LOWER", vec![Expr::from(Customer::email)]),
            )]),
        )
        .unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server projection expressions require an explicit alias"
        );
    }

    #[test]
    fn rejects_empty_or_duplicate_projection_aliases() {
        let empty_alias_error =
            SqlServerCompiler::compile_select(&SelectQuery::from_entity::<Customer>().select(
                vec![SelectProjection::expr_as(Expr::from(Customer::email), "")],
            ))
            .unwrap_err();

        assert_eq!(
            empty_alias_error.message(),
            "SQL Server projection alias cannot be empty"
        );

        let duplicate_alias_error = SqlServerCompiler::compile_select(
            &SelectQuery::from_entity::<Customer>().select(vec![
                SelectProjection::column(Customer::id),
                SelectProjection::expr_as(Expr::from(Customer::email), "id"),
            ]),
        )
        .unwrap_err();

        assert_eq!(
            duplicate_alias_error.message(),
            "SQL Server projection alias `id` is duplicated"
        );
    }

    #[test]
    fn rejects_empty_updates_and_empty_logical_predicates() {
        let empty_update = UpdateQuery::for_entity::<Customer, _>(&UpdateCustomer {
            email: None,
            active: None,
        });
        let update_error = SqlServerCompiler::compile_update(&empty_update).unwrap_err();

        assert_eq!(
            update_error.message(),
            "SQL Server update compilation requires at least one change"
        );

        let predicate_error = SqlServerCompiler::compile_select(
            &SelectQuery::from_entity::<Customer>().filter(Predicate::and(vec![])),
        )
        .unwrap_err();

        assert_eq!(
            predicate_error.message(),
            "logical predicate compilation requires at least one child predicate"
        );
    }
}
