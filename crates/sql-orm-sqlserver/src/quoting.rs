use sql_orm_core::OrmError;
use sql_orm_query::{ColumnRef, TableRef};

pub fn quote_identifier(identifier: &str) -> Result<String, OrmError> {
    validate_identifier(identifier)?;

    let escaped = identifier.replace(']', "]]");
    Ok(format!("[{escaped}]"))
}

pub fn quote_qualified_identifier(schema: &str, identifier: &str) -> Result<String, OrmError> {
    Ok(format!(
        "{}.{}",
        quote_identifier(schema)?,
        quote_identifier(identifier)?,
    ))
}

pub fn quote_table_ref(table: &TableRef) -> Result<String, OrmError> {
    quote_qualified_identifier(table.schema, table.table)
}

pub fn quote_table_source(table: &TableRef) -> Result<String, OrmError> {
    let source = quote_table_ref(table)?;

    match table.alias {
        Some(alias) => Ok(format!("{source} AS {}", quote_identifier(alias)?)),
        None => Ok(source),
    }
}

pub fn quote_table_reference(table: &TableRef) -> Result<String, OrmError> {
    match table.alias {
        Some(alias) => quote_identifier(alias),
        None => quote_table_ref(table),
    }
}

pub fn quote_column_ref(column: &ColumnRef) -> Result<String, OrmError> {
    Ok(format!(
        "{}.{}",
        quote_table_reference(&column.table)?,
        quote_identifier(column.column_name)?,
    ))
}

fn validate_identifier(identifier: &str) -> Result<(), OrmError> {
    if identifier.is_empty() {
        return Err(OrmError::compile("SQL Server identifier cannot be empty"));
    }

    if identifier.contains('.') {
        return Err(OrmError::compile(
            "SQL Server identifier cannot contain '.'; quote each part separately",
        ));
    }

    if identifier.chars().any(|ch| ch.is_control()) {
        return Err(OrmError::compile(
            "SQL Server identifier cannot contain control characters",
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        quote_column_ref, quote_identifier, quote_qualified_identifier, quote_table_ref,
        quote_table_reference, quote_table_source,
    };
    use sql_orm_core::OrmErrorKind;
    use sql_orm_query::{ColumnRef, TableRef};

    #[test]
    fn quotes_simple_identifier_with_brackets() {
        assert_eq!(quote_identifier("customers").unwrap(), "[customers]");
    }

    #[test]
    fn escapes_closing_brackets_inside_identifier() {
        assert_eq!(
            quote_identifier("report]archive").unwrap(),
            "[report]]archive]"
        );
    }

    #[test]
    fn rejects_empty_identifier() {
        let error = quote_identifier("").unwrap_err();

        assert_eq!(error.kind(), OrmErrorKind::Compile);
        assert_eq!(error.message(), "SQL Server identifier cannot be empty");
    }

    #[test]
    fn rejects_control_characters() {
        let error = quote_identifier("line\nbreak").unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server identifier cannot contain control characters"
        );
    }

    #[test]
    fn rejects_multipart_identifier_in_single_segment_api() {
        let error = quote_identifier("dbo.customers").unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server identifier cannot contain '.'; quote each part separately"
        );
    }

    #[test]
    fn quotes_schema_qualified_identifier() {
        assert_eq!(
            quote_qualified_identifier("sales", "customers").unwrap(),
            "[sales].[customers]"
        );
    }

    #[test]
    fn quotes_table_and_column_refs_from_ast() {
        let table = TableRef::new("sales", "customers");
        let column = ColumnRef::new(table, "email", "email");

        assert_eq!(quote_table_ref(&table).unwrap(), "[sales].[customers]");
        assert_eq!(
            quote_column_ref(&column).unwrap(),
            "[sales].[customers].[email]"
        );
    }

    #[test]
    fn quotes_aliased_table_sources_and_column_refs_from_ast() {
        let table = TableRef::with_alias("sales", "customers", "c");
        let column = ColumnRef::new(table, "email", "email");

        assert_eq!(
            quote_table_source(&table).unwrap(),
            "[sales].[customers] AS [c]"
        );
        assert_eq!(quote_table_reference(&table).unwrap(), "[c]");
        assert_eq!(quote_column_ref(&column).unwrap(), "[c].[email]");
    }
}
