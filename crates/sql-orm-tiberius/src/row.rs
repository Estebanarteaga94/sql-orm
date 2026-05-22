use crate::error::{TiberiusErrorContext, map_tiberius_error};
use chrono::{NaiveDate, NaiveDateTime};
use rust_decimal::Decimal;
use sql_orm_core::{OrmError, Row as OrmRow, SqlValue};
use tiberius::{ColumnType, Row};
use uuid::Uuid;

#[derive(Debug, Clone, Copy)]
pub struct MssqlRow<'a> {
    inner: &'a Row,
}

impl<'a> MssqlRow<'a> {
    pub fn new(inner: &'a Row) -> Self {
        Self { inner }
    }

    pub fn inner(&self) -> &'a Row {
        self.inner
    }
}

impl OrmRow for MssqlRow<'_> {
    fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
        let Some((index, column_type)) =
            self.inner
                .columns()
                .iter()
                .enumerate()
                .find_map(|(index, metadata)| {
                    (metadata.name() == column).then_some((index, metadata.column_type()))
                })
        else {
            return Ok(None);
        };

        read_sql_value(self.inner, index, column_type).map(Some)
    }
}

fn read_sql_value(row: &Row, index: usize, column_type: ColumnType) -> Result<SqlValue, OrmError> {
    if let Some(value) = static_sql_value(column_type) {
        return Ok(value);
    }

    if let Some(error) = unsupported_column_type_error(column_type) {
        return Err(error);
    }

    match column_type {
        ColumnType::Bit | ColumnType::Bitn => {
            read_typed(row, index, |value: bool| SqlValue::Bool(value))
        }
        ColumnType::Int1 => read_typed(row, index, |value: u8| SqlValue::I32(i32::from(value))),
        ColumnType::Int2 => read_typed(row, index, |value: i16| SqlValue::I32(i32::from(value))),
        ColumnType::Int4 => read_typed(row, index, |value: i32| SqlValue::I32(value)),
        ColumnType::Int8 => read_typed(row, index, |value: i64| SqlValue::I64(value)),
        ColumnType::Intn => read_intn(row, index),
        ColumnType::Float4 => read_typed(row, index, |value: f32| SqlValue::F64(f64::from(value))),
        ColumnType::Float8 | ColumnType::Floatn | ColumnType::Money | ColumnType::Money4 => {
            read_typed(row, index, |value: f64| SqlValue::F64(value))
        }
        ColumnType::Guid => read_typed(row, index, |value: Uuid| SqlValue::Uuid(value)),
        ColumnType::Decimaln | ColumnType::Numericn => {
            read_typed(row, index, |value: Decimal| SqlValue::Decimal(value))
        }
        ColumnType::Daten => read_typed(row, index, |value: NaiveDate| SqlValue::Date(value)),
        ColumnType::Datetime
        | ColumnType::Datetime4
        | ColumnType::Datetimen
        | ColumnType::Datetime2 => {
            read_typed(row, index, |value: NaiveDateTime| SqlValue::DateTime(value))
        }
        ColumnType::BigVarChar
        | ColumnType::BigChar
        | ColumnType::NVarchar
        | ColumnType::NChar
        | ColumnType::Text
        | ColumnType::NText => read_string(row, index),
        ColumnType::BigVarBin | ColumnType::BigBinary | ColumnType::Image => read_bytes(row, index),
        ColumnType::Null
        | ColumnType::Timen
        | ColumnType::DatetimeOffsetn
        | ColumnType::Xml
        | ColumnType::Udt
        | ColumnType::SSVariant => {
            unreachable!("special-case column type should have returned early")
        }
    }
}

fn static_sql_value(column_type: ColumnType) -> Option<SqlValue> {
    match column_type {
        ColumnType::Null => Some(SqlValue::Null),
        _ => None,
    }
}

fn unsupported_column_type_error(column_type: ColumnType) -> Option<OrmError> {
    match column_type {
        ColumnType::Timen
        | ColumnType::DatetimeOffsetn
        | ColumnType::Xml
        | ColumnType::Udt
        | ColumnType::SSVariant => Some(OrmError::mapping(
            "unsupported SQL Server column type in MssqlRow",
        )),
        _ => None,
    }
}

fn read_typed<T>(
    row: &Row,
    index: usize,
    map: impl FnOnce(T) -> SqlValue,
) -> Result<SqlValue, OrmError>
where
    for<'a> T: tiberius::FromSql<'a>,
{
    let value = row
        .try_get::<T, _>(index)
        .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ReadRowValue))?;

    Ok(value.map(map).unwrap_or(SqlValue::Null))
}

fn read_string(row: &Row, index: usize) -> Result<SqlValue, OrmError> {
    let value = row
        .try_get::<&str, _>(index)
        .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ReadRowValue))?;

    Ok(value
        .map(|value| SqlValue::String(value.to_owned()))
        .unwrap_or(SqlValue::Null))
}

fn read_bytes(row: &Row, index: usize) -> Result<SqlValue, OrmError> {
    let value = row
        .try_get::<&[u8], _>(index)
        .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ReadRowValue))?;

    Ok(value
        .map(|value| SqlValue::Bytes(value.to_vec()))
        .unwrap_or(SqlValue::Null))
}

fn read_intn(row: &Row, index: usize) -> Result<SqlValue, OrmError> {
    if let Some(value) = row
        .try_get::<i64, _>(index)
        .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ReadRowValue))?
    {
        return Ok(SqlValue::I64(value));
    }

    if let Some(value) = row
        .try_get::<i32, _>(index)
        .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ReadRowValue))?
    {
        return Ok(SqlValue::I32(value));
    }

    if let Some(value) = row
        .try_get::<i16, _>(index)
        .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ReadRowValue))?
    {
        return Ok(SqlValue::I32(i32::from(value)));
    }

    if let Some(value) = row
        .try_get::<u8, _>(index)
        .map_err(|error| map_tiberius_error(&error, TiberiusErrorContext::ReadRowValue))?
    {
        return Ok(SqlValue::I32(i32::from(value)));
    }

    Ok(SqlValue::Null)
}

#[cfg(test)]
mod tests {
    use super::{static_sql_value, unsupported_column_type_error};
    use sql_orm_core::{OrmErrorKind, SqlValue};
    use tiberius::ColumnType;

    #[test]
    fn reports_unsupported_sql_server_column_types() {
        for column_type in [
            ColumnType::Timen,
            ColumnType::DatetimeOffsetn,
            ColumnType::Xml,
            ColumnType::Udt,
            ColumnType::SSVariant,
        ] {
            let error = unsupported_column_type_error(column_type).unwrap();
            assert_eq!(
                error.message(),
                "unsupported SQL Server column type in MssqlRow"
            );
            assert_eq!(error.kind(), OrmErrorKind::Mapping);
        }
    }

    #[test]
    fn treats_sql_null_columns_as_sql_value_null() {
        let value = static_sql_value(ColumnType::Null).unwrap();

        assert_eq!(value, SqlValue::Null);
    }
}
