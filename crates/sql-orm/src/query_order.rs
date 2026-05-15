use crate::AliasedEntityColumn;
use sql_orm_core::{Entity, EntityColumn};
use sql_orm_query::{OrderBy, SortDirection};

pub trait EntityColumnOrderExt<E: Entity> {
    fn asc(self) -> OrderBy;

    fn desc(self) -> OrderBy;
}

impl<E: Entity> EntityColumnOrderExt<E> for EntityColumn<E> {
    fn asc(self) -> OrderBy {
        OrderBy::asc(self)
    }

    fn desc(self) -> OrderBy {
        OrderBy::desc(self)
    }
}

impl<E: Entity> EntityColumnOrderExt<E> for AliasedEntityColumn<E> {
    fn asc(self) -> OrderBy {
        let column = self.column_ref();
        OrderBy::new(column.table, column.column_name, SortDirection::Asc)
    }

    fn desc(self) -> OrderBy {
        let column = self.column_ref();
        OrderBy::new(column.table, column.column_name, SortDirection::Desc)
    }
}

#[cfg(test)]
mod tests {
    use super::EntityColumnOrderExt;
    use crate::EntityColumnAliasExt;
    use sql_orm_core::{
        ColumnMetadata, Entity, EntityColumn, EntityMetadata, PrimaryKeyMetadata, SqlServerType,
    };
    use sql_orm_query::{OrderBy, SortDirection, TableRef};

    struct TestEntity;

    static TEST_ENTITY_COLUMNS: [ColumnMetadata; 2] = [
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
            rust_field: "created_at",
            column_name: "created_at",
            renamed_from: None,
            sql_type: SqlServerType::DateTime2,
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

    static TEST_ENTITY_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "TestEntity",
        schema: "dbo",
        table: "test_entities",
        renamed_from: None,
        columns: &TEST_ENTITY_COLUMNS,
        primary_key: PrimaryKeyMetadata {
            name: None,
            columns: &["id"],
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
        const created_at: EntityColumn<TestEntity> = EntityColumn::new("created_at", "created_at");
    }

    #[test]
    fn ordering_methods_build_expected_order_by_values() {
        assert_eq!(
            TestEntity::id.asc(),
            OrderBy::new(
                TableRef::new("dbo", "test_entities"),
                "id",
                SortDirection::Asc
            )
        );
        assert_eq!(
            TestEntity::created_at.desc(),
            OrderBy::new(
                TableRef::new("dbo", "test_entities"),
                "created_at",
                SortDirection::Desc
            )
        );
    }

    #[test]
    fn aliased_columns_build_order_by_against_table_alias() {
        assert_eq!(
            TestEntity::created_at.aliased("t").desc(),
            OrderBy::new(
                TableRef::with_alias("dbo", "test_entities", "t"),
                "created_at",
                SortDirection::Desc
            )
        );
    }
}
