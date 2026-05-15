use crate::expr::{Expr, TableRef};
use crate::join::Join;
use crate::order::OrderBy;
use crate::pagination::Pagination;
use crate::predicate::Predicate;
use sql_orm_core::{Entity, EntityColumn};

#[derive(Debug, Clone, PartialEq)]
pub struct SelectProjection {
    pub expr: Expr,
    pub alias: Option<&'static str>,
}

impl SelectProjection {
    pub fn column<E: Entity>(column: EntityColumn<E>) -> Self {
        let alias = column.column_name();
        Self {
            expr: Expr::from(column),
            alias: Some(alias),
        }
    }

    pub fn expr(expr: Expr) -> Self {
        let alias = match &expr {
            Expr::Column(column) => Some(column.column_name),
            _ => None,
        };

        Self { expr, alias }
    }

    pub fn expr_as(expr: Expr, alias: &'static str) -> Self {
        Self {
            expr,
            alias: Some(alias),
        }
    }
}

impl<E: Entity> From<EntityColumn<E>> for SelectProjection {
    fn from(value: EntityColumn<E>) -> Self {
        Self::column(value)
    }
}

impl From<Expr> for SelectProjection {
    fn from(value: Expr) -> Self {
        Self::expr(value)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct SelectQuery {
    pub from: TableRef,
    pub joins: Vec<Join>,
    pub projection: Vec<SelectProjection>,
    pub predicate: Option<Predicate>,
    pub order_by: Vec<OrderBy>,
    pub pagination: Option<Pagination>,
}

impl SelectQuery {
    pub fn from_entity<E: Entity>() -> Self {
        Self {
            from: TableRef::for_entity::<E>(),
            joins: Vec::new(),
            projection: Vec::new(),
            predicate: None,
            order_by: Vec::new(),
            pagination: None,
        }
    }

    pub fn from_entity_as<E: Entity>(alias: &'static str) -> Self {
        Self {
            from: TableRef::for_entity_as::<E>(alias),
            joins: Vec::new(),
            projection: Vec::new(),
            predicate: None,
            order_by: Vec::new(),
            pagination: None,
        }
    }

    pub fn select<P, I>(mut self, projection: I) -> Self
    where
        P: Into<SelectProjection>,
        I: IntoIterator<Item = P>,
    {
        self.projection = projection.into_iter().map(Into::into).collect();
        self
    }

    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.predicate = Some(match self.predicate.take() {
            Some(existing) => Predicate::and(vec![existing, predicate]),
            None => predicate,
        });
        self
    }

    pub fn join(mut self, join: Join) -> Self {
        self.joins.push(join);
        self
    }

    pub fn inner_join<E: Entity>(self, on: Predicate) -> Self {
        self.join(Join::inner_entity::<E>(on))
    }

    pub fn left_join<E: Entity>(self, on: Predicate) -> Self {
        self.join(Join::left_entity::<E>(on))
    }

    pub fn inner_join_as<E: Entity>(self, alias: &'static str, on: Predicate) -> Self {
        self.join(Join::inner_entity_as::<E>(alias, on))
    }

    pub fn left_join_as<E: Entity>(self, alias: &'static str, on: Predicate) -> Self {
        self.join(Join::left_entity_as::<E>(alias, on))
    }

    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.order_by.push(order);
        self
    }

    pub fn paginate(mut self, pagination: Pagination) -> Self {
        self.pagination = Some(pagination);
        self
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CountQuery {
    pub from: TableRef,
    pub predicate: Option<Predicate>,
}

impl CountQuery {
    pub fn from_entity<E: Entity>() -> Self {
        Self {
            from: TableRef::for_entity::<E>(),
            predicate: None,
        }
    }

    pub fn from_entity_as<E: Entity>(alias: &'static str) -> Self {
        Self {
            from: TableRef::for_entity_as::<E>(alias),
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
