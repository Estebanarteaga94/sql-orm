use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use sql_orm_core::{
    ColumnMetadata, EntityMetadata, ForeignKeyMetadata, IdentityMetadata, IndexColumnMetadata,
    IndexMetadata, ReferentialAction, SqlServerType,
};
use std::collections::BTreeMap;

/// Serializable model snapshot shape used by future migration history artifacts.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModelSnapshot {
    pub schemas: Vec<SchemaSnapshot>,
}

impl ModelSnapshot {
    pub fn new(schemas: Vec<SchemaSnapshot>) -> Self {
        Self { schemas }
    }

    pub fn from_entities(entities: &[&'static EntityMetadata]) -> Self {
        let mut schemas = BTreeMap::<String, Vec<&'static EntityMetadata>>::new();

        for entity in entities {
            schemas
                .entry(entity.schema.to_string())
                .or_default()
                .push(*entity);
        }

        let schemas = schemas
            .into_iter()
            .map(|(schema_name, mut entities)| {
                entities.sort_by(|left, right| left.table.cmp(right.table));

                SchemaSnapshot::new(
                    schema_name,
                    entities.into_iter().map(TableSnapshot::from).collect(),
                )
            })
            .collect();

        Self { schemas }
    }

    pub fn schema(&self, name: &str) -> Option<&SchemaSnapshot> {
        self.schemas.iter().find(|schema| schema.name == name)
    }

    pub fn to_json_pretty(&self) -> Result<String, sql_orm_core::OrmError> {
        serde_json::to_string_pretty(self)
            .map(|json| format!("{json}\n"))
            .map_err(|_| sql_orm_core::OrmError::migration("failed to serialize model snapshot"))
    }

    pub fn from_json(json: &str) -> Result<Self, sql_orm_core::OrmError> {
        serde_json::from_str(json)
            .map_err(|_| sql_orm_core::OrmError::migration("failed to deserialize model snapshot"))
    }
}

/// Snapshot of a SQL Server schema and the tables currently modeled inside it.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct SchemaSnapshot {
    pub name: String,
    pub tables: Vec<TableSnapshot>,
}

impl SchemaSnapshot {
    pub fn new(name: impl Into<String>, tables: Vec<TableSnapshot>) -> Self {
        Self {
            name: name.into(),
            tables,
        }
    }

    pub fn table(&self, name: &str) -> Option<&TableSnapshot> {
        self.tables.iter().find(|table| table.name == name)
    }
}

/// Snapshot of a SQL Server table with the minimum structural information needed
/// for the first migration diff passes.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct TableSnapshot {
    pub name: String,
    pub renamed_from: Option<String>,
    pub columns: Vec<ColumnSnapshot>,
    pub primary_key_name: Option<String>,
    pub primary_key_columns: Vec<String>,
    pub indexes: Vec<IndexSnapshot>,
    pub foreign_keys: Vec<ForeignKeySnapshot>,
}

impl TableSnapshot {
    pub fn new(
        name: impl Into<String>,
        columns: Vec<ColumnSnapshot>,
        primary_key_name: Option<String>,
        primary_key_columns: Vec<String>,
        indexes: Vec<IndexSnapshot>,
        foreign_keys: Vec<ForeignKeySnapshot>,
    ) -> Self {
        Self {
            name: name.into(),
            renamed_from: None,
            columns,
            primary_key_name,
            primary_key_columns,
            indexes,
            foreign_keys,
        }
    }

    pub fn column(&self, name: &str) -> Option<&ColumnSnapshot> {
        self.columns.iter().find(|column| column.name == name)
    }

    pub fn with_renamed_from(mut self, renamed_from: impl Into<String>) -> Self {
        self.renamed_from = Some(renamed_from.into());
        self
    }

    pub fn index(&self, name: &str) -> Option<&IndexSnapshot> {
        self.indexes.iter().find(|index| index.name == name)
    }

    pub fn foreign_key(&self, name: &str) -> Option<&ForeignKeySnapshot> {
        self.foreign_keys
            .iter()
            .find(|foreign_key| foreign_key.name == name)
    }
}

/// Snapshot of a table column, aligned with the code-first metadata already
/// defined in `sql-orm-core`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ColumnSnapshot {
    pub name: String,
    pub renamed_from: Option<String>,
    #[serde(with = "sql_server_type_json")]
    pub sql_type: SqlServerType,
    pub nullable: bool,
    pub primary_key: bool,
    #[serde(with = "identity_json")]
    pub identity: Option<IdentityMetadata>,
    pub default_sql: Option<String>,
    pub computed_sql: Option<String>,
    pub rowversion: bool,
    pub insertable: bool,
    pub updatable: bool,
    pub max_length: Option<u32>,
    pub precision: Option<u8>,
    pub scale: Option<u8>,
}

impl ColumnSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        sql_type: SqlServerType,
        nullable: bool,
        primary_key: bool,
        identity: Option<IdentityMetadata>,
        default_sql: Option<String>,
        computed_sql: Option<String>,
        rowversion: bool,
        insertable: bool,
        updatable: bool,
        max_length: Option<u32>,
        precision: Option<u8>,
        scale: Option<u8>,
    ) -> Self {
        Self {
            name: name.into(),
            renamed_from: None,
            sql_type,
            nullable,
            primary_key,
            identity,
            default_sql,
            computed_sql,
            rowversion,
            insertable,
            updatable,
            max_length,
            precision,
            scale,
        }
    }

    pub fn with_renamed_from(mut self, renamed_from: impl Into<String>) -> Self {
        self.renamed_from = Some(renamed_from.into());
        self
    }
}

impl From<&ColumnMetadata> for ColumnSnapshot {
    fn from(column: &ColumnMetadata) -> Self {
        Self {
            name: column.column_name.to_string(),
            renamed_from: column.renamed_from.map(str::to_owned),
            sql_type: column.sql_type,
            nullable: column.nullable,
            primary_key: column.primary_key,
            identity: column.identity,
            default_sql: column.default_sql.map(str::to_owned),
            computed_sql: column.computed_sql.map(str::to_owned),
            rowversion: column.rowversion,
            insertable: column.insertable,
            updatable: column.updatable,
            max_length: column.max_length,
            precision: column.precision,
            scale: column.scale,
        }
    }
}

/// Snapshot of an index, including the participating columns and sort order.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct IndexSnapshot {
    pub name: String,
    pub columns: Vec<IndexColumnSnapshot>,
    pub unique: bool,
}

impl IndexSnapshot {
    pub fn new(name: impl Into<String>, columns: Vec<IndexColumnSnapshot>, unique: bool) -> Self {
        Self {
            name: name.into(),
            columns,
            unique,
        }
    }
}

impl From<&IndexMetadata> for IndexSnapshot {
    fn from(index: &IndexMetadata) -> Self {
        Self {
            name: index.name.to_string(),
            columns: index
                .columns
                .iter()
                .map(IndexColumnSnapshot::from)
                .collect(),
            unique: index.unique,
        }
    }
}

/// Snapshot of a column inside an index definition.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexColumnSnapshot {
    pub column_name: String,
    pub descending: bool,
}

impl IndexColumnSnapshot {
    pub fn asc(column_name: impl Into<String>) -> Self {
        Self {
            column_name: column_name.into(),
            descending: false,
        }
    }

    pub fn desc(column_name: impl Into<String>) -> Self {
        Self {
            column_name: column_name.into(),
            descending: true,
        }
    }
}

impl From<&IndexColumnMetadata> for IndexColumnSnapshot {
    fn from(column: &IndexColumnMetadata) -> Self {
        Self {
            column_name: column.column_name.to_string(),
            descending: column.descending,
        }
    }
}

/// Snapshot of a foreign key, including referenced target and referential actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForeignKeySnapshot {
    pub name: String,
    pub columns: Vec<String>,
    pub referenced_schema: String,
    pub referenced_table: String,
    pub referenced_columns: Vec<String>,
    #[serde(with = "referential_action_json")]
    pub on_delete: ReferentialAction,
    #[serde(with = "referential_action_json")]
    pub on_update: ReferentialAction,
}

impl ForeignKeySnapshot {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: impl Into<String>,
        columns: Vec<String>,
        referenced_schema: impl Into<String>,
        referenced_table: impl Into<String>,
        referenced_columns: Vec<String>,
        on_delete: ReferentialAction,
        on_update: ReferentialAction,
    ) -> Self {
        Self {
            name: name.into(),
            columns,
            referenced_schema: referenced_schema.into(),
            referenced_table: referenced_table.into(),
            referenced_columns,
            on_delete,
            on_update,
        }
    }
}

impl From<&ForeignKeyMetadata> for ForeignKeySnapshot {
    fn from(foreign_key: &ForeignKeyMetadata) -> Self {
        Self {
            name: foreign_key.name.to_string(),
            columns: foreign_key
                .columns
                .iter()
                .map(|column| (*column).to_string())
                .collect(),
            referenced_schema: foreign_key.referenced_schema.to_string(),
            referenced_table: foreign_key.referenced_table.to_string(),
            referenced_columns: foreign_key
                .referenced_columns
                .iter()
                .map(|column| (*column).to_string())
                .collect(),
            on_delete: foreign_key.on_delete,
            on_update: foreign_key.on_update,
        }
    }
}

impl From<&EntityMetadata> for TableSnapshot {
    fn from(entity: &EntityMetadata) -> Self {
        Self {
            name: entity.table.to_string(),
            renamed_from: entity.renamed_from.map(str::to_owned),
            columns: entity.columns.iter().map(ColumnSnapshot::from).collect(),
            primary_key_name: entity.primary_key.name.map(str::to_owned),
            primary_key_columns: entity
                .primary_key
                .columns
                .iter()
                .map(|column| (*column).to_string())
                .collect(),
            indexes: entity.indexes.iter().map(IndexSnapshot::from).collect(),
            foreign_keys: entity
                .foreign_keys
                .iter()
                .map(ForeignKeySnapshot::from)
                .collect(),
        }
    }
}

mod identity_json {
    use super::*;

    #[derive(Serialize, Deserialize)]
    struct IdentitySnapshot {
        seed: i64,
        increment: i64,
    }

    pub fn serialize<S>(
        identity: &Option<IdentityMetadata>,
        serializer: S,
    ) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        identity
            .map(|identity| IdentitySnapshot {
                seed: identity.seed,
                increment: identity.increment,
            })
            .serialize(serializer)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Option<IdentityMetadata>, D::Error>
    where
        D: Deserializer<'de>,
    {
        Option::<IdentitySnapshot>::deserialize(deserializer).map(|identity| {
            identity.map(|identity| IdentityMetadata::new(identity.seed, identity.increment))
        })
    }
}

mod sql_server_type_json {
    use super::*;

    pub fn serialize<S>(sql_type: &SqlServerType, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match sql_type {
            SqlServerType::Custom(value) => serializer.serialize_str(&format!("custom:{value}")),
            other => serializer.serialize_str(to_str(other)),
        }
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<SqlServerType, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        from_str(&value).ok_or_else(|| {
            de::Error::custom(format!("unsupported SQL Server type in snapshot: {value}"))
        })
    }

    fn to_str(sql_type: &SqlServerType) -> &str {
        match sql_type {
            SqlServerType::BigInt => "bigint",
            SqlServerType::Int => "int",
            SqlServerType::SmallInt => "smallint",
            SqlServerType::TinyInt => "tinyint",
            SqlServerType::Bit => "bit",
            SqlServerType::UniqueIdentifier => "uniqueidentifier",
            SqlServerType::Date => "date",
            SqlServerType::DateTime2 => "datetime2",
            SqlServerType::Decimal => "decimal",
            SqlServerType::Float => "float",
            SqlServerType::Money => "money",
            SqlServerType::NVarChar => "nvarchar",
            SqlServerType::VarBinary => "varbinary",
            SqlServerType::RowVersion => "rowversion",
            SqlServerType::Custom(value) => value,
        }
    }

    fn from_str(value: &str) -> Option<SqlServerType> {
        if let Some(custom) = value.strip_prefix("custom:") {
            return if custom.is_empty() {
                None
            } else {
                Some(SqlServerType::Custom(leak_static_str(custom)))
            };
        }

        match value {
            "bigint" => Some(SqlServerType::BigInt),
            "int" => Some(SqlServerType::Int),
            "smallint" => Some(SqlServerType::SmallInt),
            "tinyint" => Some(SqlServerType::TinyInt),
            "bit" => Some(SqlServerType::Bit),
            "uniqueidentifier" => Some(SqlServerType::UniqueIdentifier),
            "date" => Some(SqlServerType::Date),
            "datetime2" => Some(SqlServerType::DateTime2),
            "decimal" => Some(SqlServerType::Decimal),
            "float" => Some(SqlServerType::Float),
            "money" => Some(SqlServerType::Money),
            "nvarchar" => Some(SqlServerType::NVarChar),
            "varbinary" => Some(SqlServerType::VarBinary),
            "rowversion" => Some(SqlServerType::RowVersion),
            _ => None,
        }
    }

    fn leak_static_str(value: &str) -> &'static str {
        Box::leak(value.to_owned().into_boxed_str())
    }
}

mod referential_action_json {
    use super::*;

    pub fn serialize<S>(action: &ReferentialAction, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(match action {
            ReferentialAction::NoAction => "no_action",
            ReferentialAction::Cascade => "cascade",
            ReferentialAction::SetNull => "set_null",
            ReferentialAction::SetDefault => "set_default",
        })
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<ReferentialAction, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        match value.as_str() {
            "no_action" => Ok(ReferentialAction::NoAction),
            "cascade" => Ok(ReferentialAction::Cascade),
            "set_null" => Ok(ReferentialAction::SetNull),
            "set_default" => Ok(ReferentialAction::SetDefault),
            _ => Err(de::Error::custom(format!(
                "unsupported referential action in snapshot: {value}"
            ))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ColumnSnapshot, ForeignKeySnapshot, IndexColumnSnapshot, IndexSnapshot, ModelSnapshot,
        SchemaSnapshot, TableSnapshot,
    };
    use sql_orm_core::{IdentityMetadata, OrmErrorKind, ReferentialAction, SqlServerType};

    #[test]
    fn serializes_empty_model_snapshot_as_stable_json() {
        let json = ModelSnapshot::default().to_json_pretty().unwrap();

        assert_eq!(json, "{\n  \"schemas\": []\n}\n");
        assert_eq!(
            ModelSnapshot::from_json(&json).unwrap(),
            ModelSnapshot::default()
        );
    }

    #[test]
    fn classifies_invalid_model_snapshot_json_as_migration_error() {
        let error = ModelSnapshot::from_json("{").unwrap_err();

        assert_eq!(error.kind(), OrmErrorKind::Migration);
        assert_eq!(error.message(), "failed to deserialize model snapshot");
    }

    #[test]
    fn roundtrips_complete_model_snapshot_json() {
        let snapshot = ModelSnapshot::new(vec![SchemaSnapshot::new(
            "sales",
            vec![TableSnapshot {
                name: "orders".to_string(),
                renamed_from: Some("legacy_orders".to_string()),
                columns: vec![
                    ColumnSnapshot::new(
                        "id",
                        SqlServerType::BigInt,
                        false,
                        true,
                        Some(IdentityMetadata::new(1, 1)),
                        None,
                        None,
                        false,
                        false,
                        false,
                        None,
                        None,
                        None,
                    ),
                    ColumnSnapshot::new(
                        "status",
                        SqlServerType::Custom("varchar(24)"),
                        false,
                        false,
                        None,
                        Some("'open'".to_string()),
                        None,
                        false,
                        true,
                        true,
                        Some(24),
                        None,
                        None,
                    )
                    .with_renamed_from("state"),
                ],
                primary_key_name: Some("pk_orders".to_string()),
                primary_key_columns: vec!["id".to_string()],
                indexes: vec![IndexSnapshot::new(
                    "ix_orders_status",
                    vec![IndexColumnSnapshot::desc("status")],
                    false,
                )],
                foreign_keys: vec![ForeignKeySnapshot::new(
                    "fk_orders_customers",
                    vec!["customer_id".to_string()],
                    "sales",
                    "customers",
                    vec!["id".to_string()],
                    ReferentialAction::Cascade,
                    ReferentialAction::NoAction,
                )],
            }],
        )]);

        let json = snapshot.to_json_pretty().unwrap();
        let parsed = ModelSnapshot::from_json(&json).unwrap();

        assert_eq!(parsed, snapshot);
        assert!(json.contains("\"sql_type\": \"custom:varchar(24)\""));
        assert!(json.contains("\"on_delete\": \"cascade\""));
    }
}
