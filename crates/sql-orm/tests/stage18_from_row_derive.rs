use sql_orm::prelude::*;
use std::collections::BTreeMap;

struct TestRow {
    values: BTreeMap<&'static str, SqlValue>,
}

impl Row for TestRow {
    fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
        Ok(self.values.get(column).cloned())
    }
}

#[derive(Debug, PartialEq, FromRow)]
struct ProjectionDto {
    id: i64,
    #[orm(column = "email_address")]
    email: String,
    display_name: Option<String>,
    missing_nullable_alias: Option<String>,
}

#[test]
fn derive_from_row_materializes_projection_aliases_and_nullable_fields() {
    let row = TestRow {
        values: BTreeMap::from([
            ("id", SqlValue::I64(7)),
            (
                "email_address",
                SqlValue::String("ana@example.com".to_string()),
            ),
            ("display_name", SqlValue::Null),
        ]),
    };

    let dto = ProjectionDto::from_row(&row).unwrap();

    assert_eq!(
        dto,
        ProjectionDto {
            id: 7,
            email: "ana@example.com".to_string(),
            display_name: None,
            missing_nullable_alias: None,
        }
    );
}
