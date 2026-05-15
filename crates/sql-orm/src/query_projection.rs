use crate::AliasedEntityColumn;
use sql_orm_core::{Entity, EntityColumn};
use sql_orm_query::{Expr, SelectProjection};

pub trait SelectProjections {
    fn into_select_projections(self) -> Vec<SelectProjection>;
}

impl<P> SelectProjections for Vec<P>
where
    P: Into<SelectProjection>,
{
    fn into_select_projections(self) -> Vec<SelectProjection> {
        self.into_iter().map(Into::into).collect()
    }
}

impl<P, const N: usize> SelectProjections for [P; N]
where
    P: Into<SelectProjection>,
{
    fn into_select_projections(self) -> Vec<SelectProjection> {
        self.into_iter().map(Into::into).collect()
    }
}

impl SelectProjections for SelectProjection {
    fn into_select_projections(self) -> Vec<SelectProjection> {
        vec![self]
    }
}

impl SelectProjections for Expr {
    fn into_select_projections(self) -> Vec<SelectProjection> {
        vec![SelectProjection::from(self)]
    }
}

impl<E> SelectProjections for EntityColumn<E>
where
    E: Entity,
{
    fn into_select_projections(self) -> Vec<SelectProjection> {
        vec![SelectProjection::from(self)]
    }
}

impl<E> SelectProjections for AliasedEntityColumn<E>
where
    E: Entity,
{
    fn into_select_projections(self) -> Vec<SelectProjection> {
        vec![SelectProjection::from(self)]
    }
}

macro_rules! impl_select_projections_tuple {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> SelectProjections for ($($name,)+)
        where
            $($name: Into<SelectProjection>),+
        {
            #[allow(non_snake_case)]
            fn into_select_projections(self) -> Vec<SelectProjection> {
                let ($($name,)+) = self;
                vec![$($name.into()),+]
            }
        }
    };
}

impl_select_projections_tuple!(A);
impl_select_projections_tuple!(A, B);
impl_select_projections_tuple!(A, B, C);
impl_select_projections_tuple!(A, B, C, D);
impl_select_projections_tuple!(A, B, C, D, E);
impl_select_projections_tuple!(A, B, C, D, E, F);
impl_select_projections_tuple!(A, B, C, D, E, F, G);
impl_select_projections_tuple!(A, B, C, D, E, F, G, H);
impl_select_projections_tuple!(A, B, C, D, E, F, G, H, I);
impl_select_projections_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_select_projections_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_select_projections_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);
