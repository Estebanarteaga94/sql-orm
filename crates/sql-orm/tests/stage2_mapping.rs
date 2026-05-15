use std::collections::BTreeMap;

use sql_orm::prelude::*;

#[allow(dead_code)]
#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "customers", schema = "sales")]
struct Customer {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    email: String,
    #[orm(nullable)]
    phone: Option<String>,
    active: bool,
}

#[derive(Insertable, Debug, Clone)]
#[orm(entity = Customer)]
struct NewCustomer {
    email: String,
    phone: Option<String>,
    active: bool,
}

#[derive(Changeset, Debug, Clone)]
#[orm(entity = Customer)]
struct UpdateCustomer {
    email: Option<String>,
    phone: Option<Option<String>>,
    active: Option<bool>,
}

struct TestRow {
    values: BTreeMap<&'static str, SqlValue>,
}

impl Row for TestRow {
    fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
        Ok(self.values.get(column).cloned())
    }
}

#[derive(Debug, PartialEq)]
struct CustomerRecord {
    id: i64,
    email: String,
    phone: Option<String>,
    active: bool,
}

impl FromRow for CustomerRecord {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            id: row.get_required_typed::<i64>("id")?,
            email: row.get_required_typed::<String>("email")?,
            phone: row.try_get_typed::<Option<String>>("phone")?.flatten(),
            active: row.get_required_typed::<bool>("active")?,
        })
    }
}

#[test]
fn from_row_maps_required_and_nullable_columns() {
    let row = TestRow {
        values: BTreeMap::from([
            ("id", SqlValue::I64(7)),
            ("email", SqlValue::String("ana@example.com".to_string())),
            ("phone", SqlValue::Null),
            ("active", SqlValue::Bool(true)),
        ]),
    };

    let record = CustomerRecord::from_row(&row).expect("row mapping should succeed");

    assert_eq!(
        record,
        CustomerRecord {
            id: 7,
            email: "ana@example.com".to_string(),
            phone: None,
            active: true,
        }
    );
}

#[test]
fn entity_derive_generates_from_row_for_required_and_nullable_columns() {
    let row = TestRow {
        values: BTreeMap::from([
            ("id", SqlValue::I64(7)),
            ("email", SqlValue::String("ana@example.com".to_string())),
            ("phone", SqlValue::Null),
            ("active", SqlValue::Bool(true)),
        ]),
    };

    let customer = Customer::from_row(&row).expect("derived entity row mapping should succeed");

    assert_eq!(
        customer,
        Customer {
            id: 7,
            email: "ana@example.com".to_string(),
            phone: None,
            active: true,
        }
    );
}

#[test]
fn from_row_reports_missing_required_columns() {
    let row = TestRow {
        values: BTreeMap::from([("email", SqlValue::String("ana@example.com".to_string()))]),
    };

    let error = CustomerRecord::from_row(&row).expect_err("missing id must fail");

    assert_eq!(error.message(), "required column value was not present");
}

#[test]
fn from_row_reports_type_mismatches() {
    let row = TestRow {
        values: BTreeMap::from([
            ("id", SqlValue::String("7".to_string())),
            ("email", SqlValue::String("ana@example.com".to_string())),
            ("active", SqlValue::Bool(true)),
        ]),
    };

    let error = CustomerRecord::from_row(&row).expect_err("invalid id type must fail");

    assert_eq!(error.message(), "expected i64 value");
}

#[test]
fn insertable_extracts_persistible_values_in_field_order() {
    let new_customer = NewCustomer {
        email: "ana@example.com".to_string(),
        phone: None,
        active: true,
    };

    let values = <NewCustomer as Insertable<Customer>>::values(&new_customer);

    assert_eq!(
        values,
        vec![
            ColumnValue::new("email", SqlValue::String("ana@example.com".to_string())),
            ColumnValue::new("phone", SqlValue::TypedNull(SqlServerType::NVarChar)),
            ColumnValue::new("active", SqlValue::Bool(true)),
        ]
    );
}

#[test]
fn changeset_extracts_only_present_changes_and_preserves_nulls() {
    let update = UpdateCustomer {
        email: None,
        phone: Some(None),
        active: Some(false),
    };

    let changes = <UpdateCustomer as Changeset<Customer>>::changes(&update);

    assert_eq!(
        changes,
        vec![
            ColumnValue::new("phone", SqlValue::TypedNull(SqlServerType::NVarChar)),
            ColumnValue::new("active", SqlValue::Bool(false)),
        ]
    );
}
