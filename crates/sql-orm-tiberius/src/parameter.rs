use crate::error::{TiberiusErrorContext, map_tiberius_error};
use sql_orm_core::{OrmError, SqlServerType, SqlValue};
use sql_orm_query::CompiledQuery;
use std::collections::BTreeSet;
use tiberius::numeric::Numeric;
use tiberius::{Client, Query, QueryStream};

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum BoundSqlValue {
    Null,
    TypedNull(SqlServerType),
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
    Uuid(uuid::Uuid),
    Decimal(rust_decimal::Decimal),
    Date(chrono::NaiveDate),
    DateTime(chrono::NaiveDateTime),
}

#[derive(Debug, Clone, PartialEq)]
pub(crate) struct PreparedQuery {
    pub sql: String,
    pub params: Vec<BoundSqlValue>,
}

impl PreparedQuery {
    pub fn from_compiled(query: CompiledQuery) -> Self {
        Self {
            sql: query.sql,
            params: query.params.into_iter().map(BoundSqlValue::from).collect(),
        }
    }

    pub fn validate_parameter_count(&self) -> Result<(), OrmError> {
        let expected = sql_parameter_plan(&self.sql)?;

        if expected != self.params.len() {
            return Err(OrmError::compile(
                "compiled query parameter count does not match SQL placeholders",
            ));
        }

        Ok(())
    }

    pub async fn execute<S>(
        self,
        client: &mut Client<S>,
    ) -> Result<tiberius::ExecuteResult, OrmError>
    where
        S: futures_io::AsyncRead + futures_io::AsyncWrite + Unpin + Send,
    {
        self.execute_driver(client)
            .await
            .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ExecuteQuery))
    }

    pub async fn query<'a, S>(self, client: &'a mut Client<S>) -> Result<QueryStream<'a>, OrmError>
    where
        S: futures_io::AsyncRead + futures_io::AsyncWrite + Unpin + Send,
    {
        self.query_driver(client)
            .await
            .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ExecuteQuery))
    }

    pub async fn execute_driver<S>(
        self,
        client: &mut Client<S>,
    ) -> Result<tiberius::ExecuteResult, tiberius::error::Error>
    where
        S: futures_io::AsyncRead + futures_io::AsyncWrite + Unpin + Send,
    {
        let mut query = Query::new(self.sql.as_str());

        for param in &self.params {
            bind_sql_value(&mut query, param);
        }

        query.execute(client).await
    }

    pub async fn query_driver<'a, S>(
        self,
        client: &'a mut Client<S>,
    ) -> Result<QueryStream<'a>, tiberius::error::Error>
    where
        S: futures_io::AsyncRead + futures_io::AsyncWrite + Unpin + Send,
    {
        let mut query = Query::new(self.sql.as_str());

        for param in &self.params {
            bind_sql_value(&mut query, param);
        }

        query.query(client).await
    }
}

impl From<SqlValue> for BoundSqlValue {
    fn from(value: SqlValue) -> Self {
        match value {
            SqlValue::Null => Self::Null,
            SqlValue::TypedNull(sql_type) => Self::TypedNull(sql_type),
            SqlValue::Bool(value) => Self::Bool(value),
            SqlValue::I32(value) => Self::I32(value),
            SqlValue::I64(value) => Self::I64(value),
            SqlValue::F64(value) => Self::F64(value),
            SqlValue::String(value) => Self::String(value),
            SqlValue::Bytes(value) => Self::Bytes(value),
            SqlValue::Uuid(value) => Self::Uuid(value),
            SqlValue::Decimal(value) => Self::Decimal(value),
            SqlValue::Date(value) => Self::Date(value),
            SqlValue::DateTime(value) => Self::DateTime(value),
        }
    }
}

fn bind_sql_value<'a>(query: &mut Query<'a>, value: &'a BoundSqlValue) {
    match value {
        BoundSqlValue::Null => query.bind(Option::<String>::None),
        BoundSqlValue::TypedNull(sql_type) => bind_typed_null(query, *sql_type),
        BoundSqlValue::Bool(value) => query.bind(*value),
        BoundSqlValue::I32(value) => query.bind(*value),
        BoundSqlValue::I64(value) => query.bind(*value),
        BoundSqlValue::F64(value) => query.bind(*value),
        BoundSqlValue::String(value) => query.bind(value),
        BoundSqlValue::Bytes(value) => query.bind(value),
        BoundSqlValue::Uuid(value) => query.bind(value),
        BoundSqlValue::Decimal(value) => query.bind(Numeric::new_with_scale(
            value.mantissa(),
            value.scale() as u8,
        )),
        BoundSqlValue::Date(value) => query.bind(*value),
        BoundSqlValue::DateTime(value) => query.bind(*value),
    }
}

fn bind_typed_null<'a>(query: &mut Query<'a>, sql_type: SqlServerType) {
    match sql_type {
        SqlServerType::BigInt => query.bind(Option::<i64>::None),
        SqlServerType::Int => query.bind(Option::<i32>::None),
        SqlServerType::SmallInt => query.bind(Option::<i16>::None),
        SqlServerType::TinyInt => query.bind(Option::<u8>::None),
        SqlServerType::Bit => query.bind(Option::<bool>::None),
        SqlServerType::UniqueIdentifier => query.bind(Option::<uuid::Uuid>::None),
        SqlServerType::Date => query.bind(Option::<chrono::NaiveDate>::None),
        SqlServerType::DateTime2 => query.bind(Option::<chrono::NaiveDateTime>::None),
        SqlServerType::Decimal => query.bind(Option::<Numeric>::None),
        SqlServerType::Float => query.bind(Option::<f64>::None),
        SqlServerType::Money => query.bind(Option::<f64>::None),
        SqlServerType::NVarChar => query.bind(Option::<String>::None),
        SqlServerType::VarBinary | SqlServerType::RowVersion => query.bind(Option::<Vec<u8>>::None),
        SqlServerType::Custom(_) => query.bind(Option::<String>::None),
    }
}

fn sql_parameter_plan(sql: &str) -> Result<usize, OrmError> {
    let bytes = sql.as_bytes();
    let mut index = 0;
    let mut placeholders = BTreeSet::new();

    while index + 2 < bytes.len() {
        if let Some(next_index) = skip_sql_non_code(bytes, index) {
            index = next_index;
            continue;
        }

        if bytes[index] == b'@' && bytes[index + 1] == b'P' && bytes[index + 2].is_ascii_digit() {
            index += 2;
            let start = index;

            while index < bytes.len() && bytes[index].is_ascii_digit() {
                index += 1;
            }

            let parameter_index = sql[start..index].parse::<usize>().map_err(|_| {
                OrmError::compile("compiled query placeholder index is larger than supported")
            })?;

            if parameter_index == 0 {
                return Err(OrmError::compile(
                    "compiled query placeholders must start at @P1",
                ));
            }

            placeholders.insert(parameter_index);
            continue;
        }

        index += 1;
    }

    let max_index = placeholders.iter().next_back().copied().unwrap_or(0);
    for expected in 1..=max_index {
        if !placeholders.contains(&expected) {
            return Err(OrmError::compile(format!(
                "compiled query placeholders must be continuous from @P1 to @P{}",
                max_index
            )));
        }
    }

    Ok(max_index)
}

fn skip_sql_non_code(bytes: &[u8], index: usize) -> Option<usize> {
    match bytes[index] {
        b'\'' => Some(skip_quoted_string(bytes, index)),
        b'[' => Some(skip_bracket_identifier(bytes, index)),
        b'"' => Some(skip_double_quoted_identifier(bytes, index)),
        b'-' if index + 1 < bytes.len() && bytes[index + 1] == b'-' => {
            Some(skip_line_comment(bytes, index))
        }
        b'/' if index + 1 < bytes.len() && bytes[index + 1] == b'*' => {
            Some(skip_block_comment(bytes, index))
        }
        _ => None,
    }
}

fn skip_quoted_string(bytes: &[u8], mut index: usize) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b'\'' {
            index += 1;
            if index < bytes.len() && bytes[index] == b'\'' {
                index += 1;
                continue;
            }
            break;
        }
        index += 1;
    }
    index
}

fn skip_bracket_identifier(bytes: &[u8], mut index: usize) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b']' {
            index += 1;
            if index < bytes.len() && bytes[index] == b']' {
                index += 1;
                continue;
            }
            break;
        }
        index += 1;
    }
    index
}

fn skip_double_quoted_identifier(bytes: &[u8], mut index: usize) -> usize {
    index += 1;
    while index < bytes.len() {
        if bytes[index] == b'"' {
            index += 1;
            if index < bytes.len() && bytes[index] == b'"' {
                index += 1;
                continue;
            }
            break;
        }
        index += 1;
    }
    index
}

fn skip_line_comment(bytes: &[u8], mut index: usize) -> usize {
    index += 2;
    while index < bytes.len() && !matches!(bytes[index], b'\n' | b'\r') {
        index += 1;
    }
    index
}

fn skip_block_comment(bytes: &[u8], mut index: usize) -> usize {
    index += 2;
    while index + 1 < bytes.len() {
        if bytes[index] == b'*' && bytes[index + 1] == b'/' {
            return index + 2;
        }
        index += 1;
    }
    bytes.len()
}

#[cfg(test)]
mod tests {
    use super::{BoundSqlValue, PreparedQuery};
    use chrono::NaiveDate;
    use rust_decimal::Decimal;
    use sql_orm_core::{OrmErrorKind, SqlServerType, SqlValue};
    use sql_orm_query::CompiledQuery;
    use uuid::Uuid;

    #[test]
    fn prepares_query_preserving_sql_and_parameter_order() {
        let compiled = CompiledQuery::new(
            "SELECT @P1, @P2, @P3, @P4, @P5, @P6, @P7, @P8, @P9, @P10",
            vec![
                SqlValue::Null,
                SqlValue::Bool(true),
                SqlValue::I32(1),
                SqlValue::I64(2),
                SqlValue::F64(3.5),
                SqlValue::String("ana@example.com".to_string()),
                SqlValue::Bytes(vec![1, 2, 3]),
                SqlValue::Uuid(Uuid::nil()),
                SqlValue::Decimal(Decimal::new(1234, 2)),
                SqlValue::DateTime(
                    NaiveDate::from_ymd_opt(2026, 4, 23)
                        .unwrap()
                        .and_hms_opt(10, 20, 30)
                        .unwrap(),
                ),
            ],
        );

        let prepared = PreparedQuery::from_compiled(compiled);

        assert_eq!(
            prepared.sql,
            "SELECT @P1, @P2, @P3, @P4, @P5, @P6, @P7, @P8, @P9, @P10"
        );
        assert_eq!(
            prepared.params,
            vec![
                BoundSqlValue::Null,
                BoundSqlValue::Bool(true),
                BoundSqlValue::I32(1),
                BoundSqlValue::I64(2),
                BoundSqlValue::F64(3.5),
                BoundSqlValue::String("ana@example.com".to_string()),
                BoundSqlValue::Bytes(vec![1, 2, 3]),
                BoundSqlValue::Uuid(Uuid::nil()),
                BoundSqlValue::Decimal(Decimal::new(1234, 2)),
                BoundSqlValue::DateTime(
                    NaiveDate::from_ymd_opt(2026, 4, 23)
                        .unwrap()
                        .and_hms_opt(10, 20, 30)
                        .unwrap(),
                ),
            ]
        );
    }

    #[test]
    fn prepares_typed_null_preserving_sql_type() {
        let prepared = PreparedQuery::from_compiled(CompiledQuery::new(
            "SELECT @P1, @P2",
            vec![
                SqlValue::TypedNull(SqlServerType::BigInt),
                SqlValue::TypedNull(SqlServerType::DateTime2),
            ],
        ));

        assert_eq!(
            prepared.params,
            vec![
                BoundSqlValue::TypedNull(SqlServerType::BigInt),
                BoundSqlValue::TypedNull(SqlServerType::DateTime2),
            ]
        );
    }

    #[test]
    fn validates_parameter_count_against_sql_placeholders() {
        let prepared = PreparedQuery::from_compiled(CompiledQuery::new(
            "SELECT @P1, @P2",
            vec![SqlValue::Bool(true), SqlValue::Bool(false)],
        ));

        assert!(prepared.validate_parameter_count().is_ok());
    }

    #[test]
    fn validates_repeated_placeholders_by_max_index() {
        let prepared = PreparedQuery::from_compiled(CompiledQuery::new(
            "SELECT @P1 WHERE owner_id = @P1",
            vec![SqlValue::I64(7)],
        ));

        assert!(prepared.validate_parameter_count().is_ok());
    }

    #[test]
    fn ignores_placeholders_inside_sql_non_code_regions() {
        let prepared = PreparedQuery::from_compiled(CompiledQuery::new(
            r#"
                SELECT @P1 AS value,
                       '@P2 literal '' @P3 escaped quote' AS string_value,
                       [@P4 identifier] AS bracket_identifier,
                       "@P5 quoted identifier" AS quoted_identifier
                -- @P6 line comment
                /* @P7 block comment */
                WHERE id = @P1
            "#,
            vec![SqlValue::I64(7)],
        ));

        assert!(prepared.validate_parameter_count().is_ok());
    }

    #[test]
    fn ignores_placeholder_text_when_no_parameters_are_bound() {
        let prepared = PreparedQuery::from_compiled(CompiledQuery::new(
            r#"
                SELECT '@P1 literal' AS literal,
                       [@P2 identifier] AS identifier
                -- @P3 comment
                /* @P4 comment */
            "#,
            vec![],
        ));

        assert!(prepared.validate_parameter_count().is_ok());
    }

    #[test]
    fn rejects_mismatched_parameter_count() {
        let prepared = PreparedQuery::from_compiled(CompiledQuery::new(
            "SELECT @P1, @P2",
            vec![SqlValue::Bool(true)],
        ));

        let error = prepared.validate_parameter_count().unwrap_err();

        assert_eq!(
            error.message(),
            "compiled query parameter count does not match SQL placeholders"
        );
        assert_eq!(error.kind(), OrmErrorKind::Compile);
    }

    #[test]
    fn rejects_zero_based_placeholders_as_compile_errors() {
        let prepared = PreparedQuery::from_compiled(CompiledQuery::new(
            "SELECT @P0",
            vec![SqlValue::Bool(true)],
        ));

        let error = prepared.validate_parameter_count().unwrap_err();

        assert_eq!(
            error.message(),
            "compiled query placeholders must start at @P1"
        );
        assert_eq!(error.kind(), OrmErrorKind::Compile);
    }

    #[test]
    fn supports_date_values_in_prepared_query() {
        let prepared = PreparedQuery::from_compiled(CompiledQuery::new(
            "SELECT @P1",
            vec![SqlValue::Date(
                NaiveDate::from_ymd_opt(2026, 4, 23).unwrap(),
            )],
        ));

        assert_eq!(
            prepared.params,
            vec![BoundSqlValue::Date(
                NaiveDate::from_ymd_opt(2026, 4, 23).unwrap()
            )]
        );
    }
}
