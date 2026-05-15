use crate::expr::TableRef;
use sql_orm_core::{ColumnValue, Entity, Insertable};

#[derive(Debug, Clone, PartialEq)]
pub struct InsertQuery {
    pub into: TableRef,
    pub values: Vec<ColumnValue>,
}

impl InsertQuery {
    pub fn for_entity<E: Entity, I: Insertable<E>>(insertable: &I) -> Self {
        Self {
            into: TableRef::for_entity::<E>(),
            values: insertable.values(),
        }
    }
}
