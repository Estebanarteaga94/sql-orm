use crate::expr::TableRef;
use crate::predicate::Predicate;
use sql_orm_core::Entity;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JoinType {
    Inner,
    Left,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Join {
    pub join_type: JoinType,
    pub table: TableRef,
    pub on: Predicate,
}

impl Join {
    pub const fn new(join_type: JoinType, table: TableRef, on: Predicate) -> Self {
        Self {
            join_type,
            table,
            on,
        }
    }

    pub fn inner(table: TableRef, on: Predicate) -> Self {
        Self::new(JoinType::Inner, table, on)
    }

    pub fn left(table: TableRef, on: Predicate) -> Self {
        Self::new(JoinType::Left, table, on)
    }

    pub fn inner_entity<E: Entity>(on: Predicate) -> Self {
        Self::inner(TableRef::for_entity::<E>(), on)
    }

    pub fn left_entity<E: Entity>(on: Predicate) -> Self {
        Self::left(TableRef::for_entity::<E>(), on)
    }

    pub fn inner_entity_as<E: Entity>(alias: &'static str, on: Predicate) -> Self {
        Self::inner(TableRef::for_entity_as::<E>(alias), on)
    }

    pub fn left_entity_as<E: Entity>(alias: &'static str, on: Predicate) -> Self {
        Self::left(TableRef::for_entity_as::<E>(alias), on)
    }
}
