//! SQL Server compilation layer.

mod compiler;
mod migration;
mod quoting;

use sql_orm_core::CrateIdentity;

/// Placeholder compiler marker for the SQL Server dialect.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SqlServerCompiler;

pub use quoting::{
    quote_column_ref, quote_identifier, quote_qualified_identifier, quote_table_ref,
};

pub const CRATE_IDENTITY: CrateIdentity = CrateIdentity {
    name: "sql-orm-sqlserver",
    responsibility: "AST compilation and SQL Server specific quoting and SQL emission",
};

#[cfg(test)]
mod tests {
    use super::{CRATE_IDENTITY, SqlServerCompiler, quote_identifier};

    #[test]
    fn declares_sqlserver_compilation_boundary() {
        let compiler = SqlServerCompiler;
        assert_eq!(compiler, SqlServerCompiler);
        assert!(CRATE_IDENTITY.responsibility.contains("SQL emission"));
    }

    #[test]
    fn reexports_identifier_quoting() {
        assert_eq!(quote_identifier("users").unwrap(), "[users]");
    }
}
