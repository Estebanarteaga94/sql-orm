use sql_orm_core::{Entity, EntityColumn};
use sql_orm_query::{ColumnRef, Expr, SelectProjection, TableRef};
use std::marker::PhantomData;

/// Public column reference bound to a SQL table alias.
#[derive(Debug)]
pub struct AliasedEntityColumn<E: Entity> {
    rust_field: &'static str,
    column_name: &'static str,
    alias: &'static str,
    _entity: PhantomData<fn() -> E>,
}

impl<E: Entity> Copy for AliasedEntityColumn<E> {}

impl<E: Entity> Clone for AliasedEntityColumn<E> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<E: Entity> PartialEq for AliasedEntityColumn<E> {
    fn eq(&self, other: &Self) -> bool {
        self.rust_field == other.rust_field
            && self.column_name == other.column_name
            && self.alias == other.alias
    }
}

impl<E: Entity> Eq for AliasedEntityColumn<E> {}

impl<E: Entity> AliasedEntityColumn<E> {
    pub const fn new(column: EntityColumn<E>, alias: &'static str) -> Self {
        Self {
            rust_field: column.rust_field(),
            column_name: column.column_name(),
            alias,
            _entity: PhantomData,
        }
    }

    pub const fn column(&self) -> EntityColumn<E> {
        EntityColumn::new(self.rust_field, self.column_name)
    }

    pub const fn alias(&self) -> &'static str {
        self.alias
    }

    pub fn column_ref(self) -> ColumnRef {
        ColumnRef::new(
            TableRef::for_entity_as::<E>(self.alias),
            self.rust_field,
            self.column_name,
        )
    }

    pub fn expr(self) -> Expr {
        Expr::Column(self.column_ref())
    }

    pub fn table_ref(self) -> TableRef {
        TableRef::for_entity_as::<E>(self.alias)
    }
}

impl<E: Entity> From<AliasedEntityColumn<E>> for ColumnRef {
    fn from(value: AliasedEntityColumn<E>) -> Self {
        value.column_ref()
    }
}

impl<E: Entity> From<AliasedEntityColumn<E>> for Expr {
    fn from(value: AliasedEntityColumn<E>) -> Self {
        value.expr()
    }
}

impl<E: Entity> From<AliasedEntityColumn<E>> for SelectProjection {
    fn from(value: AliasedEntityColumn<E>) -> Self {
        let column_name = value.column_name;
        SelectProjection::expr_as(value.expr(), column_name)
    }
}

pub trait EntityColumnAliasExt<E: Entity> {
    fn aliased(self, alias: &'static str) -> AliasedEntityColumn<E>;
}

impl<E: Entity> EntityColumnAliasExt<E> for EntityColumn<E> {
    fn aliased(self, alias: &'static str) -> AliasedEntityColumn<E> {
        AliasedEntityColumn::new(self, alias)
    }
}

#[cfg(test)]
mod tests {
    use super::{AliasedEntityColumn, EntityColumnAliasExt};
    use sql_orm_core::{
        ColumnMetadata, Entity, EntityColumn, EntityMetadata, PrimaryKeyMetadata, SqlServerType,
    };
    use sql_orm_query::{ColumnRef, Expr, SelectProjection, TableRef};

    struct TestEntity;

    static TEST_ENTITY_COLUMNS: [ColumnMetadata; 1] = [ColumnMetadata {
        rust_field: "name",
        column_name: "name",
        renamed_from: None,
        sql_type: SqlServerType::NVarChar,
        nullable: false,
        primary_key: true,
        identity: None,
        default_sql: None,
        computed_sql: None,
        rowversion: false,
        insertable: false,
        updatable: false,
        max_length: Some(120),
        precision: None,
        scale: None,
    }];

    static TEST_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TestEntity",
        schema: "dbo",
        table: "test_entities",
        renamed_from: None,
        columns: &TEST_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["name"],
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
        const name: EntityColumn<TestEntity> = EntityColumn::new("name", "name");
    }

    #[test]
    fn entity_column_can_be_bound_to_table_alias() {
        let aliased: AliasedEntityColumn<TestEntity> = TestEntity::name.aliased("t");

        assert_eq!(aliased.alias(), "t");
        assert_eq!(aliased.column().rust_field(), TestEntity::name.rust_field());
        assert_eq!(
            aliased.column().column_name(),
            TestEntity::name.column_name()
        );
        assert_eq!(
            aliased.table_ref(),
            TableRef::with_alias("dbo", "test_entities", "t")
        );
        assert_eq!(
            aliased.column_ref(),
            ColumnRef::new(
                TableRef::with_alias("dbo", "test_entities", "t"),
                "name",
                "name"
            )
        );
        assert_eq!(Expr::from(aliased), Expr::Column(aliased.column_ref()));
        assert_eq!(
            SelectProjection::from(aliased),
            SelectProjection::expr_as(Expr::Column(aliased.column_ref()), "name")
        );
    }
}
