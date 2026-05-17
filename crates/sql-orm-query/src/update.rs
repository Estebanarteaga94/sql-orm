use crate::expr::TableRef;
use crate::predicate::Predicate;
use sql_orm_core::{Changeset, ColumnValue, Entity};

#[derive(Debug, Clone, PartialEq)]
pub struct UpdateQuery {
    pub table: TableRef,
    pub changes: Vec<ColumnValue>,
    pub predicate: Option<Predicate>,
    pub allow_all_rows: bool,
}

impl UpdateQuery {
    pub fn for_entity<E: Entity, C: Changeset<E>>(changeset: &C) -> Self {
        Self {
            table: TableRef::for_entity::<E>(),
            changes: changeset.changes(),
            predicate: None,
            allow_all_rows: false,
        }
    }

    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.predicate = Some(match self.predicate.take() {
            Some(existing) => Predicate::and(vec![existing, predicate]),
            None => predicate,
        });
        self
    }

    pub const fn allow_all_rows(mut self) -> Self {
        self.allow_all_rows = true;
        self
    }
}
