use crate::expr::{ColumnRef, TableRef};
use sql_orm_core::{Entity, EntityColumn};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderBy {
    pub table: TableRef,
    pub column_name: &'static str,
    pub direction: SortDirection,
}

impl OrderBy {
    pub const fn new(table: TableRef, column_name: &'static str, direction: SortDirection) -> Self {
        Self {
            table,
            column_name,
            direction,
        }
    }

    pub fn asc<E: Entity>(column: EntityColumn<E>) -> Self {
        let column_ref = ColumnRef::for_entity_column(column);
        Self::new(column_ref.table, column_ref.column_name, SortDirection::Asc)
    }

    pub fn desc<E: Entity>(column: EntityColumn<E>) -> Self {
        let column_ref = ColumnRef::for_entity_column(column);
        Self::new(
            column_ref.table,
            column_ref.column_name,
            SortDirection::Desc,
        )
    }
}
