use crate::expr::Expr;

#[derive(Debug, Clone, PartialEq)]
pub enum Predicate {
    Eq(Expr, Expr),
    Ne(Expr, Expr),
    Gt(Expr, Expr),
    Gte(Expr, Expr),
    Lt(Expr, Expr),
    Lte(Expr, Expr),
    Like(Expr, Expr),
    IsNull(Expr),
    IsNotNull(Expr),
    And(Vec<Predicate>),
    Or(Vec<Predicate>),
    Not(Box<Predicate>),
}

impl Predicate {
    pub const fn eq(left: Expr, right: Expr) -> Self {
        Self::Eq(left, right)
    }

    pub const fn ne(left: Expr, right: Expr) -> Self {
        Self::Ne(left, right)
    }

    pub const fn gt(left: Expr, right: Expr) -> Self {
        Self::Gt(left, right)
    }

    pub const fn gte(left: Expr, right: Expr) -> Self {
        Self::Gte(left, right)
    }

    pub const fn lt(left: Expr, right: Expr) -> Self {
        Self::Lt(left, right)
    }

    pub const fn lte(left: Expr, right: Expr) -> Self {
        Self::Lte(left, right)
    }

    pub const fn like(left: Expr, right: Expr) -> Self {
        Self::Like(left, right)
    }

    pub const fn is_null(expr: Expr) -> Self {
        Self::IsNull(expr)
    }

    pub const fn is_not_null(expr: Expr) -> Self {
        Self::IsNotNull(expr)
    }

    pub fn and(predicates: Vec<Predicate>) -> Self {
        Self::And(predicates)
    }

    pub fn or(predicates: Vec<Predicate>) -> Self {
        Self::Or(predicates)
    }

    pub fn negate(predicate: Predicate) -> Self {
        Self::Not(Box::new(predicate))
    }
}
