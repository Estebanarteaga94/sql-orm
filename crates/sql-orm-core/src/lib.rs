//! Core contracts and shared types for the ORM.

use chrono::{NaiveDate, NaiveDateTime};
use core::fmt;
use core::marker::PhantomData;
use rust_decimal::Decimal;
use uuid::Uuid;

/// Common error type placeholder for the workspace foundations.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OrmError {
    Message(String),
    ConcurrencyConflict,
}

impl OrmError {
    pub fn new(message: impl Into<String>) -> Self {
        Self::Message(message.into())
    }

    pub const fn concurrency_conflict() -> Self {
        Self::ConcurrencyConflict
    }

    pub fn message(&self) -> &str {
        match self {
            Self::Message(message) => message,
            Self::ConcurrencyConflict => "concurrency conflict",
        }
    }
}

impl fmt::Display for OrmError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.message())
    }
}

impl std::error::Error for OrmError {}

/// Quotes a SQL Server Unicode string literal as `N'...'`.
///
/// This helper is for trusted SQL generation paths that must interpolate text
/// into metadata or migration scripts. User values in queries should continue
/// to use parameters instead of string interpolation.
pub fn quote_sql_string_literal(value: &str) -> String {
    format!("N'{}'", value.replace('\'', "''"))
}

/// Minimal crate identity metadata used while the rest of the model is defined.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CrateIdentity {
    pub name: &'static str,
    pub responsibility: &'static str,
}

pub const CRATE_IDENTITY: CrateIdentity = CrateIdentity {
    name: "sql-orm-core",
    responsibility: "contracts, metadata, shared types and errors",
};

/// Stable contract implemented by persisted entities.
pub trait Entity: Sized + Send + Sync + 'static {
    fn metadata() -> &'static EntityMetadata;
}

/// Static metadata exposed by a reusable entity policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityPolicyMetadata {
    pub name: &'static str,
    pub columns: &'static [ColumnMetadata],
}

impl EntityPolicyMetadata {
    pub const fn new(name: &'static str, columns: &'static [ColumnMetadata]) -> Self {
        Self { name, columns }
    }
}

/// Stable contract for reusable code-first policies that contribute normal columns.
pub trait EntityPolicy: Sized + Send + Sync + 'static {
    const POLICY_NAME: &'static str;
    const COLUMN_NAMES: &'static [&'static str] = &[];

    fn columns() -> &'static [ColumnMetadata];

    fn metadata() -> EntityPolicyMetadata {
        EntityPolicyMetadata::new(Self::POLICY_NAME, Self::columns())
    }
}

pub const fn column_name_exists(columns: &[&'static str], column_name: &'static str) -> bool {
    let mut index = 0;
    while index < columns.len() {
        if column_name_eq(columns[index], column_name) {
            return true;
        }
        index += 1;
    }
    false
}

const fn column_name_eq(left: &str, right: &str) -> bool {
    let left = left.as_bytes();
    let right = right.as_bytes();
    if left.len() != right.len() {
        return false;
    }

    let mut index = 0;
    while index < left.len() {
        if left[index] != right[index] {
            return false;
        }
        index += 1;
    }
    true
}

/// Base Rust <-> SQL Server mapping contract used by row readers and persistence models.
pub trait SqlTypeMapping: Sized {
    const SQL_SERVER_TYPE: SqlServerType;
    const DEFAULT_MAX_LENGTH: Option<u32> = None;
    const DEFAULT_PRECISION: Option<u8> = None;
    const DEFAULT_SCALE: Option<u8> = None;

    fn to_sql_value(self) -> SqlValue;

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError>;
}

/// Neutral SQL value representation shared across query compilation and execution layers.
#[derive(Debug, Clone, PartialEq)]
pub enum SqlValue {
    Null,
    TypedNull(SqlServerType),
    Bool(bool),
    I32(i32),
    I64(i64),
    F64(f64),
    String(String),
    Bytes(Vec<u8>),
    Uuid(Uuid),
    Decimal(Decimal),
    Date(NaiveDate),
    DateTime(NaiveDateTime),
}

impl SqlValue {
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null | Self::TypedNull(_))
    }
}

/// Column/value pair produced by insert and update models.
#[derive(Debug, Clone, PartialEq)]
pub struct ColumnValue {
    pub column_name: &'static str,
    pub value: SqlValue,
}

impl ColumnValue {
    pub const fn new(column_name: &'static str, value: SqlValue) -> Self {
        Self { column_name, value }
    }
}

/// Row abstraction used by the core mapping contracts without depending on Tiberius.
pub trait Row {
    fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError>;

    fn get_required(&self, column: &str) -> Result<SqlValue, OrmError> {
        self.try_get(column)?
            .ok_or_else(|| OrmError::new("required column value was not present"))
    }

    fn try_get_typed<T: SqlTypeMapping>(&self, column: &str) -> Result<Option<T>, OrmError> {
        self.try_get(column)?.map(T::from_sql_value).transpose()
    }

    fn get_required_typed<T: SqlTypeMapping>(&self, column: &str) -> Result<T, OrmError> {
        T::from_sql_value(self.get_required(column)?)
    }
}

/// Stable contract for mapping a SQL row into a Rust type.
pub trait FromRow: Sized {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError>;
}

/// Stable contract for extracting persisted values for inserts.
pub trait Insertable<E: Entity> {
    fn values(&self) -> Vec<ColumnValue>;
}

/// Stable contract for extracting changed values for updates.
pub trait Changeset<E: Entity> {
    fn changes(&self) -> Vec<ColumnValue>;

    fn concurrency_token(&self) -> Result<Option<SqlValue>, OrmError> {
        Ok(None)
    }
}

/// Static column symbol generated for entities and consumed later by the query builder.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityColumn<E: Entity> {
    rust_field: &'static str,
    column_name: &'static str,
    _entity: PhantomData<fn() -> E>,
}

impl<E: Entity> EntityColumn<E> {
    pub const fn new(rust_field: &'static str, column_name: &'static str) -> Self {
        Self {
            rust_field,
            column_name,
            _entity: PhantomData,
        }
    }

    pub const fn rust_field(&self) -> &'static str {
        self.rust_field
    }

    pub const fn column_name(&self) -> &'static str {
        self.column_name
    }

    pub fn entity_metadata(&self) -> &'static EntityMetadata {
        E::metadata()
    }

    pub fn metadata(&self) -> &'static ColumnMetadata {
        E::metadata()
            .field(self.rust_field)
            .expect("generated entity column must reference existing metadata")
    }
}

/// SQL Server types supported by the metadata layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SqlServerType {
    BigInt,
    Int,
    SmallInt,
    TinyInt,
    Bit,
    UniqueIdentifier,
    Date,
    DateTime2,
    Decimal,
    Float,
    Money,
    NVarChar,
    VarBinary,
    RowVersion,
    Custom(&'static str),
}

impl SqlTypeMapping for bool {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::Bit;

    fn to_sql_value(self) -> SqlValue {
        SqlValue::Bool(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::Bool(value) => Ok(value),
            _ => Err(OrmError::new("expected bool value")),
        }
    }
}

impl SqlTypeMapping for i32 {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::Int;

    fn to_sql_value(self) -> SqlValue {
        SqlValue::I32(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::I32(value) => Ok(value),
            _ => Err(OrmError::new("expected i32 value")),
        }
    }
}

impl SqlTypeMapping for i64 {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::BigInt;

    fn to_sql_value(self) -> SqlValue {
        SqlValue::I64(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::I64(value) => Ok(value),
            _ => Err(OrmError::new("expected i64 value")),
        }
    }
}

impl SqlTypeMapping for f64 {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::Float;

    fn to_sql_value(self) -> SqlValue {
        SqlValue::F64(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::F64(value) => Ok(value),
            _ => Err(OrmError::new("expected f64 value")),
        }
    }
}

impl SqlTypeMapping for String {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::NVarChar;
    const DEFAULT_MAX_LENGTH: Option<u32> = Some(255);

    fn to_sql_value(self) -> SqlValue {
        SqlValue::String(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::String(value) => Ok(value),
            _ => Err(OrmError::new("expected string value")),
        }
    }
}

impl SqlTypeMapping for Vec<u8> {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::VarBinary;

    fn to_sql_value(self) -> SqlValue {
        SqlValue::Bytes(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::Bytes(value) => Ok(value),
            _ => Err(OrmError::new("expected bytes value")),
        }
    }
}

impl SqlTypeMapping for Uuid {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::UniqueIdentifier;

    fn to_sql_value(self) -> SqlValue {
        SqlValue::Uuid(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::Uuid(value) => Ok(value),
            _ => Err(OrmError::new("expected uuid value")),
        }
    }
}

impl SqlTypeMapping for Decimal {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::Decimal;
    const DEFAULT_PRECISION: Option<u8> = Some(18);
    const DEFAULT_SCALE: Option<u8> = Some(2);

    fn to_sql_value(self) -> SqlValue {
        SqlValue::Decimal(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::Decimal(value) => Ok(value),
            _ => Err(OrmError::new("expected decimal value")),
        }
    }
}

impl SqlTypeMapping for NaiveDate {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::Date;

    fn to_sql_value(self) -> SqlValue {
        SqlValue::Date(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::Date(value) => Ok(value),
            _ => Err(OrmError::new("expected date value")),
        }
    }
}

impl SqlTypeMapping for NaiveDateTime {
    const SQL_SERVER_TYPE: SqlServerType = SqlServerType::DateTime2;

    fn to_sql_value(self) -> SqlValue {
        SqlValue::DateTime(self)
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::DateTime(value) => Ok(value),
            _ => Err(OrmError::new("expected datetime value")),
        }
    }
}

impl<T> SqlTypeMapping for Option<T>
where
    T: SqlTypeMapping,
{
    const SQL_SERVER_TYPE: SqlServerType = T::SQL_SERVER_TYPE;
    const DEFAULT_MAX_LENGTH: Option<u32> = T::DEFAULT_MAX_LENGTH;
    const DEFAULT_PRECISION: Option<u8> = T::DEFAULT_PRECISION;
    const DEFAULT_SCALE: Option<u8> = T::DEFAULT_SCALE;

    fn to_sql_value(self) -> SqlValue {
        self.map(T::to_sql_value)
            .unwrap_or(SqlValue::TypedNull(T::SQL_SERVER_TYPE))
    }

    fn from_sql_value(value: SqlValue) -> Result<Self, OrmError> {
        match value {
            SqlValue::Null | SqlValue::TypedNull(_) => Ok(None),
            other => T::from_sql_value(other).map(Some),
        }
    }
}

/// Metadata for SQL Server identity columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IdentityMetadata {
    pub seed: i64,
    pub increment: i64,
}

impl IdentityMetadata {
    pub const fn new(seed: i64, increment: i64) -> Self {
        Self { seed, increment }
    }
}

/// Primary key metadata for an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PrimaryKeyMetadata {
    pub name: Option<&'static str>,
    pub columns: &'static [&'static str],
}

impl PrimaryKeyMetadata {
    pub const fn new(name: Option<&'static str>, columns: &'static [&'static str]) -> Self {
        Self { name, columns }
    }
}

/// Per-column metadata generated from entity definitions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ColumnMetadata {
    pub rust_field: &'static str,
    pub column_name: &'static str,
    pub renamed_from: Option<&'static str>,
    pub sql_type: SqlServerType,
    pub nullable: bool,
    pub primary_key: bool,
    pub identity: Option<IdentityMetadata>,
    pub default_sql: Option<&'static str>,
    pub computed_sql: Option<&'static str>,
    pub rowversion: bool,
    pub insertable: bool,
    pub updatable: bool,
    pub max_length: Option<u32>,
    pub precision: Option<u8>,
    pub scale: Option<u8>,
}

impl ColumnMetadata {
    pub const fn is_computed(&self) -> bool {
        self.computed_sql.is_some()
    }
}

/// Columns participating in an index and their sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexColumnMetadata {
    pub column_name: &'static str,
    pub descending: bool,
}

impl IndexColumnMetadata {
    pub const fn asc(column_name: &'static str) -> Self {
        Self {
            column_name,
            descending: false,
        }
    }

    pub const fn desc(column_name: &'static str) -> Self {
        Self {
            column_name,
            descending: true,
        }
    }
}

/// Index metadata attached to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexMetadata {
    pub name: &'static str,
    pub columns: &'static [IndexColumnMetadata],
    pub unique: bool,
}

/// Delete/update behavior for foreign keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReferentialAction {
    NoAction,
    Cascade,
    SetNull,
    SetDefault,
}

/// Foreign key metadata attached to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ForeignKeyMetadata {
    pub name: &'static str,
    pub columns: &'static [&'static str],
    pub referenced_schema: &'static str,
    pub referenced_table: &'static str,
    pub referenced_columns: &'static [&'static str],
    pub on_delete: ReferentialAction,
    pub on_update: ReferentialAction,
}

impl ForeignKeyMetadata {
    pub const fn new(
        name: &'static str,
        columns: &'static [&'static str],
        referenced_schema: &'static str,
        referenced_table: &'static str,
        referenced_columns: &'static [&'static str],
        on_delete: ReferentialAction,
        on_update: ReferentialAction,
    ) -> Self {
        Self {
            name,
            columns,
            referenced_schema,
            referenced_table,
            referenced_columns,
            on_delete,
            on_update,
        }
    }

    pub fn references_table(&self, schema: &str, table: &str) -> bool {
        self.referenced_schema == schema && self.referenced_table == table
    }

    pub fn includes_column(&self, column_name: &str) -> bool {
        self.columns.contains(&column_name)
    }
}

/// Relationship direction represented by a navigation property.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationKind {
    BelongsTo,
    HasOne,
    HasMany,
    ManyToMany,
}

/// Navigation property metadata attached to an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationMetadata {
    pub rust_field: &'static str,
    pub kind: NavigationKind,
    pub target_rust_name: &'static str,
    pub target_schema: &'static str,
    pub target_table: &'static str,
    pub local_columns: &'static [&'static str],
    pub target_columns: &'static [&'static str],
    pub foreign_key_name: Option<&'static str>,
}

impl NavigationMetadata {
    #[allow(clippy::too_many_arguments)]
    pub const fn new(
        rust_field: &'static str,
        kind: NavigationKind,
        target_rust_name: &'static str,
        target_schema: &'static str,
        target_table: &'static str,
        local_columns: &'static [&'static str],
        target_columns: &'static [&'static str],
        foreign_key_name: Option<&'static str>,
    ) -> Self {
        Self {
            rust_field,
            kind,
            target_rust_name,
            target_schema,
            target_table,
            local_columns,
            target_columns,
            foreign_key_name,
        }
    }

    pub fn targets_table(&self, schema: &str, table: &str) -> bool {
        self.target_schema == schema && self.target_table == table
    }

    pub fn uses_foreign_key(&self, foreign_key_name: &str) -> bool {
        self.foreign_key_name == Some(foreign_key_name)
    }
}

/// Static metadata describing an entity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntityMetadata {
    pub rust_name: &'static str,
    pub schema: &'static str,
    pub table: &'static str,
    pub renamed_from: Option<&'static str>,
    pub columns: &'static [ColumnMetadata],
    pub primary_key: PrimaryKeyMetadata,
    pub indexes: &'static [IndexMetadata],
    pub foreign_keys: &'static [ForeignKeyMetadata],
    pub navigations: &'static [NavigationMetadata],
}

impl EntityMetadata {
    pub fn column(&self, column_name: &str) -> Option<&'static ColumnMetadata> {
        self.columns
            .iter()
            .find(|column| column.column_name == column_name)
    }

    pub fn field(&self, rust_field: &str) -> Option<&'static ColumnMetadata> {
        self.columns
            .iter()
            .find(|column| column.rust_field == rust_field)
    }

    pub fn primary_key_columns(&self) -> Vec<&'static ColumnMetadata> {
        self.primary_key
            .columns
            .iter()
            .filter_map(|column_name| self.column(column_name))
            .collect()
    }

    pub fn rowversion_column(&self) -> Option<&'static ColumnMetadata> {
        self.columns.iter().find(|column| column.rowversion)
    }

    pub fn foreign_key(&self, name: &str) -> Option<&'static ForeignKeyMetadata> {
        self.foreign_keys
            .iter()
            .find(|foreign_key| foreign_key.name == name)
    }

    pub fn foreign_keys_for_column(&self, column_name: &str) -> Vec<&'static ForeignKeyMetadata> {
        self.foreign_keys
            .iter()
            .filter(|foreign_key| foreign_key.includes_column(column_name))
            .collect()
    }

    pub fn foreign_keys_referencing(
        &self,
        schema: &str,
        table: &str,
    ) -> Vec<&'static ForeignKeyMetadata> {
        self.foreign_keys
            .iter()
            .filter(|foreign_key| foreign_key.references_table(schema, table))
            .collect()
    }

    pub fn navigation(&self, rust_field: &str) -> Option<&'static NavigationMetadata> {
        self.navigations
            .iter()
            .find(|navigation| navigation.rust_field == rust_field)
    }

    pub fn navigations_by_kind(&self, kind: NavigationKind) -> Vec<&'static NavigationMetadata> {
        self.navigations
            .iter()
            .filter(|navigation| navigation.kind == kind)
            .collect()
    }

    pub fn navigations_for_foreign_key(
        &self,
        foreign_key_name: &str,
    ) -> Vec<&'static NavigationMetadata> {
        self.navigations
            .iter()
            .filter(|navigation| navigation.uses_foreign_key(foreign_key_name))
            .collect()
    }

    pub fn navigations_targeting(
        &self,
        schema: &str,
        table: &str,
    ) -> Vec<&'static NavigationMetadata> {
        self.navigations
            .iter()
            .filter(|navigation| navigation.targets_table(schema, table))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::{
        CRATE_IDENTITY, Changeset, ColumnMetadata, ColumnValue, Entity, EntityColumn,
        EntityMetadata, EntityPolicy, EntityPolicyMetadata, ForeignKeyMetadata, FromRow,
        IdentityMetadata, IndexColumnMetadata, IndexMetadata, Insertable, NavigationKind,
        NavigationMetadata, OrmError, PrimaryKeyMetadata, ReferentialAction, Row, SqlServerType,
        SqlTypeMapping, SqlValue, column_name_exists, quote_sql_string_literal,
    };
    use chrono::{NaiveDate, NaiveDateTime};
    use rust_decimal::Decimal;
    use std::collections::BTreeMap;
    use uuid::Uuid;

    const USER_COLUMNS: [ColumnMetadata; 4] = [
        ColumnMetadata {
            rust_field: "tenant_id",
            column_name: "tenant_id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "id",
            column_name: "id",
            renamed_from: None,
            sql_type: SqlServerType::BigInt,
            nullable: false,
            primary_key: true,
            identity: Some(IdentityMetadata::new(1, 1)),
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
            rust_field: "email",
            column_name: "email",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: true,
            max_length: Some(180),
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "version",
            column_name: "version",
            renamed_from: None,
            sql_type: SqlServerType::RowVersion,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: None,
            rowversion: true,
            insertable: false,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
    ];

    const USER_PRIMARY_KEY_COLUMNS: [&str; 2] = ["id", "tenant_id"];

    const USER_INDEXES: [IndexMetadata; 1] = [IndexMetadata {
        name: "ux_users_email",
        columns: &[IndexColumnMetadata::asc("email")],
        unique: true,
    }];

    const USER_FOREIGN_KEYS: [ForeignKeyMetadata; 1] = [ForeignKeyMetadata::new(
        "fk_users_tenants",
        &["tenant_id"],
        "dbo",
        "tenants",
        &["id"],
        ReferentialAction::NoAction,
        ReferentialAction::NoAction,
    )];

    const USER_NAVIGATIONS: [NavigationMetadata; 1] = [NavigationMetadata::new(
        "tenant",
        NavigationKind::BelongsTo,
        "Tenant",
        "dbo",
        "tenants",
        &["tenant_id"],
        &["id"],
        Some("fk_users_tenants"),
    )];

    const AUDIT_POLICY_COLUMNS: [ColumnMetadata; 2] = [
        ColumnMetadata {
            rust_field: "created_at",
            column_name: "created_at",
            renamed_from: None,
            sql_type: SqlServerType::DateTime2,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: Some("SYSUTCDATETIME()"),
            computed_sql: None,
            rowversion: false,
            insertable: true,
            updatable: false,
            max_length: None,
            precision: None,
            scale: None,
        },
        ColumnMetadata {
            rust_field: "updated_at",
            column_name: "updated_at",
            renamed_from: None,
            sql_type: SqlServerType::DateTime2,
            nullable: true,
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

    const USER_METADATA: EntityMetadata = EntityMetadata {
        rust_name: "User",
        schema: "dbo",
        table: "users",
        renamed_from: None,
        columns: &USER_COLUMNS,
        primary_key: PrimaryKeyMetadata::new(Some("pk_users"), &USER_PRIMARY_KEY_COLUMNS),
        indexes: &USER_INDEXES,
        foreign_keys: &USER_FOREIGN_KEYS,
        navigations: &USER_NAVIGATIONS,
    };

    struct User;

    impl Entity for User {
        fn metadata() -> &'static EntityMetadata {
            &USER_METADATA
        }
    }

    struct AuditPolicy;

    impl EntityPolicy for AuditPolicy {
        const POLICY_NAME: &'static str = "audit";
        const COLUMN_NAMES: &'static [&'static str] = &["created_at", "updated_at"];

        fn columns() -> &'static [ColumnMetadata] {
            &AUDIT_POLICY_COLUMNS
        }
    }

    struct TestRow {
        values: BTreeMap<&'static str, SqlValue>,
    }

    impl Row for TestRow {
        fn try_get(&self, column: &str) -> Result<Option<SqlValue>, OrmError> {
            Ok(self.values.get(column).cloned())
        }
    }

    #[derive(Debug, PartialEq)]
    struct UserRecord {
        id: i64,
        email: String,
    }

    impl FromRow for UserRecord {
        fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
            let id = row.get_required_typed::<i64>("id")?;
            let email = row.get_required_typed::<String>("email")?;

            Ok(Self { id, email })
        }
    }

    struct NewUser {
        email: String,
    }

    impl Insertable<User> for NewUser {
        fn values(&self) -> Vec<ColumnValue> {
            vec![ColumnValue::new(
                "email",
                SqlValue::String(self.email.clone()),
            )]
        }
    }

    struct UpdateUser {
        email: Option<String>,
    }

    impl Changeset<User> for UpdateUser {
        fn changes(&self) -> Vec<ColumnValue> {
            self.email
                .clone()
                .map(|email| vec![ColumnValue::new("email", SqlValue::String(email))])
                .unwrap_or_default()
        }
    }

    #[test]
    fn exposes_foundation_identity() {
        assert_eq!(CRATE_IDENTITY.name, "sql-orm-core");
    }

    #[test]
    fn preserves_error_message() {
        let error = OrmError::new("foundation");
        assert_eq!(error.message(), "foundation");
        assert_eq!(error.to_string(), "foundation");
    }

    #[test]
    fn exposes_concurrency_conflict_error() {
        let error = OrmError::concurrency_conflict();
        assert_eq!(error, OrmError::ConcurrencyConflict);
        assert_eq!(error.message(), "concurrency conflict");
        assert_eq!(error.to_string(), "concurrency conflict");
    }

    #[test]
    fn entity_trait_exposes_static_metadata() {
        let metadata = User::metadata();

        assert_eq!(metadata.rust_name, "User");
        assert_eq!(metadata.schema, "dbo");
        assert_eq!(metadata.table, "users");
        assert_eq!(metadata.primary_key.name, Some("pk_users"));
        assert_eq!(metadata.indexes.len(), 1);
        assert_eq!(metadata.foreign_keys.len(), 1);
        assert_eq!(metadata.navigations.len(), 1);
        assert_eq!(metadata.primary_key.columns, &["id", "tenant_id"]);
    }

    #[test]
    fn entity_policy_exposes_reusable_column_metadata() {
        let metadata = AuditPolicy::metadata();

        assert_eq!(
            metadata,
            EntityPolicyMetadata::new("audit", &AUDIT_POLICY_COLUMNS)
        );
        assert_eq!(metadata.columns[0].column_name, "created_at");
        assert_eq!(metadata.columns[0].default_sql, Some("SYSUTCDATETIME()"));
        assert!(!metadata.columns[0].primary_key);
        assert!(metadata.columns[1].nullable);
        assert!(metadata.columns[1].updatable);
        assert!(column_name_exists(AuditPolicy::COLUMN_NAMES, "created_at"));
        assert!(!column_name_exists(AuditPolicy::COLUMN_NAMES, "missing"));
    }

    #[test]
    fn metadata_can_lookup_columns_by_field_and_name() {
        let metadata = User::metadata();

        assert_eq!(metadata.column("email"), metadata.field("email"));
        assert_eq!(
            metadata.column("version").map(|column| column.sql_type),
            Some(SqlServerType::RowVersion)
        );
        assert!(metadata.column("missing").is_none());
    }

    #[test]
    fn foreign_key_metadata_supports_relationship_lookups() {
        let metadata = User::metadata();
        let foreign_key = metadata
            .foreign_key("fk_users_tenants")
            .expect("foreign key metadata");

        assert_eq!(foreign_key.columns, &["tenant_id"]);
        assert_eq!(foreign_key.referenced_schema, "dbo");
        assert_eq!(foreign_key.referenced_table, "tenants");
        assert_eq!(foreign_key.referenced_columns, &["id"]);
        assert!(foreign_key.references_table("dbo", "tenants"));
        assert!(!foreign_key.references_table("sales", "tenants"));
        assert!(foreign_key.includes_column("tenant_id"));
        assert!(!foreign_key.includes_column("email"));
    }

    #[test]
    fn metadata_can_filter_foreign_keys_by_column_and_target_table() {
        let metadata = User::metadata();

        let by_column = metadata.foreign_keys_for_column("tenant_id");
        assert_eq!(by_column.len(), 1);
        assert_eq!(by_column[0].name, "fk_users_tenants");

        let by_table = metadata.foreign_keys_referencing("dbo", "tenants");
        assert_eq!(by_table.len(), 1);
        assert_eq!(by_table[0].name, "fk_users_tenants");

        assert!(metadata.foreign_keys_for_column("email").is_empty());
        assert!(
            metadata
                .foreign_keys_referencing("sales", "customers")
                .is_empty()
        );
    }

    #[test]
    fn navigation_metadata_supports_relationship_lookups() {
        let metadata = User::metadata();
        let navigation = metadata.navigation("tenant").expect("navigation metadata");

        assert_eq!(navigation.kind, NavigationKind::BelongsTo);
        assert_eq!(navigation.target_rust_name, "Tenant");
        assert_eq!(navigation.target_schema, "dbo");
        assert_eq!(navigation.target_table, "tenants");
        assert_eq!(navigation.local_columns, &["tenant_id"]);
        assert_eq!(navigation.target_columns, &["id"]);
        assert_eq!(navigation.foreign_key_name, Some("fk_users_tenants"));
        assert!(navigation.targets_table("dbo", "tenants"));
        assert!(!navigation.targets_table("sales", "tenants"));
        assert!(navigation.uses_foreign_key("fk_users_tenants"));
        assert!(!navigation.uses_foreign_key("fk_users_accounts"));

        let by_kind = metadata.navigations_by_kind(NavigationKind::BelongsTo);
        assert_eq!(by_kind.len(), 1);
        assert_eq!(by_kind[0].rust_field, "tenant");

        let by_foreign_key = metadata.navigations_for_foreign_key("fk_users_tenants");
        assert_eq!(by_foreign_key.len(), 1);
        assert_eq!(by_foreign_key[0].rust_field, "tenant");

        let by_target = metadata.navigations_targeting("dbo", "tenants");
        assert_eq!(by_target.len(), 1);
        assert_eq!(by_target[0].rust_field, "tenant");

        assert!(metadata.navigation("missing").is_none());
        assert!(
            metadata
                .navigations_by_kind(NavigationKind::HasMany)
                .is_empty()
        );
        assert!(
            metadata
                .navigations_for_foreign_key("fk_users_missing")
                .is_empty()
        );
        assert!(
            metadata
                .navigations_targeting("sales", "customers")
                .is_empty()
        );
    }

    #[test]
    fn metadata_returns_primary_key_columns() {
        let metadata = User::metadata();
        let columns = metadata.primary_key_columns();

        assert_eq!(columns.len(), 2);
        assert_eq!(columns[0].column_name, "id");
        assert_eq!(columns[1].column_name, "tenant_id");
        assert!(columns.iter().all(|column| column.primary_key));
    }

    #[test]
    fn metadata_returns_rowversion_column_when_present() {
        let metadata = User::metadata();
        let column = metadata.rowversion_column().expect("rowversion column");

        assert_eq!(column.column_name, "version");
        assert!(column.rowversion);
    }

    #[test]
    fn column_metadata_marks_computed_values() {
        let computed = ColumnMetadata {
            rust_field: "full_name",
            column_name: "full_name",
            renamed_from: None,
            sql_type: SqlServerType::NVarChar,
            nullable: false,
            primary_key: false,
            identity: None,
            default_sql: None,
            computed_sql: Some("[first_name] + ' ' + [last_name]"),
            rowversion: false,
            insertable: false,
            updatable: false,
            max_length: Some(240),
            precision: None,
            scale: None,
        };

        assert!(computed.is_computed());
        assert!(!USER_COLUMNS[0].is_computed());
    }

    #[test]
    fn index_columns_preserve_sort_direction() {
        let descending = IndexColumnMetadata::desc("created_at");

        assert_eq!(
            USER_INDEXES[0].columns[0],
            IndexColumnMetadata::asc("email")
        );
        assert!(descending.descending);
        assert_eq!(descending.column_name, "created_at");
    }

    #[test]
    fn entity_column_resolves_back_to_column_metadata() {
        let column = EntityColumn::<User>::new("email", "email");

        assert_eq!(column.rust_field(), "email");
        assert_eq!(column.column_name(), "email");
        assert_eq!(column.entity_metadata().table, "users");
        assert_eq!(column.metadata(), &USER_COLUMNS[2]);
    }

    #[test]
    fn sql_value_and_row_contract_support_basic_mapping() {
        let row = TestRow {
            values: BTreeMap::from([
                ("id", SqlValue::I64(7)),
                ("email", SqlValue::String("ana@example.com".to_string())),
            ]),
        };

        let record = UserRecord::from_row(&row).expect("row mapping should succeed");

        assert_eq!(
            record,
            UserRecord {
                id: 7,
                email: "ana@example.com".to_string(),
            }
        );
    }

    #[test]
    fn insertable_and_changeset_return_column_values() {
        let insert = NewUser {
            email: "ana@example.com".to_string(),
        };
        let changes = UpdateUser {
            email: Some("ana.maria@example.com".to_string()),
        };

        assert_eq!(
            insert.values(),
            vec![ColumnValue::new(
                "email",
                SqlValue::String("ana@example.com".to_string())
            )]
        );
        assert_eq!(
            changes.changes(),
            vec![ColumnValue::new(
                "email",
                SqlValue::String("ana.maria@example.com".to_string())
            )]
        );
        assert!(UpdateUser { email: None }.changes().is_empty());
    }

    #[test]
    fn sql_type_mapping_exposes_default_sqlserver_conventions() {
        assert_eq!(String::SQL_SERVER_TYPE, SqlServerType::NVarChar);
        assert_eq!(String::DEFAULT_MAX_LENGTH, Some(255));
        assert_eq!(bool::SQL_SERVER_TYPE, SqlServerType::Bit);
        assert_eq!(i32::SQL_SERVER_TYPE, SqlServerType::Int);
        assert_eq!(i64::SQL_SERVER_TYPE, SqlServerType::BigInt);
        assert_eq!(Uuid::SQL_SERVER_TYPE, SqlServerType::UniqueIdentifier);
        assert_eq!(NaiveDateTime::SQL_SERVER_TYPE, SqlServerType::DateTime2);
        assert_eq!(Decimal::SQL_SERVER_TYPE, SqlServerType::Decimal);
        assert_eq!(Decimal::DEFAULT_PRECISION, Some(18));
        assert_eq!(Decimal::DEFAULT_SCALE, Some(2));
        assert_eq!(Vec::<u8>::SQL_SERVER_TYPE, SqlServerType::VarBinary);
        assert_eq!(Option::<String>::SQL_SERVER_TYPE, SqlServerType::NVarChar);
        assert_eq!(Option::<String>::DEFAULT_MAX_LENGTH, Some(255));
    }

    #[test]
    fn sql_type_mapping_roundtrips_supported_values() {
        let uuid = Uuid::nil();
        let date = NaiveDate::from_ymd_opt(2026, 4, 21).expect("valid date");
        let datetime = date.and_hms_opt(14, 30, 0).expect("valid datetime");
        let decimal = Decimal::new(12345, 2);

        assert_eq!(bool::from_sql_value(true.to_sql_value()), Ok(true));
        assert_eq!(i32::from_sql_value(42_i32.to_sql_value()), Ok(42));
        assert_eq!(i64::from_sql_value(99_i64.to_sql_value()), Ok(99));
        assert_eq!(f64::from_sql_value(10.5_f64.to_sql_value()), Ok(10.5));
        assert_eq!(
            String::from_sql_value("ana@example.com".to_string().to_sql_value()),
            Ok("ana@example.com".to_string())
        );
        assert_eq!(
            Vec::<u8>::from_sql_value(vec![1_u8, 2, 3].to_sql_value()),
            Ok(vec![1, 2, 3])
        );
        assert_eq!(Uuid::from_sql_value(uuid.to_sql_value()), Ok(uuid));
        assert_eq!(Decimal::from_sql_value(decimal.to_sql_value()), Ok(decimal));
        assert_eq!(NaiveDate::from_sql_value(date.to_sql_value()), Ok(date));
        assert_eq!(
            NaiveDateTime::from_sql_value(datetime.to_sql_value()),
            Ok(datetime)
        );
        assert_eq!(
            Option::<i64>::None.to_sql_value(),
            SqlValue::TypedNull(SqlServerType::BigInt)
        );
        assert_eq!(Option::<String>::from_sql_value(SqlValue::Null), Ok(None));
        assert_eq!(
            Option::<i64>::from_sql_value(SqlValue::TypedNull(SqlServerType::BigInt)),
            Ok(None)
        );
        assert_eq!(
            Option::<String>::from_sql_value(SqlValue::String("ana".to_string())),
            Ok(Some("ana".to_string()))
        );
    }

    #[test]
    fn quotes_sql_server_unicode_string_literals() {
        assert_eq!(quote_sql_string_literal("sales"), "N'sales'");
        assert_eq!(quote_sql_string_literal("O'Brien"), "N'O''Brien'");
        assert_eq!(
            quote_sql_string_literal("line 1\nline '2'"),
            "N'line 1\nline ''2'''"
        );
    }
}
