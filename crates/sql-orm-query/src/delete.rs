use crate::expr::TableRef;
use crate::predicate::Predicate;
use sql_orm_core::Entity;

#[derive(Debug, Clone, PartialEq)]
pub struct DeleteQuery {
    pub from: TableRef,
    pub predicate: Option<Predicate>,
}

impl DeleteQuery {
    pub fn from_entity<E: Entity>() -> Self {
        Self {
            from: TableRef::for_entity::<E>(),
            predicate: None,
        }
    }

    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.predicate = Some(match self.predicate.take() {
            Some(existing) => Predicate::and(vec![existing, predicate]),
            None => predicate,
        });
        self
    }
}
