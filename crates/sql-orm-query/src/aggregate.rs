use crate::expr::{Expr, TableRef};
use crate::join::Join;
use crate::order::SortDirection;
use crate::pagination::Pagination;
use crate::predicate::Predicate;
use sql_orm_core::{Entity, EntityColumn};

#[derive(Debug, Clone, PartialEq)]
pub enum AggregateExpr {
    GroupKey(Expr),
    CountAll,
    Count(Expr),
    Sum(Expr),
    Avg(Expr),
    Min(Expr),
    Max(Expr),
}

impl AggregateExpr {
    pub fn group_key(expr: impl Into<Expr>) -> Self {
        Self::GroupKey(expr.into())
    }

    pub const fn count_all() -> Self {
        Self::CountAll
    }

    pub fn count(expr: impl Into<Expr>) -> Self {
        Self::Count(expr.into())
    }

    pub fn sum(expr: impl Into<Expr>) -> Self {
        Self::Sum(expr.into())
    }

    pub fn avg(expr: impl Into<Expr>) -> Self {
        Self::Avg(expr.into())
    }

    pub fn min(expr: impl Into<Expr>) -> Self {
        Self::Min(expr.into())
    }

    pub fn max(expr: impl Into<Expr>) -> Self {
        Self::Max(expr.into())
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AggregateProjection {
    pub expr: AggregateExpr,
    pub alias: Option<&'static str>,
}

impl AggregateProjection {
    pub fn group_key<E: Entity>(column: EntityColumn<E>) -> Self {
        let alias = column.column_name();
        Self {
            expr: AggregateExpr::GroupKey(Expr::from(column)),
            alias: Some(alias),
        }
    }

    pub fn group_key_as(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self {
            expr: AggregateExpr::GroupKey(expr.into()),
            alias: Some(alias),
        }
    }

    pub fn expr(expr: AggregateExpr) -> Self {
        Self { expr, alias: None }
    }

    pub fn expr_as(expr: AggregateExpr, alias: &'static str) -> Self {
        Self {
            expr,
            alias: Some(alias),
        }
    }

    pub fn count_as(alias: &'static str) -> Self {
        Self::expr_as(AggregateExpr::CountAll, alias)
    }

    pub fn sum_as(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::expr_as(AggregateExpr::sum(expr), alias)
    }

    pub fn avg_as(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::expr_as(AggregateExpr::avg(expr), alias)
    }

    pub fn min_as(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::expr_as(AggregateExpr::min(expr), alias)
    }

    pub fn max_as(expr: impl Into<Expr>, alias: &'static str) -> Self {
        Self::expr_as(AggregateExpr::max(expr), alias)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AggregatePredicate {
    Eq(AggregateExpr, Expr),
    Ne(AggregateExpr, Expr),
    Gt(AggregateExpr, Expr),
    Gte(AggregateExpr, Expr),
    Lt(AggregateExpr, Expr),
    Lte(AggregateExpr, Expr),
    And(Vec<AggregatePredicate>),
    Or(Vec<AggregatePredicate>),
    Not(Box<AggregatePredicate>),
}

impl AggregatePredicate {
    pub fn eq(left: AggregateExpr, right: impl Into<Expr>) -> Self {
        Self::Eq(left, right.into())
    }

    pub fn ne(left: AggregateExpr, right: impl Into<Expr>) -> Self {
        Self::Ne(left, right.into())
    }

    pub fn gt(left: AggregateExpr, right: impl Into<Expr>) -> Self {
        Self::Gt(left, right.into())
    }

    pub fn gte(left: AggregateExpr, right: impl Into<Expr>) -> Self {
        Self::Gte(left, right.into())
    }

    pub fn lt(left: AggregateExpr, right: impl Into<Expr>) -> Self {
        Self::Lt(left, right.into())
    }

    pub fn lte(left: AggregateExpr, right: impl Into<Expr>) -> Self {
        Self::Lte(left, right.into())
    }

    pub fn and(predicates: Vec<AggregatePredicate>) -> Self {
        Self::And(predicates)
    }

    pub fn or(predicates: Vec<AggregatePredicate>) -> Self {
        Self::Or(predicates)
    }

    pub fn negate(predicate: AggregatePredicate) -> Self {
        Self::Not(Box::new(predicate))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AggregateOrderBy {
    pub expr: AggregateExpr,
    pub direction: SortDirection,
}

impl AggregateOrderBy {
    pub fn new(expr: AggregateExpr, direction: SortDirection) -> Self {
        Self { expr, direction }
    }

    pub fn asc(expr: AggregateExpr) -> Self {
        Self::new(expr, SortDirection::Asc)
    }

    pub fn desc(expr: AggregateExpr) -> Self {
        Self::new(expr, SortDirection::Desc)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AggregateQuery {
    pub from: TableRef,
    pub joins: Vec<Join>,
    pub projection: Vec<AggregateProjection>,
    pub predicate: Option<Predicate>,
    pub group_by: Vec<Expr>,
    pub having: Option<AggregatePredicate>,
    pub order_by: Vec<AggregateOrderBy>,
    pub pagination: Option<Pagination>,
}

impl AggregateQuery {
    pub fn from_entity<E: Entity>() -> Self {
        Self {
            from: TableRef::for_entity::<E>(),
            joins: Vec::new(),
            projection: Vec::new(),
            predicate: None,
            group_by: Vec::new(),
            having: None,
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
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            pagination: None,
        }
    }

    pub fn project<P, I>(mut self, projection: I) -> Self
    where
        P: Into<AggregateProjection>,
        I: IntoIterator<Item = P>,
    {
        self.projection = projection.into_iter().map(Into::into).collect();
        self
    }

    pub fn group_by<G, I>(mut self, group_by: I) -> Self
    where
        G: Into<Expr>,
        I: IntoIterator<Item = G>,
    {
        self.group_by = group_by.into_iter().map(Into::into).collect();
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

    pub fn having(mut self, predicate: AggregatePredicate) -> Self {
        self.having = Some(match self.having.take() {
            Some(existing) => AggregatePredicate::and(vec![existing, predicate]),
            None => predicate,
        });
        self
    }

    pub fn order_by(mut self, order: AggregateOrderBy) -> Self {
        self.order_by.push(order);
        self
    }

    pub fn paginate(mut self, pagination: Pagination) -> Self {
        self.pagination = Some(pagination);
        self
    }
}

impl From<AggregateExpr> for AggregateProjection {
    fn from(value: AggregateExpr) -> Self {
        Self::expr(value)
    }
}
