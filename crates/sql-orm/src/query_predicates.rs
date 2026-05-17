use crate::AliasedEntityColumn;
use sql_orm_core::{Entity, EntityColumn, SqlTypeMapping, SqlValue};
use sql_orm_query::{Expr, Predicate};

const LIKE_ESCAPE_CHAR: char = '\\';

// The plan maestro fija explícitamente `is_null` e `is_not_null` como API pública.
#[allow(clippy::wrong_self_convention)]
pub trait EntityColumnPredicateExt<E: Entity> {
    fn eq<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping;

    fn ne<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping;

    fn gt<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping;

    fn gte<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping;

    fn lt<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping;

    fn lte<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping;

    fn is_null(self) -> Predicate;

    fn is_not_null(self) -> Predicate;

    fn contains(self, value: impl Into<String>) -> Predicate;

    fn starts_with(self, value: impl Into<String>) -> Predicate;

    fn ends_with(self, value: impl Into<String>) -> Predicate;
}

impl<E: Entity> EntityColumnPredicateExt<E> for EntityColumn<E> {
    fn eq<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::eq(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn ne<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::ne(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn gt<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::gt(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn gte<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::gte(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn lt<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::lt(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn lte<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::lte(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn is_null(self) -> Predicate {
        Predicate::is_null(Expr::from(self))
    }

    fn is_not_null(self) -> Predicate {
        Predicate::is_not_null(Expr::from(self))
    }

    fn contains(self, value: impl Into<String>) -> Predicate {
        Predicate::like_escaped(
            Expr::from(self),
            Expr::value(SqlValue::String(format!(
                "%{}%",
                escape_like_literal(value.into())
            ))),
            LIKE_ESCAPE_CHAR,
        )
    }

    fn starts_with(self, value: impl Into<String>) -> Predicate {
        Predicate::like_escaped(
            Expr::from(self),
            Expr::value(SqlValue::String(format!(
                "{}%",
                escape_like_literal(value.into())
            ))),
            LIKE_ESCAPE_CHAR,
        )
    }

    fn ends_with(self, value: impl Into<String>) -> Predicate {
        Predicate::like_escaped(
            Expr::from(self),
            Expr::value(SqlValue::String(format!(
                "%{}",
                escape_like_literal(value.into())
            ))),
            LIKE_ESCAPE_CHAR,
        )
    }
}

impl<E: Entity> EntityColumnPredicateExt<E> for AliasedEntityColumn<E> {
    fn eq<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::eq(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn ne<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::ne(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn gt<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::gt(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn gte<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::gte(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn lt<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::lt(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn lte<V>(self, value: V) -> Predicate
    where
        V: SqlTypeMapping,
    {
        Predicate::lte(Expr::from(self), Expr::value(value.to_sql_value()))
    }

    fn is_null(self) -> Predicate {
        Predicate::is_null(Expr::from(self))
    }

    fn is_not_null(self) -> Predicate {
        Predicate::is_not_null(Expr::from(self))
    }

    fn contains(self, value: impl Into<String>) -> Predicate {
        Predicate::like_escaped(
            Expr::from(self),
            Expr::value(SqlValue::String(format!(
                "%{}%",
                escape_like_literal(value.into())
            ))),
            LIKE_ESCAPE_CHAR,
        )
    }

    fn starts_with(self, value: impl Into<String>) -> Predicate {
        Predicate::like_escaped(
            Expr::from(self),
            Expr::value(SqlValue::String(format!(
                "{}%",
                escape_like_literal(value.into())
            ))),
            LIKE_ESCAPE_CHAR,
        )
    }

    fn ends_with(self, value: impl Into<String>) -> Predicate {
        Predicate::like_escaped(
            Expr::from(self),
            Expr::value(SqlValue::String(format!(
                "%{}",
                escape_like_literal(value.into())
            ))),
            LIKE_ESCAPE_CHAR,
        )
    }
}

fn escape_like_literal(value: impl AsRef<str>) -> String {
    let value = value.as_ref();
    let mut escaped = String::with_capacity(value.len());

    for ch in value.chars() {
        if matches!(ch, LIKE_ESCAPE_CHAR | '%' | '_' | '[' | ']') {
            escaped.push(LIKE_ESCAPE_CHAR);
        }
        escaped.push(ch);
    }

    escaped
}

#[cfg(test)]
mod tests {
    use super::{EntityColumnPredicateExt, LIKE_ESCAPE_CHAR};
    use crate::EntityColumnAliasExt;
    use sql_orm_core::{
        ColumnMetadata, Entity, EntityColumn, EntityMetadata, PrimaryKeyMetadata, SqlServerType,
        SqlValue,
    };
    use sql_orm_query::{ColumnRef, Expr, Predicate, TableRef};

    struct TestEntity;

    static TEST_ENTITY_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
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
            rust_field: "name",
            column_name: "name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: true,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
    ];

    static TEST_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TestEntity",
        schema: "dbo",
        table: "test_entities",
        renamed_from: None,
        columns: &TEST_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    impl Entity for TestEntity {
        fn metadata() -> &'static EntityMetadata {
            &TEST_ENTITY_METADATA
        }
    }

    #[allow(non_upper_case_globals)]
    impl TestEntity {
        const id: EntityColumn<TestEntity> = EntityColumn::new("id", "id");
        const name: EntityColumn<TestEntity> = EntityColumn::new("name", "name");
    }

    #[test]
    fn comparison_methods_build_expected_predicates() {
        let expected_column = Expr::Column(ColumnRef::new(
            TableRef::new("dbo", "test_entities"),
            "id",
            "id",
        ));

        assert_eq!(
            TestEntity::id.eq(7_i64),
            Predicate::eq(expected_column.clone(), Expr::Value(SqlValue::I64(7)))
        );
        assert_eq!(
            TestEntity::id.ne(8_i64),
            Predicate::ne(expected_column.clone(), Expr::Value(SqlValue::I64(8)))
        );
        assert_eq!(
            TestEntity::id.gt(9_i64),
            Predicate::gt(expected_column.clone(), Expr::Value(SqlValue::I64(9)))
        );
        assert_eq!(
            TestEntity::id.gte(10_i64),
            Predicate::gte(expected_column.clone(), Expr::Value(SqlValue::I64(10)))
        );
        assert_eq!(
            TestEntity::id.lt(11_i64),
            Predicate::lt(expected_column.clone(), Expr::Value(SqlValue::I64(11)))
        );
        assert_eq!(
            TestEntity::id.lte(12_i64),
            Predicate::lte(expected_column, Expr::Value(SqlValue::I64(12)))
        );
    }

    #[test]
    fn null_predicate_methods_build_expected_predicates() {
        let expected_column = Expr::Column(ColumnRef::new(
            TableRef::new("dbo", "test_entities"),
            "name",
            "name",
        ));

        assert_eq!(
            TestEntity::name.is_null(),
            Predicate::is_null(expected_column.clone())
        );
        assert_eq!(
            TestEntity::name.is_not_null(),
            Predicate::is_not_null(expected_column)
        );
    }

    #[test]
    fn string_predicate_methods_build_expected_like_patterns() {
        let expected_column = Expr::Column(ColumnRef::new(
            TableRef::new("dbo", "test_entities"),
            "name",
            "name",
        ));

        assert_eq!(
            TestEntity::name.contains("ana"),
            Predicate::like_escaped(
                expected_column.clone(),
                Expr::Value(SqlValue::String("%ana%".to_string())),
                LIKE_ESCAPE_CHAR
            )
        );
        assert_eq!(
            TestEntity::name.starts_with("ana"),
            Predicate::like_escaped(
                expected_column.clone(),
                Expr::Value(SqlValue::String("ana%".to_string())),
                LIKE_ESCAPE_CHAR
            )
        );
        assert_eq!(
            TestEntity::name.ends_with("ana"),
            Predicate::like_escaped(
                expected_column,
                Expr::Value(SqlValue::String("%ana".to_string())),
                LIKE_ESCAPE_CHAR
            )
        );
    }

    #[test]
    fn string_predicate_methods_escape_like_wildcards_and_ranges() {
        let expected_column = Expr::Column(ColumnRef::new(
            TableRef::new("dbo", "test_entities"),
            "name",
            "name",
        ));

        assert_eq!(
            TestEntity::name.contains(r"a%_b[c]\d"),
            Predicate::like_escaped(
                expected_column,
                Expr::Value(SqlValue::String(r"%a\%\_b\[c\]\\d%".to_string())),
                LIKE_ESCAPE_CHAR
            )
        );
    }

    #[test]
    fn aliased_columns_build_predicates_against_table_alias() {
        let expected_column = Expr::Column(ColumnRef::new(
            TableRef::with_alias("dbo", "test_entities", "t"),
            "name",
            "name",
        ));

        assert_eq!(
            TestEntity::name.aliased("t").contains("ana"),
            Predicate::like_escaped(
                expected_column.clone(),
                Expr::Value(SqlValue::String("%ana%".to_string())),
                LIKE_ESCAPE_CHAR
            )
        );
        assert_eq!(
            TestEntity::name.aliased("t").is_not_null(),
            Predicate::is_not_null(expected_column)
        );
    }
}
