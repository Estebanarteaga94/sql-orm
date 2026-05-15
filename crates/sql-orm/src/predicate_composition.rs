use sql_orm_query::Predicate;

pub trait PredicateCompositionExt {
    fn and(self, other: Predicate) -> Predicate;

    fn or(self, other: Predicate) -> Predicate;

    fn not(self) -> Predicate;
}

impl PredicateCompositionExt for Predicate {
    fn and(self, other: Predicate) -> Predicate {
        match (self, other) {
            (Predicate::And(mut left), Predicate::And(right)) => {
                left.extend(right);
                Predicate::and(left)
            }
            (Predicate::And(mut left), right) => {
                left.push(right);
                Predicate::and(left)
            }
            (left, Predicate::And(mut right)) => {
                let mut predicates = vec![left];
                predicates.append(&mut right);
                Predicate::and(predicates)
            }
            (left, right) => Predicate::and(vec![left, right]),
        }
    }

    fn or(self, other: Predicate) -> Predicate {
        match (self, other) {
            (Predicate::Or(mut left), Predicate::Or(right)) => {
                left.extend(right);
                Predicate::or(left)
            }
            (Predicate::Or(mut left), right) => {
                left.push(right);
                Predicate::or(left)
            }
            (left, Predicate::Or(mut right)) => {
                let mut predicates = vec![left];
                predicates.append(&mut right);
                Predicate::or(predicates)
            }
            (left, right) => Predicate::or(vec![left, right]),
        }
    }

    fn not(self) -> Predicate {
        Predicate::negate(self)
    }
}

#[cfg(test)]
mod tests {
    use super::PredicateCompositionExt;
    use sql_orm_core::SqlValue;
    use sql_orm_query::{Expr, Predicate};

    #[test]
    fn and_combines_two_predicates() {
        let left = Predicate::eq(
            Expr::value(SqlValue::Bool(true)),
            Expr::value(SqlValue::Bool(true)),
        );
        let right = Predicate::gt(
            Expr::value(SqlValue::I64(10)),
            Expr::value(SqlValue::I64(5)),
        );

        assert_eq!(
            left.clone().and(right.clone()),
            Predicate::and(vec![left, right])
        );
    }

    #[test]
    fn and_flattens_existing_and_groups() {
        let first = Predicate::eq(Expr::value(SqlValue::I64(1)), Expr::value(SqlValue::I64(1)));
        let second = Predicate::eq(Expr::value(SqlValue::I64(2)), Expr::value(SqlValue::I64(2)));
        let third = Predicate::eq(Expr::value(SqlValue::I64(3)), Expr::value(SqlValue::I64(3)));

        assert_eq!(
            Predicate::and(vec![first.clone(), second.clone()]).and(third.clone()),
            Predicate::and(vec![first, second, third])
        );
    }

    #[test]
    fn or_combines_and_flattens_predicates() {
        let first = Predicate::eq(Expr::value(SqlValue::I64(1)), Expr::value(SqlValue::I64(1)));
        let second = Predicate::eq(Expr::value(SqlValue::I64(2)), Expr::value(SqlValue::I64(2)));
        let third = Predicate::eq(Expr::value(SqlValue::I64(3)), Expr::value(SqlValue::I64(3)));

        assert_eq!(
            Predicate::or(vec![first.clone(), second.clone()]).or(third.clone()),
            Predicate::or(vec![first, second, third])
        );
    }

    #[test]
    fn not_wraps_predicate_in_negation() {
        let predicate = Predicate::eq(
            Expr::value(SqlValue::Bool(true)),
            Expr::value(SqlValue::Bool(false)),
        );

        assert_eq!(predicate.clone().not(), Predicate::negate(predicate));
    }
}
