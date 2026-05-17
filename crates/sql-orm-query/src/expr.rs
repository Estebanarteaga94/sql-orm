use sql_orm_core::{Entity, EntityColumn, SqlValue};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TableRef {
    pub schema: &'static str,
    pub table: &'static str,
    pub alias: Option<&'static str>,
}

impl TableRef {
    pub const fn new(schema: &'static str, table: &'static str) -> Self {
        Self {
            schema,
            table,
            alias: None,
        }
    }

    pub const fn with_alias(
        schema: &'static str,
        table: &'static str,
        alias: &'static str,
    ) -> Self {
        Self {
            schema,
            table,
            alias: Some(alias),
        }
    }

    pub fn for_entity<E: Entity>() -> Self {
        let metadata = E::metadata();
        Self::new(metadata.schema, metadata.table)
    }

    pub fn for_entity_as<E: Entity>(alias: &'static str) -> Self {
        let metadata = E::metadata();
        Self::with_alias(metadata.schema, metadata.table, alias)
    }

    pub const fn as_alias(self, alias: &'static str) -> Self {
        Self {
            schema: self.schema,
            table: self.table,
            alias: Some(alias),
        }
    }

    pub const fn without_alias(self) -> Self {
        Self {
            schema: self.schema,
            table: self.table,
            alias: None,
        }
    }

    pub const fn reference_name(&self) -> &'static str {
        match self.alias {
            Some(alias) => alias,
            None => self.table,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnRef {
    pub table: TableRef,
    pub rust_field: &'static str,
    pub column_name: &'static str,
}

impl ColumnRef {
    pub const fn new(table: TableRef, rust_field: &'static str, column_name: &'static str) -> Self {
        Self {
            table,
            rust_field,
            column_name,
        }
    }

    pub fn for_entity_column<E: Entity>(column: EntityColumn<E>) -> Self {
        Self::new(
            TableRef::for_entity::<E>(),
            column.rust_field(),
            column.column_name(),
        )
    }

    pub fn for_entity_column_as<E: Entity>(column: EntityColumn<E>, alias: &'static str) -> Self {
        Self::new(
            TableRef::for_entity_as::<E>(alias),
            column.rust_field(),
            column.column_name(),
        )
    }

    pub const fn with_table_alias(self, alias: &'static str) -> Self {
        Self {
            table: self.table.as_alias(alias),
            rust_field: self.rust_field,
            column_name: self.column_name,
        }
    }
}

impl<E: Entity> From<EntityColumn<E>> for ColumnRef {
    fn from(value: EntityColumn<E>) -> Self {
        Self::for_entity_column(value)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinaryOp {
    Add,
    Subtract,
    Multiply,
    Divide,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnaryOp {
    Negate,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlFunction {
    Lower,
    Upper,
    Year,
    Month,
    Day,
    Len,
    Trim,
}

impl SqlFunction {
    pub const fn sql_name(self) -> &'static str {
        match self {
            Self::Lower => "LOWER",
            Self::Upper => "UPPER",
            Self::Year => "YEAR",
            Self::Month => "MONTH",
            Self::Day => "DAY",
            Self::Len => "LEN",
            Self::Trim => "TRIM",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Expr {
    Column(ColumnRef),
    Value(SqlValue),
    Binary {
        left: Box<Expr>,
        op: BinaryOp,
        right: Box<Expr>,
    },
    Unary {
        op: UnaryOp,
        expr: Box<Expr>,
    },
    Function {
        function: SqlFunction,
        args: Vec<Expr>,
    },
    UnsafeFunction {
        name: String,
        args: Vec<Expr>,
    },
}

impl Expr {
    pub fn column<E: Entity>(column: EntityColumn<E>) -> Self {
        Self::Column(column.into())
    }

    pub fn column_as<E: Entity>(column: EntityColumn<E>, alias: &'static str) -> Self {
        Self::Column(ColumnRef::for_entity_column_as(column, alias))
    }

    pub const fn value(value: SqlValue) -> Self {
        Self::Value(value)
    }

    pub fn binary(left: Expr, op: BinaryOp, right: Expr) -> Self {
        Self::Binary {
            left: Box::new(left),
            op,
            right: Box::new(right),
        }
    }

    pub fn unary(op: UnaryOp, expr: Expr) -> Self {
        Self::Unary {
            op,
            expr: Box::new(expr),
        }
    }

    pub fn function(function: SqlFunction, args: Vec<Expr>) -> Self {
        Self::Function { function, args }
    }

    pub fn unsafe_function(name: impl Into<String>, args: Vec<Expr>) -> Self {
        Self::UnsafeFunction {
            name: name.into(),
            args,
        }
    }
}

impl<E: Entity> From<EntityColumn<E>> for Expr {
    fn from(value: EntityColumn<E>) -> Self {
        Self::column(value)
    }
}

impl From<SqlValue> for Expr {
    fn from(value: SqlValue) -> Self {
        Self::Value(value)
    }
}
