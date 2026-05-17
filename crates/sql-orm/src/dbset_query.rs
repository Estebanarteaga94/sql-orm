use crate::context::{ActiveTenant, SharedConnection};
use crate::page_request::PageRequest;
use crate::query_alias::AliasedEntityColumn;
use crate::query_projection::SelectProjections;
use crate::{
    IncludeCollection, IncludeNavigation, SoftDeleteEntity, TenantScopedEntity,
    TrackingRegistryHandle,
};
use sql_orm_core::{
    ColumnMetadata, Entity, EntityColumn, EntityMetadata, FromRow, NavigationKind, OrmError, Row,
    SqlServerType, SqlTypeMapping, SqlValue,
};
use sql_orm_query::{
    AggregateExpr, AggregateOrderBy, AggregatePredicate, AggregateProjection, AggregateQuery,
    ColumnRef, ExistsQuery, Expr, Join, JoinType, OrderBy, Pagination, Predicate, SelectProjection,
    SelectQuery, TableRef,
};
use sql_orm_sqlserver::SqlServerCompiler;

#[derive(Clone)]
/// Fluent query builder bound to one `DbSet<E>`.
///
/// `DbSetQuery` stores query intent as AST until execution. SQL text is
/// generated only by the SQL Server compiler. Mandatory runtime policies such
/// as tenant filtering and root-entity soft-delete visibility are applied when
/// the query is compiled or executed.
pub struct DbSetQuery<E: Entity> {
    connection: Option<SharedConnection>,
    active_tenant: Option<ActiveTenant>,
    tracking_registry: Option<TrackingRegistryHandle>,
    select_query: SelectQuery,
    visibility: SoftDeleteVisibility,
    _entity: core::marker::PhantomData<fn() -> E>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SoftDeleteVisibility {
    Default,
    WithDeleted,
    OnlyDeleted,
}

/// Loading strategy for a `has_many` collection include.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionIncludeStrategy {
    /// Load roots and related rows through one `LEFT JOIN`, then group joined
    /// rows by the root primary key.
    Join,
    /// Planned split-query strategy for large collections.
    ///
    /// The strategy is explicit in the public API, but execution returns a
    /// clear error until the split-query implementation lands.
    SplitQuery,
}

const DEFAULT_INCLUDE_MANY_JOIN_ROW_LIMIT: usize = 10_000;

impl<E: Entity> DbSetQuery<E> {
    pub(crate) fn new(connection: Option<SharedConnection>, select_query: SelectQuery) -> Self {
        let active_tenant = connection
            .as_ref()
            .and_then(SharedConnection::active_tenant);
        Self {
            connection,
            active_tenant,
            tracking_registry: None,
            select_query,
            visibility: SoftDeleteVisibility::Default,
            _entity: core::marker::PhantomData,
        }
    }

    pub(crate) fn with_tracking_registry(
        mut self,
        tracking_registry: TrackingRegistryHandle,
    ) -> Self {
        self.tracking_registry = Some(tracking_registry);
        self
    }

    #[cfg(test)]
    pub(crate) fn with_active_tenant_for_test(mut self, active_tenant: ActiveTenant) -> Self {
        self.active_tenant = Some(active_tenant);
        self
    }

    /// Replaces the underlying `SelectQuery` AST while keeping this query bound
    /// to the same connection and runtime policies.
    pub fn with_select_query(mut self, select_query: SelectQuery) -> Self {
        self.select_query = select_query;
        self
    }

    /// Adds a predicate to the query.
    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.select_query = self.select_query.filter(predicate);
        self
    }

    /// Adds an explicit join described by the query AST.
    pub fn join(mut self, join: Join) -> Self {
        self.select_query = self.select_query.join(join);
        self
    }

    /// Adds an explicit `INNER JOIN` to another entity.
    pub fn inner_join<J: Entity>(mut self, on: Predicate) -> Self {
        self.select_query = self.select_query.inner_join::<J>(on);
        self
    }

    /// Adds an explicit `LEFT JOIN` to another entity.
    pub fn left_join<J: Entity>(mut self, on: Predicate) -> Self {
        self.select_query = self.select_query.left_join::<J>(on);
        self
    }

    /// Adds an `INNER JOIN` inferred from navigation metadata.
    ///
    /// The navigation must be declared on the root entity `E`, and its target
    /// table must match `J`. This only builds the SQL join; it does not load or
    /// materialize the related entity.
    pub fn try_inner_join_navigation<J: Entity>(
        self,
        navigation: &'static str,
    ) -> Result<Self, OrmError> {
        self.try_join_navigation::<J>(navigation, JoinType::Inner, None)
    }

    /// Adds a `LEFT JOIN` inferred from navigation metadata.
    ///
    /// The navigation must be declared on the root entity `E`, and its target
    /// table must match `J`. This only builds the SQL join; it does not load or
    /// materialize the related entity.
    pub fn try_left_join_navigation<J: Entity>(
        self,
        navigation: &'static str,
    ) -> Result<Self, OrmError> {
        self.try_join_navigation::<J>(navigation, JoinType::Left, None)
    }

    /// Adds an aliased `INNER JOIN` inferred from navigation metadata.
    pub fn try_inner_join_navigation_as<J: Entity>(
        self,
        navigation: &'static str,
        alias: &'static str,
    ) -> Result<Self, OrmError> {
        self.try_join_navigation::<J>(navigation, JoinType::Inner, Some(alias))
    }

    /// Adds an aliased `LEFT JOIN` inferred from navigation metadata.
    pub fn try_left_join_navigation_as<J: Entity>(
        self,
        navigation: &'static str,
        alias: &'static str,
    ) -> Result<Self, OrmError> {
        self.try_join_navigation::<J>(navigation, JoinType::Left, Some(alias))
    }

    /// Includes a single related entity through a `belongs_to` or `has_one`
    /// navigation.
    ///
    /// This first eager-loading cut uses a left join and materializes the
    /// related row into `Navigation<J>`. Collection navigations (`has_many`)
    /// are intentionally rejected because they need grouping or split-query
    /// semantics.
    pub fn include<J: Entity>(
        self,
        navigation: &'static str,
    ) -> Result<DbSetQueryIncludeOne<E, J>, OrmError> {
        self.include_as::<J>(navigation, navigation)
    }

    /// Includes a single related entity using an explicit table alias.
    pub fn include_as<J: Entity>(
        self,
        navigation: &'static str,
        alias: &'static str,
    ) -> Result<DbSetQueryIncludeOne<E, J>, OrmError> {
        let metadata = E::metadata();
        let navigation_metadata = metadata.navigation(navigation).ok_or_else(|| {
            OrmError::new(format!(
                "entity `{}` does not declare navigation `{}`",
                metadata.rust_name, navigation
            ))
        })?;

        if !matches!(
            navigation_metadata.kind,
            NavigationKind::BelongsTo | NavigationKind::HasOne
        ) {
            return Err(OrmError::new(format!(
                "include only supports belongs_to and has_one navigations; `{}` is {:?}",
                navigation_metadata.rust_field, navigation_metadata.kind
            )));
        }

        Ok(DbSetQueryIncludeOne {
            query: self.try_join_navigation::<J>(navigation, JoinType::Left, Some(alias))?,
            navigation,
            alias,
            _target: core::marker::PhantomData,
        })
    }

    /// Includes a collection navigation through a `has_many` relationship.
    ///
    /// This first collection include cut uses a left join, materializes joined
    /// rows, then groups them by the root entity primary key before assigning
    /// `Collection<J>`. Pagination is rejected for this join-based path
    /// because limiting joined rows is not equivalent to limiting root
    /// entities.
    pub fn include_many<J: Entity>(
        self,
        navigation: &'static str,
    ) -> Result<DbSetQueryIncludeMany<E, J>, OrmError> {
        self.include_many_as::<J>(navigation, navigation)
    }

    /// Includes a collection navigation using an explicit table alias.
    pub fn include_many_as<J: Entity>(
        self,
        navigation: &'static str,
        alias: &'static str,
    ) -> Result<DbSetQueryIncludeMany<E, J>, OrmError> {
        let metadata = E::metadata();
        let navigation_metadata = metadata.navigation(navigation).ok_or_else(|| {
            OrmError::new(format!(
                "entity `{}` does not declare navigation `{}`",
                metadata.rust_name, navigation
            ))
        })?;

        if !matches!(navigation_metadata.kind, NavigationKind::HasMany) {
            return Err(OrmError::new(format!(
                "include_many only supports has_many navigations; `{}` is {:?}",
                navigation_metadata.rust_field, navigation_metadata.kind
            )));
        }

        Ok(DbSetQueryIncludeMany {
            query: self.try_join_navigation::<J>(navigation, JoinType::Left, Some(alias))?,
            navigation,
            alias,
            strategy: CollectionIncludeStrategy::Join,
            join_row_limit: Some(DEFAULT_INCLUDE_MANY_JOIN_ROW_LIMIT),
            _target: core::marker::PhantomData,
        })
    }

    /// Adds an ordering expression.
    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.select_query = self.select_query.order_by(order);
        self
    }

    /// Limits the number of returned rows with zero offset.
    pub fn limit(mut self, limit: u64) -> Self {
        self.select_query = self.select_query.paginate(Pagination::new(0, limit));
        self
    }

    /// Alias for `limit(...)`.
    pub fn take(self, limit: u64) -> Self {
        self.limit(limit)
    }

    /// Applies page-based pagination.
    pub fn paginate(mut self, request: PageRequest) -> Self {
        self.select_query = self.select_query.paginate(request.to_pagination());
        self
    }

    /// Selects an explicit projection instead of materializing full entities.
    ///
    /// Use `all_as::<T>()` or `first_as::<T>()` to materialize the projection
    /// into a DTO implementing `FromRow`.
    pub fn select<P>(mut self, projection: P) -> Self
    where
        P: SelectProjections,
    {
        self.select_query = self
            .select_query
            .select(projection.into_select_projections());
        self
    }

    /// Starts a grouped aggregate query over one or more group key
    /// expressions.
    ///
    /// The returned builder materializes DTOs through `all_as::<T>()` and
    /// `first_as::<T>()`; it does not expose full-entity materialization.
    pub fn group_by<G>(self, group_by: G) -> Result<DbSetGroupedQuery<E>, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        G: GroupByExpressions,
    {
        let group_by = group_by.into_group_by_expressions();
        if group_by.is_empty() {
            return Err(OrmError::new(
                "group_by requires at least one group key expression",
            ));
        }

        let connection = self.connection.clone();
        let effective = self.effective_select_query()?;
        Ok(DbSetGroupedQuery {
            connection,
            aggregate_query: AggregateQuery {
                from: effective.from,
                joins: effective.joins,
                projection: Vec::new(),
                predicate: effective.predicate,
                group_by,
                having: None,
                order_by: Vec::new(),
                pagination: None,
            },
            _entity: core::marker::PhantomData,
        })
    }

    #[cfg(test)]
    pub(crate) fn select_query(&self) -> &SelectQuery {
        &self.select_query
    }

    /// Includes logically deleted rows for entities with `soft_delete`.
    ///
    /// This affects only the root entity `E`, not every manually joined entity.
    pub fn with_deleted(mut self) -> Self {
        self.visibility = SoftDeleteVisibility::WithDeleted;
        self
    }

    /// Returns only logically deleted rows for entities with `soft_delete`.
    ///
    /// This affects only the root entity `E`, not every manually joined entity.
    pub fn only_deleted(mut self) -> Self {
        self.visibility = SoftDeleteVisibility::OnlyDeleted;
        self
    }

    #[cfg(test)]
    pub(crate) fn into_select_query(self) -> SelectQuery {
        self.select_query
    }

    /// Executes the query and materializes full entities.
    pub async fn all(self) -> Result<Vec<E>, OrmError>
    where
        E: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        let compiled = SqlServerCompiler::compile_select(&self.effective_select_query()?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        connection.fetch_all(compiled).await
    }

    /// Executes the query and materializes the first full entity, if any.
    pub async fn first(self) -> Result<Option<E>, OrmError>
    where
        E: FromRow + Send + SoftDeleteEntity + TenantScopedEntity,
    {
        let compiled = SqlServerCompiler::compile_select(&self.effective_select_query()?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        connection.fetch_one(compiled).await
    }

    /// Executes the query and materializes projected rows as DTOs.
    pub async fn all_as<T>(self) -> Result<Vec<T>, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        T: FromRow + Send,
    {
        let compiled = SqlServerCompiler::compile_select(&self.effective_select_query()?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        connection.fetch_all(compiled).await
    }

    /// Executes the query and materializes the first projected DTO, if any.
    pub async fn first_as<T>(self) -> Result<Option<T>, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        T: FromRow + Send,
    {
        let compiled = SqlServerCompiler::compile_select(&self.effective_select_query()?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        connection.fetch_one(compiled).await
    }

    /// Executes the query as a `COUNT(*)` over the effective filters.
    pub async fn count(self) -> Result<i64, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
    {
        let compiled = SqlServerCompiler::compile_aggregate(
            &self.scalar_aggregate_query(AggregateProjection::count_as("count"))?,
        )?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        let row = connection.fetch_one::<CountRow>(compiled).await?;

        row.map(|row| row.value)
            .ok_or_else(|| OrmError::new("count query did not return a row"))
    }

    /// Executes the query as an `EXISTS` predicate over the effective filters.
    pub async fn exists(self) -> Result<bool, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
    {
        let compiled = SqlServerCompiler::compile_exists(&self.exists_query()?)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        let row = connection.fetch_one::<ExistsRow>(compiled).await?;

        row.map(|row| row.value)
            .ok_or_else(|| OrmError::new("exists query did not return a row"))
    }

    /// Alias for `exists()`.
    pub async fn any(self) -> Result<bool, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
    {
        self.exists().await
    }

    /// Executes the query as a scalar `SUM(...)` aggregate.
    pub async fn sum<T>(self, column: impl Into<Expr>) -> Result<Option<T>, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        T: SqlTypeMapping + Send,
    {
        self.scalar_aggregate(AggregateExpr::sum(column)).await
    }

    /// Executes the query as a scalar `AVG(...)` aggregate.
    pub async fn avg<T>(self, column: impl Into<Expr>) -> Result<Option<T>, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        T: SqlTypeMapping + Send,
    {
        self.scalar_aggregate(AggregateExpr::avg(column)).await
    }

    /// Executes the query as a scalar `MIN(...)` aggregate.
    pub async fn min<T>(self, column: impl Into<Expr>) -> Result<Option<T>, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        T: SqlTypeMapping + Send,
    {
        self.scalar_aggregate(AggregateExpr::min(column)).await
    }

    /// Executes the query as a scalar `MAX(...)` aggregate.
    pub async fn max<T>(self, column: impl Into<Expr>) -> Result<Option<T>, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        T: SqlTypeMapping + Send,
    {
        self.scalar_aggregate(AggregateExpr::max(column)).await
    }

    async fn scalar_aggregate<T>(self, expr: AggregateExpr) -> Result<Option<T>, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        T: SqlTypeMapping + Send,
    {
        let compiled = SqlServerCompiler::compile_aggregate(
            &self.scalar_aggregate_query(AggregateProjection::expr_as(expr, "value"))?,
        )?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        let row = connection
            .fetch_one::<ScalarAggregateRow<T>>(compiled)
            .await?;

        row.map(|row| row.value)
            .ok_or_else(|| OrmError::new("scalar aggregate query did not return a row"))
    }

    fn exists_query(&self) -> Result<ExistsQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
    {
        let effective = self.effective_select_query()?;
        Ok(ExistsQuery {
            from: effective.from,
            joins: effective.joins,
            predicate: effective.predicate,
        })
    }

    fn scalar_aggregate_query(
        &self,
        projection: AggregateProjection,
    ) -> Result<AggregateQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
    {
        let effective = self.effective_select_query()?;
        Ok(AggregateQuery {
            from: effective.from,
            joins: effective.joins,
            projection: vec![projection],
            predicate: effective.predicate,
            group_by: Vec::new(),
            having: None,
            order_by: Vec::new(),
            pagination: None,
        })
    }

    fn effective_select_query(&self) -> Result<SelectQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
    {
        let mut query = self.select_query.clone();

        if let Some(predicate) =
            tenant_predicate_for::<E>(self.active_tenant.as_ref(), TableRef::for_entity::<E>())?
        {
            query = query.filter(predicate);
        }

        if let Some(predicate) =
            soft_delete_visibility_predicate_for::<E>(TableRef::for_entity::<E>(), self.visibility)?
        {
            query = query.filter(predicate);
        }

        Ok(query)
    }

    fn require_connection(&self) -> Result<SharedConnection, OrmError> {
        self.connection
            .as_ref()
            .cloned()
            .ok_or_else(|| OrmError::new("DbSetQuery requires an initialized shared connection"))
    }

    fn try_join_navigation<J: Entity>(
        mut self,
        navigation: &'static str,
        join_type: JoinType,
        alias: Option<&'static str>,
    ) -> Result<Self, OrmError> {
        let join = self.navigation_join::<J>(navigation, join_type, alias)?;
        self.select_query = self.select_query.join(join);
        Ok(self)
    }

    fn navigation_join<J: Entity>(
        &self,
        navigation: &'static str,
        join_type: JoinType,
        alias: Option<&'static str>,
    ) -> Result<Join, OrmError> {
        let root_metadata = E::metadata();
        let target_metadata = J::metadata();
        let navigation = root_metadata.navigation(navigation).ok_or_else(|| {
            OrmError::new(format!(
                "entity `{}` does not declare navigation `{}`",
                root_metadata.rust_name, navigation
            ))
        })?;

        if navigation.target_schema != target_metadata.schema
            || navigation.target_table != target_metadata.table
        {
            return Err(OrmError::new(format!(
                "navigation `{}` on `{}` targets `{}.{}`, not entity `{}` (`{}.{}`)",
                navigation.rust_field,
                root_metadata.rust_name,
                navigation.target_schema,
                navigation.target_table,
                target_metadata.rust_name,
                target_metadata.schema,
                target_metadata.table
            )));
        }

        if navigation.local_columns.is_empty()
            || navigation.local_columns.len() != navigation.target_columns.len()
        {
            return Err(OrmError::new(format!(
                "navigation `{}` on `{}` has invalid join column metadata",
                navigation.rust_field, root_metadata.rust_name
            )));
        }

        let target_table = match alias {
            Some(alias) => TableRef::for_entity_as::<J>(alias),
            None => TableRef::for_entity::<J>(),
        };

        let predicates = navigation
            .local_columns
            .iter()
            .zip(navigation.target_columns.iter())
            .map(|(local_column, target_column)| {
                Ok(Predicate::eq(
                    metadata_column_expr(root_metadata, self.select_query.from, local_column)?,
                    metadata_column_expr(target_metadata, target_table, target_column)?,
                ))
            })
            .collect::<Result<Vec<_>, OrmError>>()?;

        let on = if predicates.len() == 1 {
            predicates[0].clone()
        } else {
            Predicate::and(predicates)
        };

        Ok(Join::new(join_type, target_table, on))
    }
}

/// Converts public `group_by(...)` arguments into neutral query expressions.
pub trait GroupByExpressions {
    fn into_group_by_expressions(self) -> Vec<Expr>;
}

impl GroupByExpressions for Expr {
    fn into_group_by_expressions(self) -> Vec<Expr> {
        vec![self]
    }
}

impl<E> GroupByExpressions for EntityColumn<E>
where
    E: Entity,
{
    fn into_group_by_expressions(self) -> Vec<Expr> {
        vec![Expr::from(self)]
    }
}

impl<E> GroupByExpressions for AliasedEntityColumn<E>
where
    E: Entity,
{
    fn into_group_by_expressions(self) -> Vec<Expr> {
        vec![Expr::from(self)]
    }
}

impl<P> GroupByExpressions for Vec<P>
where
    P: Into<Expr>,
{
    fn into_group_by_expressions(self) -> Vec<Expr> {
        self.into_iter().map(Into::into).collect()
    }
}

impl<P, const N: usize> GroupByExpressions for [P; N]
where
    P: Into<Expr>,
{
    fn into_group_by_expressions(self) -> Vec<Expr> {
        self.into_iter().map(Into::into).collect()
    }
}

macro_rules! impl_group_by_expressions_tuple {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> GroupByExpressions for ($($name,)+)
        where
            $($name: Into<Expr>),+
        {
            #[allow(non_snake_case)]
            fn into_group_by_expressions(self) -> Vec<Expr> {
                let ($($name,)+) = self;
                vec![$($name.into()),+]
            }
        }
    };
}

impl_group_by_expressions_tuple!(A);
impl_group_by_expressions_tuple!(A, B);
impl_group_by_expressions_tuple!(A, B, C);
impl_group_by_expressions_tuple!(A, B, C, D);
impl_group_by_expressions_tuple!(A, B, C, D, E);
impl_group_by_expressions_tuple!(A, B, C, D, E, F);
impl_group_by_expressions_tuple!(A, B, C, D, E, F, G);
impl_group_by_expressions_tuple!(A, B, C, D, E, F, G, H);
impl_group_by_expressions_tuple!(A, B, C, D, E, F, G, H, I);
impl_group_by_expressions_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_group_by_expressions_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_group_by_expressions_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

/// Converts public `select_aggregate(...)` arguments into aggregate
/// projections.
pub trait AggregateProjections {
    fn into_aggregate_projections(self) -> Vec<AggregateProjection>;
}

impl AggregateProjections for AggregateProjection {
    fn into_aggregate_projections(self) -> Vec<AggregateProjection> {
        vec![self]
    }
}

impl<P> AggregateProjections for Vec<P>
where
    P: Into<AggregateProjection>,
{
    fn into_aggregate_projections(self) -> Vec<AggregateProjection> {
        self.into_iter().map(Into::into).collect()
    }
}

impl<P, const N: usize> AggregateProjections for [P; N]
where
    P: Into<AggregateProjection>,
{
    fn into_aggregate_projections(self) -> Vec<AggregateProjection> {
        self.into_iter().map(Into::into).collect()
    }
}

macro_rules! impl_aggregate_projections_tuple {
    ($($name:ident),+ $(,)?) => {
        impl<$($name),+> AggregateProjections for ($($name,)+)
        where
            $($name: Into<AggregateProjection>),+
        {
            #[allow(non_snake_case)]
            fn into_aggregate_projections(self) -> Vec<AggregateProjection> {
                let ($($name,)+) = self;
                vec![$($name.into()),+]
            }
        }
    };
}

impl_aggregate_projections_tuple!(A);
impl_aggregate_projections_tuple!(A, B);
impl_aggregate_projections_tuple!(A, B, C);
impl_aggregate_projections_tuple!(A, B, C, D);
impl_aggregate_projections_tuple!(A, B, C, D, E);
impl_aggregate_projections_tuple!(A, B, C, D, E, F);
impl_aggregate_projections_tuple!(A, B, C, D, E, F, G);
impl_aggregate_projections_tuple!(A, B, C, D, E, F, G, H);
impl_aggregate_projections_tuple!(A, B, C, D, E, F, G, H, I);
impl_aggregate_projections_tuple!(A, B, C, D, E, F, G, H, I, J);
impl_aggregate_projections_tuple!(A, B, C, D, E, F, G, H, I, J, K);
impl_aggregate_projections_tuple!(A, B, C, D, E, F, G, H, I, J, K, L);

/// Query builder returned by `DbSetQuery::group_by(...)`.
pub struct DbSetGroupedQuery<E: Entity> {
    connection: Option<SharedConnection>,
    aggregate_query: AggregateQuery,
    _entity: core::marker::PhantomData<fn() -> E>,
}

impl<E: Entity> DbSetGroupedQuery<E> {
    /// Selects aggregate projections materialized by alias into DTOs.
    pub fn select_aggregate<P>(mut self, projection: P) -> Self
    where
        P: AggregateProjections,
    {
        self.aggregate_query.projection = projection.into_aggregate_projections();
        self
    }

    /// Adds a `HAVING` predicate over aggregate expressions or group keys.
    pub fn having(mut self, predicate: AggregatePredicate) -> Self {
        self.aggregate_query = self.aggregate_query.having(predicate);
        self
    }

    /// Adds aggregate ordering.
    pub fn order_by(mut self, order: AggregateOrderBy) -> Self {
        self.aggregate_query = self.aggregate_query.order_by(order);
        self
    }

    /// Limits grouped aggregate rows with zero offset.
    pub fn limit(mut self, limit: u64) -> Self {
        self.aggregate_query = self.aggregate_query.paginate(Pagination::new(0, limit));
        self
    }

    /// Alias for `limit(...)`.
    pub fn take(self, limit: u64) -> Self {
        self.limit(limit)
    }

    /// Applies page-based pagination to grouped aggregate rows.
    pub fn paginate(mut self, request: PageRequest) -> Self {
        self.aggregate_query = self.aggregate_query.paginate(request.to_pagination());
        self
    }

    /// Executes the grouped query and materializes projected rows as DTOs.
    pub async fn all_as<T>(self) -> Result<Vec<T>, OrmError>
    where
        T: FromRow + Send,
    {
        let compiled = SqlServerCompiler::compile_aggregate(&self.aggregate_query)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        connection.fetch_all(compiled).await
    }

    /// Executes the grouped query and materializes the first DTO, if any.
    pub async fn first_as<T>(self) -> Result<Option<T>, OrmError>
    where
        T: FromRow + Send,
    {
        let compiled = SqlServerCompiler::compile_aggregate(&self.aggregate_query)?;
        let shared_connection = self.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        connection.fetch_one(compiled).await
    }

    #[cfg(test)]
    pub(crate) fn aggregate_query(&self) -> &AggregateQuery {
        &self.aggregate_query
    }

    fn require_connection(&self) -> Result<SharedConnection, OrmError> {
        self.connection.as_ref().cloned().ok_or_else(|| {
            OrmError::new("DbSetGroupedQuery requires an initialized shared connection")
        })
    }
}

impl<E: Entity> core::fmt::Debug for DbSetGroupedQuery<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DbSetGroupedQuery")
            .field("entity", &E::metadata().rust_name)
            .field("table", &E::metadata().table)
            .field("aggregate_query", &self.aggregate_query)
            .finish()
    }
}

/// Query builder returned by `DbSetQuery::include::<T>(...)` for a single
/// included navigation.
pub struct DbSetQueryIncludeOne<E: Entity, J: Entity> {
    query: DbSetQuery<E>,
    navigation: &'static str,
    alias: &'static str,
    _target: core::marker::PhantomData<fn() -> J>,
}

impl<E: Entity, J: Entity> DbSetQueryIncludeOne<E, J> {
    /// Adds a predicate after configuring the include.
    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.query = self.query.filter(predicate);
        self
    }

    /// Adds an explicit join after configuring the include.
    pub fn join(mut self, join: Join) -> Self {
        self.query = self.query.join(join);
        self
    }

    /// Adds an explicit `INNER JOIN` after configuring the include.
    pub fn inner_join<K: Entity>(mut self, on: Predicate) -> Self {
        self.query = self.query.inner_join::<K>(on);
        self
    }

    /// Adds an explicit `LEFT JOIN` after configuring the include.
    pub fn left_join<K: Entity>(mut self, on: Predicate) -> Self {
        self.query = self.query.left_join::<K>(on);
        self
    }

    /// Adds an ordering expression after configuring the include.
    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.query = self.query.order_by(order);
        self
    }

    /// Limits the number of returned rows with zero offset.
    pub fn limit(mut self, limit: u64) -> Self {
        self.query = self.query.limit(limit);
        self
    }

    /// Alias for `limit(...)`.
    pub fn take(self, limit: u64) -> Self {
        self.limit(limit)
    }

    /// Applies page-based pagination after configuring the include.
    pub fn paginate(mut self, request: PageRequest) -> Self {
        self.query = self.query.paginate(request);
        self
    }

    /// Includes logically deleted root rows for entities with `soft_delete`.
    ///
    /// This affects only the root entity `E`; included entities still apply
    /// their own default `soft_delete` visibility inside the include join.
    pub fn with_deleted(mut self) -> Self {
        self.query = self.query.with_deleted();
        self
    }

    /// Returns only logically deleted root rows for entities with `soft_delete`.
    ///
    /// This affects only the root entity `E`; included entities still apply
    /// their own default `soft_delete` visibility inside the include join.
    pub fn only_deleted(mut self) -> Self {
        self.query = self.query.only_deleted();
        self
    }

    /// Executes the query and materializes root entities with one included
    /// navigation attached.
    pub async fn all(self) -> Result<Vec<E>, OrmError>
    where
        E: FromRow + IncludeNavigation<J> + Send + SoftDeleteEntity + TenantScopedEntity,
        J: Clone + FromRow + Send + SoftDeleteEntity + Sync + TenantScopedEntity + 'static,
    {
        let navigation = self.navigation;
        let alias = self.alias;
        let tracking_registry = self.query.tracking_registry.clone();
        let compiled = SqlServerCompiler::compile_select(&self.effective_select_query()?)?;
        let shared_connection = self.query.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        connection
            .fetch_all_with(compiled, move |row| {
                materialize_include_one::<E, J>(&row, navigation, alias, tracking_registry.as_ref())
            })
            .await
    }

    /// Executes the query and materializes the first root entity with one
    /// included navigation attached, if any.
    pub async fn first(self) -> Result<Option<E>, OrmError>
    where
        E: FromRow + IncludeNavigation<J> + Send + SoftDeleteEntity + TenantScopedEntity,
        J: Clone + FromRow + Send + SoftDeleteEntity + Sync + TenantScopedEntity + 'static,
    {
        let navigation = self.navigation;
        let alias = self.alias;
        let tracking_registry = self.query.tracking_registry.clone();
        let compiled = SqlServerCompiler::compile_select(&self.effective_select_query()?)?;
        let shared_connection = self.query.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        connection
            .fetch_one_with(compiled, move |row| {
                materialize_include_one::<E, J>(&row, navigation, alias, tracking_registry.as_ref())
            })
            .await
    }

    #[cfg(test)]
    pub(crate) fn select_query(&self) -> Result<SelectQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        J: SoftDeleteEntity + TenantScopedEntity,
    {
        self.effective_select_query()
    }

    fn effective_select_query(&self) -> Result<SelectQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        J: SoftDeleteEntity + TenantScopedEntity,
    {
        let query = self.query.effective_select_query()?;
        let query = apply_include_policy_filters::<J>(
            query,
            self.query.active_tenant.as_ref(),
            self.alias,
        )?;
        apply_include_projection::<E, J>(query, self.alias)
    }
}

/// Query builder returned by `DbSetQuery::include_many::<T>(...)` for a
/// collection navigation.
pub struct DbSetQueryIncludeMany<E: Entity, J: Entity> {
    query: DbSetQuery<E>,
    navigation: &'static str,
    alias: &'static str,
    strategy: CollectionIncludeStrategy,
    join_row_limit: Option<usize>,
    _target: core::marker::PhantomData<fn() -> J>,
}

impl<E: Entity, J: Entity> DbSetQueryIncludeMany<E, J> {
    /// Adds a predicate after configuring the collection include.
    pub fn filter(mut self, predicate: Predicate) -> Self {
        self.query = self.query.filter(predicate);
        self
    }

    /// Adds an explicit join after configuring the collection include.
    pub fn join(mut self, join: Join) -> Self {
        self.query = self.query.join(join);
        self
    }

    /// Adds an explicit `INNER JOIN` after configuring the collection include.
    pub fn inner_join<K: Entity>(mut self, on: Predicate) -> Self {
        self.query = self.query.inner_join::<K>(on);
        self
    }

    /// Adds an explicit `LEFT JOIN` after configuring the collection include.
    pub fn left_join<K: Entity>(mut self, on: Predicate) -> Self {
        self.query = self.query.left_join::<K>(on);
        self
    }

    /// Adds an ordering expression after configuring the collection include.
    pub fn order_by(mut self, order: OrderBy) -> Self {
        self.query = self.query.order_by(order);
        self
    }

    /// Uses the join-based collection loading strategy.
    ///
    /// This is the default strategy. The row limit protects callers from
    /// accidentally loading an unbounded cartesian result through one join.
    pub fn join_strategy(mut self) -> Self {
        self.strategy = CollectionIncludeStrategy::Join;
        self
    }

    /// Selects the planned split-query loading strategy.
    ///
    /// Execution currently returns a clear error because split queries need a
    /// separate implementation that loads roots first and related rows second.
    pub fn split_query(mut self) -> Self {
        self.strategy = CollectionIncludeStrategy::SplitQuery;
        self
    }

    /// Overrides the maximum number of joined rows accepted before grouping.
    ///
    /// Use this only when the expected root/collection cardinality is known.
    pub fn max_joined_rows(mut self, limit: usize) -> Self {
        self.join_row_limit = Some(limit);
        self
    }

    /// Removes the join row safety limit.
    ///
    /// This keeps the API explicit for callers that intentionally accept a
    /// large join result.
    pub fn unbounded_join(mut self) -> Self {
        self.join_row_limit = None;
        self
    }

    /// Includes logically deleted root rows for entities with `soft_delete`.
    ///
    /// This affects only the root entity `E`; included collection entities
    /// still apply their own default `soft_delete` visibility inside the join.
    pub fn with_deleted(mut self) -> Self {
        self.query = self.query.with_deleted();
        self
    }

    /// Returns only logically deleted root rows for entities with `soft_delete`.
    ///
    /// This affects only the root entity `E`; included collection entities
    /// still apply their own default `soft_delete` visibility inside the join.
    pub fn only_deleted(mut self) -> Self {
        self.query = self.query.only_deleted();
        self
    }

    /// Executes the query and materializes root entities with one collection
    /// navigation attached.
    pub async fn all(self) -> Result<Vec<E>, OrmError>
    where
        E: FromRow + IncludeCollection<J> + Send + SoftDeleteEntity + TenantScopedEntity,
        J: Clone + FromRow + Send + SoftDeleteEntity + Sync + TenantScopedEntity + 'static,
    {
        if self.strategy == CollectionIncludeStrategy::SplitQuery {
            return Err(OrmError::new(
                "include_many split-query loading is not implemented yet; use join_strategy() with an explicit max_joined_rows(...) limit",
            ));
        }

        let navigation = self.navigation;
        let alias = self.alias;
        let tracking_registry = self.query.tracking_registry.clone();
        let compiled = SqlServerCompiler::compile_select(&self.effective_select_query()?)?;
        let shared_connection = self.query.require_connection()?;
        let mut connection = shared_connection.lock().await?;
        let rows = connection
            .fetch_all_with(compiled, move |row| {
                materialize_include_many_row::<E, J>(&row, alias)
            })
            .await?;

        enforce_include_many_join_row_limit(rows.len(), self.join_row_limit)?;
        group_include_many_rows::<E, J>(rows, navigation, tracking_registry.as_ref())
    }

    #[cfg(test)]
    pub(crate) fn select_query(&self) -> Result<SelectQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        J: SoftDeleteEntity + TenantScopedEntity,
    {
        self.effective_select_query()
    }

    fn effective_select_query(&self) -> Result<SelectQuery, OrmError>
    where
        E: SoftDeleteEntity + TenantScopedEntity,
        J: SoftDeleteEntity + TenantScopedEntity,
    {
        let query = self.query.effective_select_query()?;
        if query.pagination.is_some() {
            return Err(OrmError::new(
                "include_many does not support pagination in the join-based collection loading cut",
            ));
        }

        let query = apply_include_policy_filters::<J>(
            query,
            self.query.active_tenant.as_ref(),
            self.alias,
        )?;
        apply_include_projection::<E, J>(query, self.alias)
    }
}

fn tenant_predicate_for<E: TenantScopedEntity>(
    active_tenant: Option<&ActiveTenant>,
    table: TableRef,
) -> Result<Option<Predicate>, OrmError> {
    let Some(policy) = E::tenant_policy() else {
        return Ok(None);
    };

    if policy.columns.len() != 1 {
        return Err(OrmError::new(
            "tenant query filter requires exactly one tenant policy column",
        ));
    }

    let tenant_column = &policy.columns[0];
    let active_tenant = active_tenant.ok_or_else(|| {
        OrmError::new("tenant-scoped query requires an active tenant in the DbContext")
    })?;

    if active_tenant.column_name != tenant_column.column_name {
        return Err(OrmError::new(format!(
            "active tenant column `{}` does not match entity tenant column `{}`",
            active_tenant.column_name, tenant_column.column_name
        )));
    }

    if !tenant_value_matches_column_type(&active_tenant.value, tenant_column) {
        return Err(OrmError::new(format!(
            "active tenant value is not compatible with entity tenant column `{}`",
            tenant_column.column_name
        )));
    }

    Ok(Some(Predicate::eq(
        Expr::Column(ColumnRef::new(
            table,
            tenant_column.rust_field,
            tenant_column.column_name,
        )),
        Expr::Value(active_tenant.value.clone()),
    )))
}

fn soft_delete_visibility_predicate_for<E: SoftDeleteEntity>(
    table: TableRef,
    visibility: SoftDeleteVisibility,
) -> Result<Option<Predicate>, OrmError> {
    let Some(policy) = E::soft_delete_policy() else {
        return Ok(None);
    };

    let visibility = match visibility {
        SoftDeleteVisibility::Default => SoftDeleteVisibility::Default,
        SoftDeleteVisibility::WithDeleted => return Ok(None),
        SoftDeleteVisibility::OnlyDeleted => SoftDeleteVisibility::OnlyDeleted,
    };

    let indicator = policy.columns.first().ok_or_else(|| {
        OrmError::new("soft_delete query visibility requires at least one policy column")
    })?;
    let column = Expr::Column(ColumnRef::new(
        table,
        indicator.rust_field,
        indicator.column_name,
    ));

    if indicator.sql_type == SqlServerType::Bit {
        return Ok(Some(match visibility {
            SoftDeleteVisibility::Default => {
                Predicate::eq(column, Expr::Value(SqlValue::Bool(false)))
            }
            SoftDeleteVisibility::OnlyDeleted => {
                Predicate::eq(column, Expr::Value(SqlValue::Bool(true)))
            }
            SoftDeleteVisibility::WithDeleted => unreachable!(),
        }));
    }

    if indicator.nullable {
        return Ok(Some(match visibility {
            SoftDeleteVisibility::Default => Predicate::is_null(column),
            SoftDeleteVisibility::OnlyDeleted => Predicate::is_not_null(column),
            SoftDeleteVisibility::WithDeleted => unreachable!(),
        }));
    }

    Err(OrmError::new(
        "soft_delete query visibility requires the first policy column to be nullable or bit",
    ))
}

fn apply_include_policy_filters<J: Entity + SoftDeleteEntity + TenantScopedEntity>(
    mut query: SelectQuery,
    active_tenant: Option<&ActiveTenant>,
    alias: &'static str,
) -> Result<SelectQuery, OrmError> {
    let target_table = TableRef::for_entity_as::<J>(alias);
    let mut predicates = Vec::new();

    if let Some(predicate) = tenant_predicate_for::<J>(active_tenant, target_table)? {
        predicates.push(predicate);
    }

    if let Some(predicate) =
        soft_delete_visibility_predicate_for::<J>(target_table, SoftDeleteVisibility::Default)?
    {
        predicates.push(predicate);
    }

    if predicates.is_empty() {
        return Ok(query);
    }

    let include_join = query
        .joins
        .iter_mut()
        .find(|join| join.table == target_table)
        .ok_or_else(|| {
            OrmError::new(format!(
                "include join for entity `{}` with alias `{}` was not found",
                J::metadata().rust_name,
                alias
            ))
        })?;

    let policy_predicate = if predicates.len() == 1 {
        predicates.remove(0)
    } else {
        Predicate::and(predicates)
    };
    include_join.on = Predicate::and(vec![include_join.on.clone(), policy_predicate]);

    Ok(query)
}

fn apply_include_projection<E: Entity, J: Entity>(
    mut query: SelectQuery,
    alias: &'static str,
) -> Result<SelectQuery, OrmError> {
    let mut projection = Vec::new();

    projection.extend(E::metadata().columns.iter().map(|column| {
        SelectProjection::expr_as(
            Expr::Column(ColumnRef::new(
                query.from,
                column.rust_field,
                column.column_name,
            )),
            column.column_name,
        )
    }));

    let target_table = TableRef::for_entity_as::<J>(alias);
    for column in J::metadata().columns {
        projection.push(SelectProjection::expr_as(
            Expr::Column(ColumnRef::new(
                target_table,
                column.rust_field,
                column.column_name,
            )),
            include_column_alias(alias, column.column_name),
        ));
    }

    query.projection = projection;
    Ok(query)
}

fn materialize_include_one<E, J>(
    row: &impl Row,
    navigation: &'static str,
    alias: &'static str,
    tracking_registry: Option<&TrackingRegistryHandle>,
) -> Result<E, OrmError>
where
    E: FromRow + IncludeNavigation<J>,
    J: Clone + Entity + FromRow + Send + Sync + 'static,
{
    let mut entity = E::from_row(row)?;
    let related = materialize_prefixed_entity::<J>(row, alias)?;
    let related_key = prefixed_primary_key_value::<J>(row, alias)?;
    let related =
        identity_mapped_optional_navigation_value(tracking_registry, related_key, related);
    entity.set_included_navigation(navigation, related)?;
    Ok(entity)
}

fn materialize_prefixed_entity<J: Entity + FromRow>(
    row: &impl Row,
    alias: &'static str,
) -> Result<Option<J>, OrmError> {
    let prefix = include_prefix(alias);
    let mut saw_value = false;

    for column in J::metadata().columns {
        let projected = prefixed_column_name(&prefix, column.column_name);
        if let Some(value) = row.try_get(&projected)? {
            if !value.is_null() {
                saw_value = true;
                break;
            }
        }
    }

    if !saw_value {
        return Ok(None);
    }

    Ok(Some(J::from_row(&PrefixedRow { row, prefix })?))
}

struct IncludeManyRow<E, J> {
    root_key: Vec<SqlValue>,
    root: E,
    related_key: Option<SqlValue>,
    related: Option<J>,
}

fn materialize_include_many_row<E, J>(
    row: &impl Row,
    alias: &'static str,
) -> Result<IncludeManyRow<E, J>, OrmError>
where
    E: Entity + FromRow,
    J: Entity + FromRow,
{
    Ok(IncludeManyRow {
        root_key: root_primary_key_values::<E>(row)?,
        root: E::from_row(row)?,
        related_key: prefixed_primary_key_value::<J>(row, alias)?,
        related: materialize_prefixed_entity::<J>(row, alias)?,
    })
}

fn prefixed_primary_key_value<J: Entity>(
    row: &impl Row,
    alias: &'static str,
) -> Result<Option<SqlValue>, OrmError> {
    let metadata = J::metadata();
    if metadata.primary_key.columns.len() != 1 {
        return Ok(None);
    }

    let prefix = include_prefix(alias);
    let column = prefixed_column_name(&prefix, metadata.primary_key.columns[0]);
    row.try_get(&column)
}

fn root_primary_key_values<E: Entity>(row: &impl Row) -> Result<Vec<SqlValue>, OrmError> {
    let metadata = E::metadata();
    if metadata.primary_key.columns.is_empty() {
        return Err(OrmError::new(format!(
            "include_many requires entity `{}` to declare a primary key for row grouping",
            metadata.rust_name
        )));
    }

    metadata
        .primary_key
        .columns
        .iter()
        .map(|column_name| row.get_required(column_name))
        .collect()
}

fn group_include_many_rows<E, J>(
    rows: Vec<IncludeManyRow<E, J>>,
    navigation: &'static str,
    tracking_registry: Option<&TrackingRegistryHandle>,
) -> Result<Vec<E>, OrmError>
where
    E: IncludeCollection<J>,
    J: Clone + Entity + Send + Sync + 'static,
{
    let mut grouped: Vec<(Vec<SqlValue>, E, Vec<J>)> = Vec::new();

    for row in rows {
        let related = identity_mapped_optional_navigation_value(
            tracking_registry,
            row.related_key,
            row.related,
        );
        if let Some((_, _, related_values)) = grouped
            .iter_mut()
            .find(|(root_key, _, _)| *root_key == row.root_key)
        {
            if let Some(related) = related {
                related_values.push(related);
            }
            continue;
        }

        let related_values = related.into_iter().collect();
        grouped.push((row.root_key, row.root, related_values));
    }

    grouped
        .into_iter()
        .map(|(_, mut root, related_values)| {
            root.set_included_collection(navigation, related_values)?;
            Ok(root)
        })
        .collect()
}

fn identity_mapped_optional_navigation_value<J>(
    tracking_registry: Option<&TrackingRegistryHandle>,
    key: Option<SqlValue>,
    value: Option<J>,
) -> Option<J>
where
    J: Clone + Entity + Send + Sync + 'static,
{
    value.map(|value| identity_mapped_navigation_value(tracking_registry, key, value))
}

fn identity_mapped_navigation_value<J>(
    tracking_registry: Option<&TrackingRegistryHandle>,
    key: Option<SqlValue>,
    value: J,
) -> J
where
    J: Clone + Entity + Send + Sync + 'static,
{
    let Some(registry) = tracking_registry else {
        return value;
    };
    let Some(key) = key else {
        return value;
    };

    registry.current_snapshot_for_key::<J>(key).unwrap_or(value)
}

fn enforce_include_many_join_row_limit(
    row_count: usize,
    limit: Option<usize>,
) -> Result<(), OrmError> {
    let Some(limit) = limit else {
        return Ok(());
    };

    if row_count > limit {
        return Err(OrmError::new(format!(
            "include_many join produced {row_count} rows, exceeding the configured limit of {limit}; use max_joined_rows(...), unbounded_join(), or wait for split-query collection loading"
        )));
    }

    Ok(())
}

struct PrefixedRow<'a, R: Row + ?Sized> {
    row: &'a R,
    prefix: String,
}

impl<R: Row + ?Sized> Row for PrefixedRow<'_, R> {
    fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
        self.row
            .try_get(&prefixed_column_name(&self.prefix, column))
    }
}

fn include_prefix(alias: &'static str) -> String {
    format!("{alias}__")
}

fn include_column_alias(alias: &'static str, column_name: &'static str) -> &'static str {
    Box::leak(format!("{alias}__{column_name}").into_boxed_str())
}

fn prefixed_column_name(prefix: &str, column_name: &str) -> String {
    format!("{prefix}{column_name}")
}

fn metadata_column_expr(
    metadata: &'static EntityMetadata,
    table: TableRef,
    column_name: &str,
) -> Result<Expr, OrmError> {
    let column = metadata.column(column_name).ok_or_else(|| {
        OrmError::new(format!(
            "entity `{}` metadata does not contain column `{}` required by navigation join",
            metadata.rust_name, column_name
        ))
    })?;

    Ok(Expr::Column(ColumnRef::new(
        table,
        column.rust_field,
        column.column_name,
    )))
}

pub(crate) fn tenant_value_matches_column_type(value: &SqlValue, column: &ColumnMetadata) -> bool {
    if value.is_null() {
        return false;
    }

    match column.sql_type {
        SqlServerType::BigInt => matches!(value, SqlValue::I64(_)),
        SqlServerType::Int | SqlServerType::SmallInt | SqlServerType::TinyInt => {
            matches!(value, SqlValue::I32(_))
        }
        SqlServerType::Bit => matches!(value, SqlValue::Bool(_)),
        SqlServerType::UniqueIdentifier => matches!(value, SqlValue::Uuid(_)),
        SqlServerType::Date => matches!(value, SqlValue::Date(_)),
        SqlServerType::DateTime2 => matches!(value, SqlValue::DateTime(_)),
        SqlServerType::Decimal | SqlServerType::Money => matches!(value, SqlValue::Decimal(_)),
        SqlServerType::Float => matches!(value, SqlValue::F64(_)),
        SqlServerType::NVarChar | SqlServerType::Custom(_) => {
            matches!(value, SqlValue::String(_))
        }
        SqlServerType::VarBinary | SqlServerType::RowVersion => matches!(value, SqlValue::Bytes(_)),
    }
}

impl<E: Entity> core::fmt::Debug for DbSetQuery<E> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("DbSetQuery")
            .field("entity", &E::metadata().rust_name)
            .field("table", &E::metadata().table)
            .field("select_query", &self.select_query)
            .finish()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CountRow {
    value: i64,
}

impl FromRow for CountRow {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        match row.get_required("count")? {
            SqlValue::I32(value) => Ok(Self {
                value: i64::from(value),
            }),
            SqlValue::I64(value) => Ok(Self { value }),
            _ => Err(OrmError::new(
                "expected SQL Server COUNT result as i32 or i64",
            )),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ExistsRow {
    value: bool,
}

impl FromRow for ExistsRow {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            value: row.get_required_typed::<bool>("exists")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
struct ScalarAggregateRow<T> {
    value: Option<T>,
}

impl<T> FromRow for ScalarAggregateRow<T>
where
    T: SqlTypeMapping,
{
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        let value = row
            .try_get("value")?
            .ok_or_else(|| OrmError::new("scalar aggregate result column was not present"))?;

        if value.is_null() {
            return Ok(Self { value: None });
        }

        Ok(Self {
            value: Some(T::from_sql_value(value)?),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{
        DbSetQuery, enforce_include_many_join_row_limit, identity_mapped_navigation_value,
        tenant_value_matches_column_type,
    };
    use crate::context::{ActiveTenant, DbSet};
    use crate::page_request::PageRequest;
    use crate::{
        EntityColumnAliasExt, IncludeCollection, SoftDeleteEntity, TenantScopedEntity, Tracked,
        TrackingRegistry,
    };
    use chrono::{NaiveDate, NaiveDateTime};
    use insta::assert_snapshot;
    use rust_decimal::Decimal;
    use sql_orm_core::{
        ColumnMetadata, Entity, EntityColumn, EntityMetadata, EntityPolicyMetadata, FromRow,
        NavigationKind, NavigationMetadata, OrmError, PrimaryKeyMetadata, Row, SqlServerType,
        SqlValue,
    };
    use sql_orm_query::{
        AggregateExpr, AggregateOrderBy, AggregatePredicate, AggregateProjection, ColumnRef,
        CompiledQuery, Expr, Join, JoinType, OrderBy, Pagination, Predicate, SelectProjection,
        SelectQuery, SortDirection, TableRef,
    };
    use sql_orm_sqlserver::SqlServerCompiler;

    struct TestEntity;
    struct JoinedEntity;
    #[derive(Debug)]
    struct NavigationRoot;
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct NavigationTarget {
        id: i64,
        owner_id: i64,
    }
    struct TenantNavigationRoot;
    struct TenantNavigationTarget;
    struct SoftDeleteEntityUnderTest;
    struct BoolSoftDeleteEntity;
    struct TenantEntity;

    static TEST_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TestEntity",
        schema: "dbo",
        table: "test_entities",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &[],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    impl Entity for TestEntity {
        fn metadata() -> &'static EntityMetadata {
            &TEST_ENTITY_METADATA
        }
    }

    #[allow(non_upper_case_globals)]
    impl TestEntity {
        const id: EntityColumn<TestEntity> = EntityColumn::new("id", "id");
        const name: EntityColumn<TestEntity> = EntityColumn::new("name", "name");
    }

    static JOINED_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "JoinedEntity",
        schema: "dbo",
        table: "joined_entities",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &[],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    impl Entity for JoinedEntity {
        fn metadata() -> &'static EntityMetadata {
            &JOINED_ENTITY_METADATA
        }
    }

    static NAVIGATION_ROOT_COLUMNS: [ColumnMetadata; 1] = [ColumnMetadata {
        rust_field: "id",
        column_name: "id",
        renamed_from: None,
        sql_type: SqlServerType::BigInt,
        nullable: false,
        primary_key: true,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: false,
        updatable: false,
        max_length: None,
        precision: None,
        scale: None,
    }];

    static NAVIGATION_TARGET_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "owner_id",
            column_name: "owner_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static NAVIGATION_ROOT_NAVIGATIONS: [NavigationMetadata; 1] = [NavigationMetadata::new(
        "orders",
        NavigationKind::HasMany,
        "NavigationTarget",
        "sales",
        "navigation_targets",
        &["id"],
        &["owner_id"],
        Some("fk_navigation_targets_owner"),
    )];

    static NAVIGATION_ROOT_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "NavigationRoot",
        schema: "dbo",
        table: "navigation_roots",
        renamed_from: None,
        columns: &NAVIGATION_ROOT_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &NAVIGATION_ROOT_NAVIGATIONS,
    };

    static NAVIGATION_TARGET_NAVIGATIONS: [NavigationMetadata; 1] = [NavigationMetadata::new(
        "owner",
        NavigationKind::BelongsTo,
        "NavigationRoot",
        "dbo",
        "navigation_roots",
        &["owner_id"],
        &["id"],
        Some("fk_navigation_targets_owner"),
    )];

    static NAVIGATION_TARGET_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "NavigationTarget",
        schema: "sales",
        table: "navigation_targets",
        renamed_from: None,
        columns: &NAVIGATION_TARGET_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &NAVIGATION_TARGET_NAVIGATIONS,
    };

    impl Entity for NavigationRoot {
        fn metadata() -> &'static EntityMetadata {
            &NAVIGATION_ROOT_METADATA
        }
    }

    impl Entity for NavigationTarget {
        fn metadata() -> &'static EntityMetadata {
            &NAVIGATION_TARGET_METADATA
        }
    }

    #[allow(non_upper_case_globals)]
    impl NavigationTarget {
        const id: EntityColumn<NavigationTarget> = EntityColumn::new("id", "id");
        const owner_id: EntityColumn<NavigationTarget> = EntityColumn::new("owner_id", "owner_id");
    }

    impl FromRow for NavigationRoot {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self)
        }
    }

    impl FromRow for NavigationTarget {
        fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
            Ok(Self {
                id: required_i64(row, "id")?,
                owner_id: required_i64(row, "owner_id")?,
            })
        }
    }

    fn required_i64<R: Row>(row: &R, column: &str) -> Result<i64, OrmError> {
        match row.get_required(column)? {
            SqlValue::I64(value) => Ok(value),
            value => Err(OrmError::new(format!(
                "expected `{column}` as i64, got {value:?}"
            ))),
        }
    }

    static SOFT_DELETE_POLICY_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "deleted_at",
            column_name: "deleted_at",
            renamed_from: None,
            sql_type: SqlServerType::DateTime2,
            nullable: true,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "deleted_by",
            column_name: "deleted_by",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: true,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: true,
            max_length: Some(120),
            precision: None,
            scale: None,
        },
    ];

    static BOOL_SOFT_DELETE_POLICY_COLUMNS: [ColumnMetadata; 1] = [ColumnMetadata {
        rust_field: "is_deleted",
        column_name: "is_deleted",
        renamed_from: None,
        sql_type: SqlServerType::Bit,
        nullable: false,
        primary_key: false,
        identity: None,
        default_sql: Some("0"),
        computed_sql: None,
        rowversion: false,
        insertable: false,
        updatable: true,
        max_length: None,
        precision: None,
        scale: None,
    }];

    static SOFT_DELETE_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "SoftDeleteEntityUnderTest",
        schema: "dbo",
        table: "soft_delete_entities",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &[],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static BOOL_SOFT_DELETE_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "BoolSoftDeleteEntity",
        schema: "dbo",
        table: "bool_soft_delete_entities",
        renamed_from: None,
        columns: &[],
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &[],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static TENANT_POLICY_COLUMNS: [ColumnMetadata; 1] = [ColumnMetadata {
        rust_field: "tenant_id",
        column_name: "tenant_id",
        renamed_from: None,
        sql_type: SqlServerType::BigInt,
        nullable: false,
        primary_key: false,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: true,
        updatable: false,
        max_length: None,
        precision: None,
        scale: None,
    }];

    static TENANT_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TenantEntity",
        schema: "sales",
        table: "tenant_entities",
        renamed_from: None,
        columns: &TENANT_POLICY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &[],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static TENANT_NAVIGATION_ROOT_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        TENANT_POLICY_COLUMNS[0],
    ];

    static TENANT_NAVIGATION_TARGET_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "owner_id",
            column_name: "owner_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    static TENANT_NAVIGATION_TARGET_NAVIGATIONS: [NavigationMetadata; 1] =
        [NavigationMetadata::new(
            "owner",
            NavigationKind::BelongsTo,
            "TenantNavigationRoot",
            "sales",
            "tenant_navigation_roots",
            &["owner_id"],
            &["id"],
            Some("fk_tenant_navigation_targets_owner"),
        )];

    static TENANT_NAVIGATION_ROOT_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TenantNavigationRoot",
        schema: "sales",
        table: "tenant_navigation_roots",
        renamed_from: None,
        columns: &TENANT_NAVIGATION_ROOT_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &[],
    };

    static TENANT_NAVIGATION_TARGET_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TenantNavigationTarget",
        schema: "sales",
        table: "tenant_navigation_targets",
        renamed_from: None,
        columns: &TENANT_NAVIGATION_TARGET_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
        },
        indexes: &[],
        foreign_keys: &[],
        navigations: &TENANT_NAVIGATION_TARGET_NAVIGATIONS,
    };

    impl Entity for SoftDeleteEntityUnderTest {
        fn metadata() -> &'static EntityMetadata {
            &SOFT_DELETE_ENTITY_METADATA
        }
    }

    impl Entity for BoolSoftDeleteEntity {
        fn metadata() -> &'static EntityMetadata {
            &BOOL_SOFT_DELETE_ENTITY_METADATA
        }
    }

    impl Entity for TenantEntity {
        fn metadata() -> &'static EntityMetadata {
            &TENANT_ENTITY_METADATA
        }
    }

    #[allow(non_upper_case_globals)]
    impl TenantEntity {
        const tenant_id: EntityColumn<TenantEntity> = EntityColumn::new("tenant_id", "tenant_id");
    }

    impl Entity for TenantNavigationRoot {
        fn metadata() -> &'static EntityMetadata {
            &TENANT_NAVIGATION_ROOT_METADATA
        }
    }

    impl Entity for TenantNavigationTarget {
        fn metadata() -> &'static EntityMetadata {
            &TENANT_NAVIGATION_TARGET_METADATA
        }
    }

    impl SoftDeleteEntity for TestEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for JoinedEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for SoftDeleteEntityUnderTest {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new(
                "soft_delete",
                &SOFT_DELETE_POLICY_COLUMNS,
            ))
        }
    }

    impl SoftDeleteEntity for BoolSoftDeleteEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new(
                "soft_delete",
                &BOOL_SOFT_DELETE_POLICY_COLUMNS,
            ))
        }
    }

    impl SoftDeleteEntity for TenantEntity {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for TestEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for JoinedEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for SoftDeleteEntityUnderTest {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for BoolSoftDeleteEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for TenantEntity {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new("tenant", &TENANT_POLICY_COLUMNS))
        }
    }

    impl SoftDeleteEntity for NavigationRoot {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new(
                "soft_delete",
                &SOFT_DELETE_POLICY_COLUMNS,
            ))
        }
    }

    impl SoftDeleteEntity for NavigationTarget {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for TenantNavigationRoot {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl SoftDeleteEntity for TenantNavigationTarget {
        fn soft_delete_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for NavigationRoot {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for NavigationTarget {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl TenantScopedEntity for TenantNavigationRoot {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            Some(EntityPolicyMetadata::new("tenant", &TENANT_POLICY_COLUMNS))
        }
    }

    impl TenantScopedEntity for TenantNavigationTarget {
        fn tenant_policy() -> Option<EntityPolicyMetadata> {
            None
        }
    }

    impl IncludeCollection<NavigationTarget> for NavigationRoot {
        fn set_included_collection(
            &mut self,
            _navigation: &str,
            _values: Vec<NavigationTarget>,
        ) -> Result<(), OrmError> {
            Ok(())
        }
    }

    #[derive(Debug)]
    struct TestProjectionRow;

    impl FromRow for TestProjectionRow {
        fn from_row<R: Row>(_row: &R) -> Result<Self, OrmError> {
            Ok(Self)
        }
    }

    #[test]
    fn dbset_query_starts_from_entity_select_query() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let query = dbset.query();

        assert_eq!(
            query.select_query(),
            &SelectQuery::from_entity::<TestEntity>()
        );
    }

    #[test]
    fn dbset_query_accepts_replacement_select_query() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let custom = SelectQuery::from_entity::<TestEntity>().filter(Predicate::eq(
            Expr::value(SqlValue::Bool(true)),
            Expr::value(SqlValue::Bool(true)),
        ));

        let query = dbset.query().with_select_query(custom.clone());

        assert_eq!(query.select_query(), &custom);
        assert_eq!(query.into_select_query(), custom);
    }

    #[test]
    fn dbset_query_filter_builds_on_internal_select_query() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset.query().filter(Predicate::eq(
            Expr::value(SqlValue::Bool(true)),
            Expr::value(SqlValue::Bool(true)),
        ));

        assert_eq!(
            query.into_select_query(),
            SelectQuery::from_entity::<TestEntity>().filter(Predicate::eq(
                Expr::value(SqlValue::Bool(true)),
                Expr::value(SqlValue::Bool(true)),
            ))
        );
    }

    #[test]
    fn dbset_query_select_builds_projection_with_aliases() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset
            .query()
            .select((TestEntity::id, TestEntity::name))
            .into_select_query();

        assert_eq!(
            query.projection,
            vec![
                SelectProjection::column(TestEntity::id),
                SelectProjection::column(TestEntity::name),
            ]
        );
    }

    #[tokio::test]
    async fn dbset_query_all_as_reuses_projection_compilation_before_connection() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let error = dbset
            .query()
            .select(TestEntity::id)
            .all_as::<TestProjectionRow>()
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "DbSetQuery requires an initialized shared connection"
        );
    }

    #[tokio::test]
    async fn dbset_query_first_as_rejects_unaliased_expression_projection() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let error = dbset
            .query()
            .select(Expr::function("LOWER", vec![Expr::from(TestEntity::name)]))
            .first_as::<TestProjectionRow>()
            .await
            .unwrap_err();

        assert_eq!(
            error.message(),
            "SQL Server projection expressions require an explicit alias"
        );
    }

    #[test]
    fn dbset_query_applies_active_only_visibility_for_nullable_indicator() {
        let dbset = DbSet::<SoftDeleteEntityUnderTest>::disconnected();

        let query = dbset.query().effective_select_query().unwrap();

        assert_eq!(
            query,
            SelectQuery::from_entity::<SoftDeleteEntityUnderTest>().filter(Predicate::is_null(
                Expr::Column(sql_orm_query::ColumnRef::new(
                    TableRef::new("dbo", "soft_delete_entities"),
                    "deleted_at",
                    "deleted_at",
                )),
            ))
        );
    }

    #[test]
    fn dbset_query_with_deleted_removes_soft_delete_filter() {
        let dbset = DbSet::<SoftDeleteEntityUnderTest>::disconnected();

        let query = dbset
            .query()
            .with_deleted()
            .effective_select_query()
            .unwrap();

        assert_eq!(
            query,
            SelectQuery::from_entity::<SoftDeleteEntityUnderTest>()
        );
    }

    #[test]
    fn dbset_query_only_deleted_filters_nullable_indicator() {
        let dbset = DbSet::<SoftDeleteEntityUnderTest>::disconnected();

        let query = dbset
            .query()
            .only_deleted()
            .effective_select_query()
            .unwrap();

        assert_eq!(
            query,
            SelectQuery::from_entity::<SoftDeleteEntityUnderTest>().filter(Predicate::is_not_null(
                Expr::Column(sql_orm_query::ColumnRef::new(
                    TableRef::new("dbo", "soft_delete_entities"),
                    "deleted_at",
                    "deleted_at",
                ))
            ))
        );
    }

    #[test]
    fn dbset_query_uses_bool_indicator_when_soft_delete_column_is_bit() {
        let dbset = DbSet::<BoolSoftDeleteEntity>::disconnected();

        let active = dbset.query().effective_select_query().unwrap();
        let deleted = dbset
            .query()
            .only_deleted()
            .effective_select_query()
            .unwrap();

        assert_eq!(
            active,
            SelectQuery::from_entity::<BoolSoftDeleteEntity>().filter(Predicate::eq(
                Expr::Column(sql_orm_query::ColumnRef::new(
                    TableRef::new("dbo", "bool_soft_delete_entities"),
                    "is_deleted",
                    "is_deleted",
                )),
                Expr::Value(SqlValue::Bool(false)),
            ))
        );
        assert_eq!(
            deleted,
            SelectQuery::from_entity::<BoolSoftDeleteEntity>().filter(Predicate::eq(
                Expr::Column(sql_orm_query::ColumnRef::new(
                    TableRef::new("dbo", "bool_soft_delete_entities"),
                    "is_deleted",
                    "is_deleted",
                )),
                Expr::Value(SqlValue::Bool(true)),
            ))
        );
    }

    #[test]
    fn dbset_query_applies_active_tenant_filter_for_tenant_scoped_entities() {
        let query = DbSetQuery::<TenantEntity>::new(
            None,
            SelectQuery::from_entity::<TenantEntity>().filter(Predicate::eq(
                Expr::value(SqlValue::Bool(true)),
                Expr::value(SqlValue::Bool(true)),
            )),
        )
        .with_active_tenant_for_test(ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        })
        .effective_select_query()
        .unwrap();

        assert_eq!(
            query,
            SelectQuery::from_entity::<TenantEntity>()
                .filter(Predicate::eq(
                    Expr::value(SqlValue::Bool(true)),
                    Expr::value(SqlValue::Bool(true)),
                ))
                .filter(Predicate::eq(
                    Expr::Column(sql_orm_query::ColumnRef::new(
                        TableRef::new("sales", "tenant_entities"),
                        "tenant_id",
                        "tenant_id",
                    )),
                    Expr::Value(SqlValue::I64(42)),
                ))
        );
    }

    #[test]
    fn tenant_security_guardrail_keeps_joined_read_sql_tenant_scoped() {
        let query = DbSetQuery::<TenantEntity>::new(
            None,
            SelectQuery::from_entity::<TenantEntity>().inner_join::<JoinedEntity>(Predicate::eq(
                Expr::value(SqlValue::Bool(true)),
                Expr::value(SqlValue::Bool(true)),
            )),
        )
        .with_active_tenant_for_test(ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        })
        .effective_select_query()
        .unwrap();

        let compiled = SqlServerCompiler::compile_select(&query).unwrap();

        assert!(
            compiled.sql.contains("INNER JOIN [dbo].[joined_entities]"),
            "joined tenant read should preserve explicit joins: {}",
            compiled.sql
        );
        assert!(
            compiled
                .sql
                .contains("[sales].[tenant_entities].[tenant_id] = @P"),
            "joined tenant read must include tenant predicate on the root entity: {}",
            compiled.sql
        );
        assert!(
            compiled.params.contains(&SqlValue::I64(42)),
            "joined tenant read params must include active tenant value: {:?}",
            compiled.params
        );
    }

    #[test]
    fn dbset_query_fails_closed_without_active_tenant_for_tenant_scoped_entities() {
        let error =
            DbSetQuery::<TenantEntity>::new(None, SelectQuery::from_entity::<TenantEntity>())
                .effective_select_query()
                .unwrap_err();

        assert!(
            error
                .message()
                .contains("requires an active tenant in the DbContext")
        );
    }

    #[test]
    fn dbset_query_rejects_mismatched_active_tenant_column() {
        let error =
            DbSetQuery::<TenantEntity>::new(None, SelectQuery::from_entity::<TenantEntity>())
                .with_active_tenant_for_test(ActiveTenant {
                    column_name: "company_id",
                    value: SqlValue::I64(42),
                })
                .effective_select_query()
                .unwrap_err();

        assert!(error.message().contains("does not match"));
    }

    #[test]
    fn dbset_query_rejects_incompatible_active_tenant_value() {
        let error =
            DbSetQuery::<TenantEntity>::new(None, SelectQuery::from_entity::<TenantEntity>())
                .with_active_tenant_for_test(ActiveTenant {
                    column_name: "tenant_id",
                    value: SqlValue::String("not-a-bigint".to_string()),
                })
                .effective_select_query()
                .unwrap_err();

        assert!(error.message().contains("not compatible"));
    }

    #[test]
    fn tenant_value_type_matching_rejects_null_even_for_nullable_columns() {
        assert!(!tenant_value_matches_column_type(
            &SqlValue::Null,
            &TENANT_POLICY_COLUMNS[0],
        ));
    }

    #[test]
    fn dbset_query_order_by_builds_on_internal_select_query() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset.query().order_by(OrderBy::new(
            TableRef::new("dbo", "test_entities"),
            "created_at",
            SortDirection::Desc,
        ));

        assert_eq!(
            query.into_select_query(),
            SelectQuery::from_entity::<TestEntity>().order_by(OrderBy::new(
                TableRef::new("dbo", "test_entities"),
                "created_at",
                SortDirection::Desc,
            ))
        );
    }

    #[test]
    fn dbset_query_join_builds_on_internal_select_query() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let join = Join::left(
            TableRef::new("dbo", "joined_entities"),
            Predicate::eq(
                Expr::value(SqlValue::Bool(true)),
                Expr::value(SqlValue::Bool(true)),
            ),
        );

        let query = dbset.query().join(join.clone());

        assert_eq!(
            query.into_select_query(),
            SelectQuery::from_entity::<TestEntity>().join(join)
        );
    }

    #[test]
    fn dbset_query_exposes_entity_targeted_join_helpers() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset
            .query()
            .inner_join::<JoinedEntity>(Predicate::eq(
                Expr::value(SqlValue::Bool(true)),
                Expr::value(SqlValue::Bool(true)),
            ))
            .left_join::<JoinedEntity>(Predicate::eq(
                Expr::value(SqlValue::Bool(false)),
                Expr::value(SqlValue::Bool(false)),
            ));

        let select = query.into_select_query();

        assert_eq!(select.joins.len(), 2);
        assert_eq!(select.joins[0].join_type, JoinType::Inner);
        assert_eq!(
            select.joins[0].table,
            TableRef::new("dbo", "joined_entities")
        );
        assert_eq!(select.joins[1].join_type, JoinType::Left);
        assert_eq!(
            select.joins[1].table,
            TableRef::new("dbo", "joined_entities")
        );
    }

    #[test]
    fn dbset_query_infers_navigation_join_from_metadata() {
        let dbset = DbSet::<NavigationRoot>::disconnected();

        let select = dbset
            .query()
            .try_inner_join_navigation::<NavigationTarget>("orders")
            .unwrap()
            .into_select_query();

        assert_eq!(select.joins.len(), 1);
        assert_eq!(select.joins[0].join_type, JoinType::Inner);
        assert_eq!(
            select.joins[0].table,
            TableRef::new("sales", "navigation_targets")
        );
        assert_eq!(
            select.joins[0].on,
            Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::new("dbo", "navigation_roots"),
                    "id",
                    "id",
                )),
                Expr::Column(ColumnRef::new(
                    TableRef::new("sales", "navigation_targets"),
                    "owner_id",
                    "owner_id",
                )),
            )
        );
    }

    #[test]
    fn dbset_query_infers_aliased_navigation_join_from_metadata() {
        let dbset = DbSet::<NavigationRoot>::disconnected();

        let select = dbset
            .query()
            .try_left_join_navigation_as::<NavigationTarget>("orders", "orders")
            .unwrap()
            .into_select_query();

        assert_eq!(select.joins.len(), 1);
        assert_eq!(select.joins[0].join_type, JoinType::Left);
        assert_eq!(
            select.joins[0].table,
            TableRef::with_alias("sales", "navigation_targets", "orders")
        );
        assert_eq!(
            select.joins[0].on,
            Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::new("dbo", "navigation_roots"),
                    "id",
                    "id",
                )),
                Expr::Column(ColumnRef::new(
                    TableRef::with_alias("sales", "navigation_targets", "orders"),
                    "owner_id",
                    "owner_id",
                )),
            )
        );
    }

    #[test]
    fn dbset_query_rejects_unknown_navigation_join() {
        let error = DbSet::<NavigationRoot>::disconnected()
            .query()
            .try_inner_join_navigation::<NavigationTarget>("missing")
            .unwrap_err();

        assert!(
            error
                .message()
                .contains("does not declare navigation `missing`")
        );
    }

    #[test]
    fn dbset_query_rejects_navigation_join_target_mismatch() {
        let error = DbSet::<NavigationRoot>::disconnected()
            .query()
            .try_inner_join_navigation::<JoinedEntity>("orders")
            .unwrap_err();

        assert!(
            error
                .message()
                .contains("targets `sales.navigation_targets`")
        );
    }

    #[test]
    fn dbset_query_include_projects_root_and_prefixed_related_columns() {
        let include = DbSet::<NavigationTarget>::disconnected()
            .query()
            .include_as::<NavigationRoot>("owner", "owner")
            .unwrap();

        let select = include.select_query().unwrap();

        assert_eq!(select.joins.len(), 1);
        assert_eq!(select.joins[0].join_type, JoinType::Left);
        assert_eq!(
            select.joins[0].table,
            TableRef::with_alias("dbo", "navigation_roots", "owner")
        );
        assert_eq!(select.projection.len(), 3);
        assert_eq!(select.projection[0].alias, Some("id"));
        assert_eq!(select.projection[1].alias, Some("owner_id"));
        assert_eq!(select.projection[2].alias, Some("owner__id"));
    }

    #[test]
    fn compiled_include_sql_preserves_projection_aliases_soft_delete_and_params() {
        let include = DbSet::<NavigationTarget>::disconnected()
            .query()
            .include_as::<NavigationRoot>("owner", "owner")
            .unwrap()
            .filter(Predicate::gt(
                Expr::Column(ColumnRef::new(
                    TableRef::with_alias("dbo", "navigation_roots", "owner"),
                    "id",
                    "id",
                )),
                Expr::value(SqlValue::I64(7)),
            ))
            .order_by(OrderBy::new(
                TableRef::with_alias("dbo", "navigation_roots", "owner"),
                "id",
                SortDirection::Desc,
            ))
            .paginate(PageRequest::new(2, 10));

        let compiled = SqlServerCompiler::compile_select(&include.select_query().unwrap()).unwrap();

        assert_snapshot!(
            "compiled_include_one_with_soft_delete_and_parameters",
            render_compiled_query(&compiled)
        );
    }

    #[test]
    fn dbset_query_include_applies_included_soft_delete_filter_to_join_on() {
        let include = DbSet::<NavigationTarget>::disconnected()
            .query()
            .include_as::<NavigationRoot>("owner", "owner")
            .unwrap();

        let select = include.select_query().unwrap();

        assert_eq!(
            select.joins[0].on,
            Predicate::and(vec![
                Predicate::eq(
                    Expr::Column(ColumnRef::new(
                        TableRef::new("sales", "navigation_targets"),
                        "owner_id",
                        "owner_id",
                    )),
                    Expr::Column(ColumnRef::new(
                        TableRef::with_alias("dbo", "navigation_roots", "owner"),
                        "id",
                        "id",
                    )),
                ),
                Predicate::is_null(Expr::Column(ColumnRef::new(
                    TableRef::with_alias("dbo", "navigation_roots", "owner"),
                    "deleted_at",
                    "deleted_at",
                ))),
            ])
        );
    }

    #[test]
    fn dbset_query_include_applies_included_tenant_filter_to_join_on() {
        let include = DbSet::<TenantNavigationTarget>::disconnected()
            .query()
            .with_active_tenant_for_test(ActiveTenant {
                column_name: "tenant_id",
                value: SqlValue::I64(42),
            })
            .include_as::<TenantNavigationRoot>("owner", "owner")
            .unwrap();

        let select = include.select_query().unwrap();

        assert_eq!(
            select.joins[0].on,
            Predicate::and(vec![
                Predicate::eq(
                    Expr::Column(ColumnRef::new(
                        TableRef::new("sales", "tenant_navigation_targets"),
                        "owner_id",
                        "owner_id",
                    )),
                    Expr::Column(ColumnRef::new(
                        TableRef::with_alias("sales", "tenant_navigation_roots", "owner"),
                        "id",
                        "id",
                    )),
                ),
                Predicate::eq(
                    Expr::Column(ColumnRef::new(
                        TableRef::with_alias("sales", "tenant_navigation_roots", "owner"),
                        "tenant_id",
                        "tenant_id",
                    )),
                    Expr::Value(SqlValue::I64(42)),
                ),
            ])
        );
    }

    #[test]
    fn compiled_include_sql_preserves_included_tenant_parameter_order() {
        let include = DbSet::<TenantNavigationTarget>::disconnected()
            .query()
            .filter(Predicate::gt(
                Expr::Column(ColumnRef::new(
                    TableRef::new("sales", "tenant_navigation_targets"),
                    "id",
                    "id",
                )),
                Expr::value(SqlValue::I64(100)),
            ))
            .with_active_tenant_for_test(ActiveTenant {
                column_name: "tenant_id",
                value: SqlValue::I64(42),
            })
            .include_as::<TenantNavigationRoot>("owner", "owner")
            .unwrap();

        let compiled = SqlServerCompiler::compile_select(&include.select_query().unwrap()).unwrap();

        assert_snapshot!(
            "compiled_include_one_with_included_tenant_parameter_order",
            render_compiled_query(&compiled)
        );
        assert_eq!(compiled.params, vec![SqlValue::I64(42), SqlValue::I64(100)]);
    }

    #[test]
    fn dbset_query_include_fails_closed_for_included_tenant_without_active_tenant() {
        let include = DbSet::<TenantNavigationTarget>::disconnected()
            .query()
            .include_as::<TenantNavigationRoot>("owner", "owner")
            .unwrap();

        let error = include.select_query().unwrap_err();

        assert!(
            error
                .message()
                .contains("requires an active tenant in the DbContext")
        );
    }

    #[test]
    fn dbset_query_include_supports_chained_filter_order_and_pagination() {
        let include = DbSet::<NavigationTarget>::disconnected()
            .query()
            .include_as::<NavigationRoot>("owner", "owner")
            .unwrap()
            .filter(Predicate::gt(
                Expr::Column(ColumnRef::new(
                    TableRef::with_alias("dbo", "navigation_roots", "owner"),
                    "id",
                    "id",
                )),
                Expr::value(SqlValue::I64(0)),
            ))
            .order_by(OrderBy::new(
                TableRef::with_alias("dbo", "navigation_roots", "owner"),
                "id",
                SortDirection::Desc,
            ))
            .paginate(PageRequest::new(2, 10));

        let select = include.select_query().unwrap();

        assert_eq!(
            select.predicate,
            Some(Predicate::gt(
                Expr::Column(ColumnRef::new(
                    TableRef::with_alias("dbo", "navigation_roots", "owner"),
                    "id",
                    "id",
                )),
                Expr::value(SqlValue::I64(0)),
            ))
        );
        assert_eq!(
            select.order_by,
            vec![OrderBy::new(
                TableRef::with_alias("dbo", "navigation_roots", "owner"),
                "id",
                SortDirection::Desc,
            )]
        );
        assert_eq!(select.pagination, Some(Pagination::new(10, 10)));
    }

    #[test]
    fn dbset_query_include_rejects_collection_navigation() {
        let result = DbSet::<NavigationRoot>::disconnected()
            .query()
            .include::<NavigationTarget>("orders");
        let error = match result {
            Ok(_) => panic!("expected collection include to be rejected"),
            Err(error) => error,
        };

        assert!(error.message().contains("belongs_to and has_one"));
    }

    #[test]
    fn dbset_query_include_many_projects_root_and_prefixed_related_columns() {
        let include = DbSet::<NavigationRoot>::disconnected()
            .query()
            .include_many_as::<NavigationTarget>("orders", "orders")
            .unwrap();

        let select = include.select_query().unwrap();

        assert_eq!(select.joins.len(), 1);
        assert_eq!(select.joins[0].join_type, JoinType::Left);
        assert_eq!(
            select.joins[0].table,
            TableRef::with_alias("sales", "navigation_targets", "orders")
        );
        assert_eq!(select.projection.len(), 3);
        assert_eq!(select.projection[0].alias, Some("id"));
        assert_eq!(select.projection[1].alias, Some("orders__id"));
        assert_eq!(select.projection[2].alias, Some("orders__owner_id"));
    }

    #[test]
    fn compiled_include_many_sql_preserves_grouping_projection_and_root_soft_delete() {
        let include = DbSet::<NavigationRoot>::disconnected()
            .query()
            .include_many_as::<NavigationTarget>("orders", "orders")
            .unwrap()
            .filter(Predicate::gte(
                Expr::Column(ColumnRef::new(
                    TableRef::with_alias("sales", "navigation_targets", "orders"),
                    "owner_id",
                    "owner_id",
                )),
                Expr::value(SqlValue::I64(7)),
            ))
            .order_by(OrderBy::new(
                TableRef::with_alias("sales", "navigation_targets", "orders"),
                "id",
                SortDirection::Asc,
            ));

        let compiled = SqlServerCompiler::compile_select(&include.select_query().unwrap()).unwrap();

        assert_snapshot!(
            "compiled_include_many_with_root_soft_delete_and_parameters",
            render_compiled_query(&compiled)
        );
        assert_eq!(compiled.params, vec![SqlValue::I64(7)]);
    }

    #[test]
    fn dbset_query_include_many_rejects_non_collection_navigation() {
        let result = DbSet::<NavigationTarget>::disconnected()
            .query()
            .include_many::<NavigationRoot>("owner");
        let error = match result {
            Ok(_) => panic!("expected non-collection include_many to be rejected"),
            Err(error) => error,
        };

        assert!(error.message().contains("has_many"));
    }

    #[test]
    fn dbset_query_include_many_rejects_pagination_for_join_grouping() {
        let include = DbSet::<NavigationRoot>::disconnected()
            .query()
            .take(10)
            .include_many_as::<NavigationTarget>("orders", "orders")
            .unwrap();

        let error = include.select_query().unwrap_err();

        assert!(error.message().contains("does not support pagination"));
    }

    #[tokio::test]
    async fn dbset_query_include_many_split_query_reports_explicit_error() {
        let error = DbSet::<NavigationRoot>::disconnected()
            .query()
            .include_many_as::<NavigationTarget>("orders", "orders")
            .unwrap()
            .split_query()
            .all()
            .await
            .unwrap_err();

        assert!(
            error
                .message()
                .contains("split-query loading is not implemented yet")
        );
    }

    #[test]
    fn include_many_join_row_limit_reports_clear_error() {
        let error = enforce_include_many_join_row_limit(11, Some(10)).unwrap_err();

        assert!(error.message().contains("produced 11 rows"));
        assert!(error.message().contains("configured limit of 10"));
    }

    #[test]
    fn include_many_join_row_limit_allows_explicit_unbounded_join() {
        enforce_include_many_join_row_limit(usize::MAX, None).unwrap();
    }

    #[test]
    fn include_navigation_identity_map_helper_reuses_tracked_snapshot_without_registering() {
        let registry = std::sync::Arc::new(TrackingRegistry::default());
        let mut tracked = Tracked::from_loaded(NavigationTarget { id: 7, owner_id: 1 });
        tracked
            .attach_registry_loaded(std::sync::Arc::clone(&registry), SqlValue::I64(7))
            .unwrap();
        tracked.current_mut().owner_id = 99;

        let materialized = NavigationTarget { id: 7, owner_id: 1 };
        let mapped =
            identity_mapped_navigation_value(Some(&registry), Some(SqlValue::I64(7)), materialized);

        assert_eq!(mapped.owner_id, 99);
        assert_eq!(registry.tracked_for::<NavigationTarget>().len(), 1);

        let ordinary = identity_mapped_navigation_value(
            Some(&registry),
            Some(SqlValue::I64(8)),
            NavigationTarget { id: 8, owner_id: 2 },
        );
        assert_eq!(ordinary.owner_id, 2);
        assert_eq!(registry.tracked_for::<NavigationTarget>().len(), 1);
    }

    #[test]
    fn compiled_self_join_sql_preserves_repeated_aliases_and_parameter_order() {
        let query = SelectQuery::from_entity_as::<TestEntity>("root")
            .select(vec![
                SelectProjection::expr_as(Expr::column_as(TestEntity::id, "root"), "root_id"),
                SelectProjection::expr_as(
                    Expr::column_as(TestEntity::name, "parent"),
                    "parent_name",
                ),
                SelectProjection::expr_as(Expr::column_as(TestEntity::name, "child"), "child_name"),
            ])
            .inner_join_as::<TestEntity>(
                "parent",
                Predicate::eq(
                    Expr::column_as(TestEntity::id, "root"),
                    Expr::column_as(TestEntity::id, "parent"),
                ),
            )
            .left_join_as::<TestEntity>(
                "child",
                Predicate::eq(
                    Expr::column_as(TestEntity::id, "root"),
                    Expr::column_as(TestEntity::id, "child"),
                ),
            )
            .filter(Predicate::like(
                Expr::column_as(TestEntity::name, "parent"),
                Expr::value(SqlValue::String("%admin%".to_string())),
            ))
            .filter(Predicate::gte(
                Expr::column_as(TestEntity::id, "child"),
                Expr::value(SqlValue::I64(10)),
            ))
            .order_by(OrderBy::new(
                TableRef::with_alias("dbo", "test_entities", "child"),
                "name",
                SortDirection::Asc,
            ))
            .paginate(Pagination::new(20, 10));

        let compiled = SqlServerCompiler::compile_select(&query).unwrap();

        assert_snapshot!(
            "compiled_self_join_repeated_aliases_and_parameter_order",
            render_compiled_query(&compiled)
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::String("%admin%".to_string()),
                SqlValue::I64(10),
                SqlValue::I64(20),
                SqlValue::I64(10),
            ]
        );
    }

    #[test]
    fn dbset_query_supports_chaining_filter_and_order_by() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset
            .query()
            .filter(Predicate::eq(
                Expr::value(SqlValue::Bool(true)),
                Expr::value(SqlValue::Bool(true)),
            ))
            .order_by(OrderBy::new(
                TableRef::new("dbo", "test_entities"),
                "created_at",
                SortDirection::Asc,
            ));

        assert_eq!(
            query.into_select_query(),
            SelectQuery::from_entity::<TestEntity>()
                .filter(Predicate::eq(
                    Expr::value(SqlValue::Bool(true)),
                    Expr::value(SqlValue::Bool(true)),
                ))
                .order_by(OrderBy::new(
                    TableRef::new("dbo", "test_entities"),
                    "created_at",
                    SortDirection::Asc,
                ))
        );
    }

    #[test]
    fn dbset_query_limit_builds_zero_offset_pagination() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset.query().limit(25);

        assert_eq!(
            query.into_select_query(),
            SelectQuery::from_entity::<TestEntity>().paginate(Pagination::new(0, 25))
        );
    }

    #[test]
    fn dbset_query_take_is_alias_for_limit() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let limited = dbset.query().limit(10).into_select_query();
        let taken = dbset.query().take(10).into_select_query();

        assert_eq!(limited, taken);
    }

    #[test]
    fn dbset_query_paginate_uses_page_request_contract() {
        let dbset = DbSet::<TestEntity>::disconnected();

        let query = dbset.query().paginate(PageRequest::new(3, 25));

        assert_eq!(
            query.into_select_query(),
            SelectQuery::from_entity::<TestEntity>().paginate(Pagination::new(50, 25))
        );
    }

    #[test]
    fn count_row_accepts_i32_and_i64_results() {
        struct CountTestRow {
            value: SqlValue,
        }

        impl Row for CountTestRow {
            fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
                Ok((column == "count").then(|| self.value.clone()))
            }
        }

        let from_i32 = super::CountRow::from_row(&CountTestRow {
            value: SqlValue::I32(7),
        })
        .unwrap();
        let from_i64 = super::CountRow::from_row(&CountTestRow {
            value: SqlValue::I64(9),
        })
        .unwrap();

        assert_eq!(from_i32.value, 7);
        assert_eq!(from_i64.value, 9);
    }

    #[test]
    fn count_row_rejects_non_integer_results() {
        struct CountTestRow;

        impl Row for CountTestRow {
            fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
                Ok((column == "count").then(|| SqlValue::String("7".to_string())))
            }
        }

        let error = super::CountRow::from_row(&CountTestRow).unwrap_err();

        assert_eq!(
            error.message(),
            "expected SQL Server COUNT result as i32 or i64"
        );
    }

    #[test]
    fn exists_query_preserves_joins_and_effective_filters() {
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };
        let dbset = DbSet::<TenantEntity>::disconnected();
        let query = dbset
            .query()
            .with_active_tenant_for_test(active_tenant)
            .inner_join::<JoinedEntity>(Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::for_entity::<TenantEntity>(),
                    "tenant_id",
                    "tenant_id",
                )),
                Expr::Column(ColumnRef::new(
                    TableRef::for_entity::<JoinedEntity>(),
                    "tenant_id",
                    "tenant_id",
                )),
            ))
            .filter(Predicate::eq(
                Expr::Column(ColumnRef::new(
                    TableRef::for_entity::<TenantEntity>(),
                    "tenant_id",
                    "tenant_id",
                )),
                Expr::value(SqlValue::I64(7)),
            ));

        let exists = query.exists_query().unwrap();

        assert_eq!(exists.joins.len(), 1);
        let compiled = SqlServerCompiler::compile_exists(&exists).unwrap();
        assert!(compiled.sql.contains("INNER JOIN [dbo].[joined_entities]"));
        assert!(
            compiled
                .sql
                .contains("[sales].[tenant_entities].[tenant_id] = @P1")
        );
        assert!(
            compiled
                .sql
                .contains("[sales].[tenant_entities].[tenant_id] = @P2")
        );
        assert_eq!(compiled.params, vec![SqlValue::I64(7), SqlValue::I64(42)]);
    }

    #[test]
    fn scalar_aggregate_query_preserves_soft_delete_filter_and_ignores_pagination() {
        let dbset = DbSet::<SoftDeleteEntityUnderTest>::disconnected();
        let query = dbset
            .query()
            .filter(Predicate::eq(
                Expr::value(SqlValue::Bool(true)),
                Expr::value(SqlValue::Bool(true)),
            ))
            .order_by(OrderBy::new(
                TableRef::for_entity::<SoftDeleteEntityUnderTest>(),
                "deleted_at",
                SortDirection::Desc,
            ))
            .limit(5);

        let aggregate = query
            .scalar_aggregate_query(AggregateProjection::expr_as(
                AggregateExpr::max(Expr::Column(ColumnRef::new(
                    TableRef::for_entity::<SoftDeleteEntityUnderTest>(),
                    "deleted_at",
                    "deleted_at",
                ))),
                "value",
            ))
            .unwrap();

        assert!(aggregate.order_by.is_empty());
        assert!(aggregate.pagination.is_none());
        let compiled = SqlServerCompiler::compile_aggregate(&aggregate).unwrap();
        assert!(
            compiled
                .sql
                .starts_with("SELECT MAX([dbo].[soft_delete_entities].[deleted_at]) AS [value]")
        );
        assert!(
            compiled
                .sql
                .contains("[dbo].[soft_delete_entities].[deleted_at] IS NULL")
        );
        assert!(!compiled.sql.contains("ORDER BY"));
        assert!(!compiled.sql.contains("OFFSET"));
        assert_eq!(
            compiled.params,
            vec![SqlValue::Bool(true), SqlValue::Bool(true)]
        );
    }

    #[test]
    fn scalar_aggregate_query_preserves_explicit_join_and_aliased_column() {
        let query = DbSet::<NavigationRoot>::disconnected()
            .query()
            .try_left_join_navigation_as::<NavigationTarget>("orders", "orders")
            .unwrap()
            .filter(Predicate::gt(
                Expr::from(NavigationTarget::owner_id.aliased("orders")),
                Expr::value(SqlValue::I64(10)),
            ));

        let aggregate = query
            .scalar_aggregate_query(AggregateProjection::expr_as(
                AggregateExpr::max(NavigationTarget::id.aliased("orders")),
                "value",
            ))
            .unwrap();

        assert_eq!(aggregate.joins.len(), 1);
        assert_eq!(
            aggregate.joins[0].table,
            TableRef::with_alias("sales", "navigation_targets", "orders")
        );

        let compiled = SqlServerCompiler::compile_aggregate(&aggregate).unwrap();
        assert!(compiled.sql.contains(
            "LEFT JOIN [sales].[navigation_targets] AS [orders] ON ([dbo].[navigation_roots].[id] = [orders].[owner_id])"
        ));
        assert!(
            compiled
                .sql
                .starts_with("SELECT MAX([orders].[id]) AS [value]")
        );
        assert!(compiled.sql.contains("WHERE ("));
        assert!(compiled.sql.contains("[orders].[owner_id] > @P1"));
        assert!(
            compiled
                .sql
                .contains("[dbo].[navigation_roots].[deleted_at] IS NULL")
        );
        assert_eq!(compiled.params, vec![SqlValue::I64(10)]);
    }

    #[test]
    fn grouped_query_builds_aggregate_ast_with_projection_having_order_and_pagination() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let grouped = dbset
            .query()
            .filter(Predicate::eq(
                Expr::value(SqlValue::Bool(true)),
                Expr::value(SqlValue::Bool(true)),
            ))
            .group_by(TestEntity::id)
            .unwrap()
            .select_aggregate((
                AggregateProjection::group_key(TestEntity::id),
                AggregateProjection::count_as("entity_count"),
            ))
            .having(AggregatePredicate::gt(
                AggregateExpr::count_all(),
                Expr::value(SqlValue::I64(1)),
            ))
            .order_by(AggregateOrderBy::desc(AggregateExpr::count_all()))
            .paginate(PageRequest::new(2, 10));

        let aggregate = grouped.aggregate_query();

        assert_eq!(aggregate.group_by, vec![Expr::from(TestEntity::id)]);
        assert_eq!(aggregate.projection.len(), 2);
        assert!(matches!(
            aggregate.having,
            Some(AggregatePredicate::Gt(_, _))
        ));
        assert_eq!(aggregate.order_by.len(), 1);
        assert_eq!(aggregate.pagination, Some(Pagination::new(10, 10)));

        let compiled = SqlServerCompiler::compile_aggregate(aggregate).unwrap();
        assert!(compiled.sql.contains("COUNT(*) AS [entity_count]"));
        assert!(compiled.sql.contains("GROUP BY [dbo].[test_entities].[id]"));
        assert!(compiled.sql.contains("HAVING (COUNT(*) > @P3)"));
        assert!(compiled.sql.contains("ORDER BY COUNT(*) DESC"));
        assert!(
            compiled
                .sql
                .contains("OFFSET @P4 ROWS FETCH NEXT @P5 ROWS ONLY")
        );
        assert_eq!(
            compiled.params,
            vec![
                SqlValue::Bool(true),
                SqlValue::Bool(true),
                SqlValue::I64(1),
                SqlValue::I64(10),
                SqlValue::I64(10),
            ]
        );
    }

    #[test]
    fn grouped_query_preserves_explicit_navigation_alias_join() {
        let grouped = DbSet::<NavigationRoot>::disconnected()
            .query()
            .try_left_join_navigation_as::<NavigationTarget>("orders", "orders")
            .unwrap()
            .filter(Predicate::gt(
                Expr::from(NavigationTarget::owner_id.aliased("orders")),
                Expr::value(SqlValue::I64(10)),
            ))
            .group_by(NavigationTarget::owner_id.aliased("orders"))
            .unwrap()
            .select_aggregate((
                NavigationTarget::owner_id.aliased("orders"),
                AggregateProjection::count_as("order_count"),
            ))
            .having(AggregatePredicate::gt(
                AggregateExpr::count_all(),
                Expr::value(SqlValue::I64(1)),
            ))
            .order_by(AggregateOrderBy::desc(AggregateExpr::count_all()));

        let aggregate = grouped.aggregate_query();

        assert_eq!(aggregate.joins.len(), 1);
        assert_eq!(
            aggregate.group_by,
            vec![Expr::from(NavigationTarget::owner_id.aliased("orders"))]
        );
        assert_eq!(aggregate.projection[0].alias, "owner_id");

        let compiled = SqlServerCompiler::compile_aggregate(aggregate).unwrap();
        assert!(compiled.sql.contains(
            "LEFT JOIN [sales].[navigation_targets] AS [orders] ON ([dbo].[navigation_roots].[id] = [orders].[owner_id])"
        ));
        assert!(compiled.sql.contains("[orders].[owner_id] AS [owner_id]"));
        assert!(compiled.sql.contains("COUNT(*) AS [order_count]"));
        assert!(compiled.sql.contains("GROUP BY [orders].[owner_id]"));
        assert!(compiled.sql.contains("HAVING (COUNT(*) > @P2)"));
        assert!(compiled.sql.contains("ORDER BY COUNT(*) DESC"));
        assert!(
            compiled
                .sql
                .contains("[dbo].[navigation_roots].[deleted_at] IS NULL")
        );
        assert_eq!(compiled.params, vec![SqlValue::I64(10), SqlValue::I64(1)]);
    }

    #[test]
    fn grouped_query_does_not_infer_hidden_join_from_aliased_group_key() {
        let grouped = DbSet::<NavigationRoot>::disconnected()
            .query()
            .group_by(NavigationTarget::owner_id.aliased("orders"))
            .unwrap()
            .select_aggregate((
                NavigationTarget::owner_id.aliased("orders"),
                AggregateProjection::count_as("order_count"),
            ));

        let aggregate = grouped.aggregate_query();

        assert!(aggregate.joins.is_empty());
        assert_eq!(
            aggregate.group_by,
            vec![Expr::from(NavigationTarget::owner_id.aliased("orders"))]
        );

        let compiled = SqlServerCompiler::compile_aggregate(aggregate).unwrap();
        assert!(!compiled.sql.contains(" JOIN "));
        assert!(compiled.sql.contains("GROUP BY [orders].[owner_id]"));
    }

    #[test]
    fn grouped_query_preserves_root_tenant_and_soft_delete_filters() {
        let active_tenant = ActiveTenant {
            column_name: "tenant_id",
            value: SqlValue::I64(42),
        };
        let dbset = DbSet::<TenantEntity>::disconnected();
        let grouped = dbset
            .query()
            .with_active_tenant_for_test(active_tenant)
            .group_by(TenantEntity::tenant_id)
            .unwrap()
            .select_aggregate((
                AggregateProjection::group_key(TenantEntity::tenant_id),
                AggregateProjection::count_as("tenant_count"),
            ));

        let compiled = SqlServerCompiler::compile_aggregate(grouped.aggregate_query()).unwrap();

        assert!(
            compiled
                .sql
                .contains("GROUP BY [sales].[tenant_entities].[tenant_id]")
        );
        assert!(
            compiled
                .sql
                .contains("[sales].[tenant_entities].[tenant_id] = @P1")
        );
        assert_eq!(compiled.params, vec![SqlValue::I64(42)]);
    }

    #[test]
    fn grouped_query_rejects_empty_group_by_early() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let error = dbset.query().group_by(Vec::<Expr>::new()).unwrap_err();

        assert_eq!(
            error.message(),
            "group_by requires at least one group key expression"
        );
    }

    #[test]
    fn grouped_query_debug_mentions_entity_type() {
        let dbset = DbSet::<TestEntity>::disconnected();
        let grouped = dbset
            .query()
            .group_by(TestEntity::id)
            .unwrap()
            .select_aggregate(AggregateProjection::count_as("entity_count"));

        let rendered = format!("{grouped:?}");

        assert!(rendered.contains("DbSetGroupedQuery"));
        assert!(rendered.contains("test_entities"));
    }

    #[test]
    fn scalar_aggregate_row_materializes_values_and_nulls() {
        struct AggregateTestRow {
            value: Option<SqlValue>,
        }

        impl Row for AggregateTestRow {
            fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
                Ok((column == "value").then(|| self.value.clone()).flatten())
            }
        }

        let from_value = super::ScalarAggregateRow::<i64>::from_row(&AggregateTestRow {
            value: Some(SqlValue::I64(12)),
        })
        .unwrap();
        let from_null = super::ScalarAggregateRow::<i64>::from_row(&AggregateTestRow {
            value: Some(SqlValue::Null),
        })
        .unwrap();
        let missing = super::ScalarAggregateRow::<i64>::from_row(&AggregateTestRow { value: None })
            .unwrap_err();

        assert_eq!(from_value.value, Some(12));
        assert_eq!(from_null.value, None);
        assert_eq!(
            missing.message(),
            "scalar aggregate result column was not present"
        );
    }

    #[test]
    fn scalar_aggregate_row_validates_supported_return_types_strictly() {
        struct AggregateTestRow {
            value: SqlValue,
        }

        impl Row for AggregateTestRow {
            fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
                Ok((column == "value").then(|| self.value.clone()))
            }
        }

        let date = NaiveDate::from_ymd_opt(2026, 5, 17).expect("valid date");
        let datetime: NaiveDateTime = date.and_hms_opt(9, 30, 0).expect("valid datetime");
        let decimal = Decimal::new(12345, 2);

        let i32_row = super::ScalarAggregateRow::<i32>::from_row(&AggregateTestRow {
            value: SqlValue::I32(7),
        })
        .unwrap();
        let i64_row = super::ScalarAggregateRow::<i64>::from_row(&AggregateTestRow {
            value: SqlValue::I64(9),
        })
        .unwrap();
        let f64_row = super::ScalarAggregateRow::<f64>::from_row(&AggregateTestRow {
            value: SqlValue::F64(10.5),
        })
        .unwrap();
        let decimal_row = super::ScalarAggregateRow::<Decimal>::from_row(&AggregateTestRow {
            value: SqlValue::Decimal(decimal),
        })
        .unwrap();
        let string_row = super::ScalarAggregateRow::<String>::from_row(&AggregateTestRow {
            value: SqlValue::String("last".to_string()),
        })
        .unwrap();
        let date_row = super::ScalarAggregateRow::<NaiveDate>::from_row(&AggregateTestRow {
            value: SqlValue::Date(date),
        })
        .unwrap();
        let datetime_row =
            super::ScalarAggregateRow::<NaiveDateTime>::from_row(&AggregateTestRow {
                value: SqlValue::DateTime(datetime),
            })
            .unwrap();
        let mismatch = super::ScalarAggregateRow::<i64>::from_row(&AggregateTestRow {
            value: SqlValue::I32(7),
        })
        .unwrap_err();

        assert_eq!(i32_row.value, Some(7));
        assert_eq!(i64_row.value, Some(9));
        assert_eq!(f64_row.value, Some(10.5));
        assert_eq!(decimal_row.value, Some(decimal));
        assert_eq!(string_row.value, Some("last".to_string()));
        assert_eq!(date_row.value, Some(date));
        assert_eq!(datetime_row.value, Some(datetime));
        assert_eq!(mismatch.message(), "expected i64 value");
    }

    #[test]
    fn exists_row_materializes_bool_result() {
        struct ExistsTestRow;

        impl Row for ExistsTestRow {
            fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
                Ok((column == "exists").then_some(SqlValue::Bool(true)))
            }
        }

        let row = super::ExistsRow::from_row(&ExistsTestRow).unwrap();

        assert!(row.value);
    }

    #[test]
    fn debug_mentions_entity_type() {
        let query = DbSetQuery::<TestEntity>::new(None, SelectQuery::from_entity::<TestEntity>());

        let rendered = format!("{query:?}");

        assert!(rendered.contains("DbSetQuery"));
        assert!(rendered.contains("test_entities"));
    }

    fn render_compiled_query(compiled: &CompiledQuery) -> String {
        let params = compiled
            .params
            .iter()
            .enumerate()
            .map(|(index, value)| format!("{}: {:?}", index + 1, value))
            .collect::<Vec<_>>();

        if params.is_empty() {
            format!("SQL: {}\nParams:\n<none>", compiled.sql)
        } else {
            format!("SQL: {}\nParams:\n{}", compiled.sql, params.join("\n"))
        }
    }
}
