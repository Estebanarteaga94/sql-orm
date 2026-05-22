use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};
use core::sync::atomic::{AtomicU64, Ordering};
use rust_decimal::Decimal;
use sql_orm_core::{FromRow, OrmError, Row, SqlValue};
use sql_orm_query::CompiledQuery;
use sql_orm_tiberius::MssqlConnection;
use uuid::Uuid;

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";

static NEXT_TABLE_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, PartialEq, Eq)]
struct IntegrationUser {
    id: i32,
    email: String,
    active: bool,
    created_at: NaiveDateTime,
}

impl FromRow for IntegrationUser {
    fn from_row<R: sql_orm_core::Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            id: row.get_required_typed::<i32>("id")?,
            email: row.get_required_typed::<String>("email")?,
            active: row.get_required_typed::<bool>("active")?,
            created_at: row.get_required_typed::<NaiveDateTime>("created_at")?,
        })
    }
}

#[derive(Debug, Clone, PartialEq)]
struct SupportedSqlTypes {
    bit_value: SqlValue,
    tinyint_value: SqlValue,
    smallint_value: SqlValue,
    int_value: SqlValue,
    bigint_value: SqlValue,
    real_value: SqlValue,
    float_value: SqlValue,
    decimal_value: SqlValue,
    numeric_value: SqlValue,
    uniqueidentifier_value: SqlValue,
    date_value: SqlValue,
    time_value: SqlValue,
    datetime_value: SqlValue,
    datetime2_value: SqlValue,
    datetimeoffset_value: SqlValue,
    smalldatetime_value: SqlValue,
    money_value: SqlValue,
    smallmoney_value: SqlValue,
    char_value: SqlValue,
    varchar_value: SqlValue,
    nchar_value: SqlValue,
    nvarchar_value: SqlValue,
    text_value: SqlValue,
    ntext_value: SqlValue,
    binary_value: SqlValue,
    varbinary_value: SqlValue,
    image_value: SqlValue,
    null_money_value: SqlValue,
}

impl FromRow for SupportedSqlTypes {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            bit_value: row.get_required("bit_value")?,
            tinyint_value: row.get_required("tinyint_value")?,
            smallint_value: row.get_required("smallint_value")?,
            int_value: row.get_required("int_value")?,
            bigint_value: row.get_required("bigint_value")?,
            real_value: row.get_required("real_value")?,
            float_value: row.get_required("float_value")?,
            decimal_value: row.get_required("decimal_value")?,
            numeric_value: row.get_required("numeric_value")?,
            uniqueidentifier_value: row.get_required("uniqueidentifier_value")?,
            date_value: row.get_required("date_value")?,
            time_value: row.get_required("time_value")?,
            datetime_value: row.get_required("datetime_value")?,
            datetime2_value: row.get_required("datetime2_value")?,
            datetimeoffset_value: row.get_required("datetimeoffset_value")?,
            smalldatetime_value: row.get_required("smalldatetime_value")?,
            money_value: row.get_required("money_value")?,
            smallmoney_value: row.get_required("smallmoney_value")?,
            char_value: row.get_required("char_value")?,
            varchar_value: row.get_required("varchar_value")?,
            nchar_value: row.get_required("nchar_value")?,
            nvarchar_value: row.get_required("nvarchar_value")?,
            text_value: row.get_required("text_value")?,
            ntext_value: row.get_required("ntext_value")?,
            binary_value: row.get_required("binary_value")?,
            varbinary_value: row.get_required("varbinary_value")?,
            image_value: row.get_required("image_value")?,
            null_money_value: row.get_required("null_money_value")?,
        })
    }
}

#[tokio::test]
async fn sqlserver_adapter_executes_and_maps_rows_against_real_database() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!("skipping SQL Server integration test because {TEST_CONNECTION_ENV} is not set");
        return Ok(());
    };

    let mut connection = MssqlConnection::connect(&connection_string).await?;
    let table_name = unique_table_name();
    let first_created_at = fixed_datetime(2026, 4, 23, 10, 20, 30);
    let second_created_at = fixed_datetime(2026, 4, 23, 11, 21, 31);
    let keep_tables = keep_test_tables();

    create_test_table(&mut connection, &table_name).await?;
    announce_test_table(&table_name, keep_tables);

    let insert_first = connection
        .execute(CompiledQuery::new(
            format!("INSERT INTO {table_name} (email, active, created_at) VALUES (@P1, @P2, @P3)"),
            vec![
                SqlValue::String("ana@example.com".to_string()),
                SqlValue::Bool(true),
                SqlValue::DateTime(first_created_at),
            ],
        ))
        .await?;

    let insert_second = connection
        .execute(CompiledQuery::new(
            format!("INSERT INTO {table_name} (email, active, created_at) VALUES (@P1, @P2, @P3)"),
            vec![
                SqlValue::String("bruno@example.com".to_string()),
                SqlValue::Bool(false),
                SqlValue::DateTime(second_created_at),
            ],
        ))
        .await?;

    assert_eq!(insert_first.total(), 1);
    assert_eq!(insert_second.total(), 1);

    let fetched_one = connection
        .fetch_one::<IntegrationUser>(CompiledQuery::new(
            format!(
                "SELECT TOP (1) id, email, active, created_at \
                 FROM {table_name} WHERE email = @P1 ORDER BY id ASC"
            ),
            vec![SqlValue::String("ana@example.com".to_string())],
        ))
        .await?;

    assert_eq!(
        fetched_one,
        Some(IntegrationUser {
            id: 1,
            email: "ana@example.com".to_string(),
            active: true,
            created_at: first_created_at,
        })
    );

    let fetched_all = connection
        .fetch_all::<IntegrationUser>(CompiledQuery::new(
            format!("SELECT id, email, active, created_at FROM {table_name} ORDER BY id ASC"),
            vec![],
        ))
        .await?;

    assert_eq!(
        fetched_all,
        vec![
            IntegrationUser {
                id: 1,
                email: "ana@example.com".to_string(),
                active: true,
                created_at: first_created_at,
            },
            IntegrationUser {
                id: 2,
                email: "bruno@example.com".to_string(),
                active: false,
                created_at: second_created_at,
            },
        ]
    );

    cleanup_test_table(&mut connection, &table_name, keep_tables).await?;

    Ok(())
}

#[tokio::test]
async fn sqlserver_adapter_maps_common_sql_server_column_types() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!("skipping SQL Server integration test because {TEST_CONNECTION_ENV} is not set");
        return Ok(());
    };

    let mut connection = MssqlConnection::connect(&connection_string).await?;
    let row = connection
        .fetch_one::<SupportedSqlTypes>(CompiledQuery::new(
            "\
            SELECT \
                CAST(1 AS bit) AS bit_value,\
                CAST(255 AS tinyint) AS tinyint_value,\
                CAST(-123 AS smallint) AS smallint_value,\
                CAST(123456 AS int) AS int_value,\
                CAST(1234567890123 AS bigint) AS bigint_value,\
                CAST(3.25 AS real) AS real_value,\
                CAST(6.5 AS float) AS float_value,\
                CAST(1234.56 AS decimal(10, 2)) AS decimal_value,\
                CAST(789.01 AS numeric(10, 2)) AS numeric_value,\
                CAST('6F9619FF-8B86-D011-B42D-00C04FC964FF' AS uniqueidentifier) AS uniqueidentifier_value,\
                CAST('2026-04-28' AS date) AS date_value,\
                CAST('12:34:56.1234567' AS time) AS time_value,\
                CAST('2026-04-28T12:34:56' AS datetime) AS datetime_value,\
                CAST('2026-04-28T12:34:56.1234567' AS datetime2) AS datetime2_value,\
                CAST('2026-04-28T12:34:56.1234567-05:00' AS datetimeoffset) AS datetimeoffset_value,\
                CAST('2026-04-28T12:34:00' AS smalldatetime) AS smalldatetime_value,\
                CAST(12.34 AS money) AS money_value,\
                CAST(5.67 AS smallmoney) AS smallmoney_value,\
                CAST('abc' AS char(3)) AS char_value,\
                CAST('def' AS varchar(3)) AS varchar_value,\
                CAST(N'ghi' AS nchar(3)) AS nchar_value,\
                CAST(N'jkl' AS nvarchar(3)) AS nvarchar_value,\
                CAST('text-value' AS text) AS text_value,\
                CAST(N'ntext-value' AS ntext) AS ntext_value,\
                CAST(0x010203 AS binary(3)) AS binary_value,\
                CAST(0x040506 AS varbinary(3)) AS varbinary_value,\
                CAST(0x070809 AS image) AS image_value,\
                CAST(NULL AS money) AS null_money_value\
            ",
            vec![],
        ))
        .await?
        .expect("SELECT without FROM should return one row");

    assert_eq!(row.bit_value, SqlValue::Bool(true));
    assert_eq!(row.tinyint_value, SqlValue::I32(255));
    assert_eq!(row.smallint_value, SqlValue::I32(-123));
    assert_eq!(row.int_value, SqlValue::I32(123456));
    assert_eq!(row.bigint_value, SqlValue::I64(1234567890123));
    assert_eq!(row.real_value, SqlValue::F64(3.25));
    assert_eq!(row.float_value, SqlValue::F64(6.5));
    assert_eq!(
        row.decimal_value,
        SqlValue::Decimal(Decimal::new(123456, 2))
    );
    assert_eq!(row.numeric_value, SqlValue::Decimal(Decimal::new(78901, 2)));
    assert_eq!(
        row.uniqueidentifier_value,
        SqlValue::Uuid(Uuid::parse_str("6F9619FF-8B86-D011-B42D-00C04FC964FF").unwrap())
    );
    assert_eq!(
        row.date_value,
        SqlValue::Date(NaiveDate::from_ymd_opt(2026, 4, 28).unwrap())
    );
    assert_eq!(
        row.time_value,
        SqlValue::Time(NaiveTime::from_hms_nano_opt(12, 34, 56, 123_456_700).unwrap())
    );
    assert_eq!(
        row.datetime_value,
        SqlValue::DateTime(fixed_datetime(2026, 4, 28, 12, 34, 56))
    );
    assert_eq!(
        row.datetime2_value,
        SqlValue::DateTime(
            NaiveDate::from_ymd_opt(2026, 4, 28)
                .unwrap()
                .and_hms_nano_opt(12, 34, 56, 123_456_700)
                .unwrap()
        )
    );
    assert_eq!(
        row.datetimeoffset_value,
        SqlValue::DateTimeOffset(
            DateTime::parse_from_rfc3339("2026-04-28T12:34:56.1234567-05:00").unwrap()
        )
    );
    assert_eq!(
        row.smalldatetime_value,
        SqlValue::DateTime(fixed_datetime(2026, 4, 28, 12, 34, 0))
    );
    assert_eq!(row.money_value, SqlValue::F64(12.34));
    assert_eq!(row.smallmoney_value, SqlValue::F64(5.67));
    assert_eq!(row.char_value, SqlValue::String("abc".to_string()));
    assert_eq!(row.varchar_value, SqlValue::String("def".to_string()));
    assert_eq!(row.nchar_value, SqlValue::String("ghi".to_string()));
    assert_eq!(row.nvarchar_value, SqlValue::String("jkl".to_string()));
    assert_eq!(row.text_value, SqlValue::String("text-value".to_string()));
    assert_eq!(row.ntext_value, SqlValue::String("ntext-value".to_string()));
    assert_eq!(row.binary_value, SqlValue::Bytes(vec![1, 2, 3]));
    assert_eq!(row.varbinary_value, SqlValue::Bytes(vec![4, 5, 6]));
    assert_eq!(row.image_value, SqlValue::Bytes(vec![7, 8, 9]));
    assert_eq!(row.null_money_value, SqlValue::Null);

    Ok(())
}

#[tokio::test]
async fn sqlserver_adapter_surfaces_missing_rows_as_none() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!("skipping SQL Server integration test because {TEST_CONNECTION_ENV} is not set");
        return Ok(());
    };

    let mut connection = MssqlConnection::connect(&connection_string).await?;
    let table_name = unique_table_name();
    let keep_tables = keep_test_tables();

    create_test_table(&mut connection, &table_name).await?;
    announce_test_table(&table_name, keep_tables);

    let fetched = connection
        .fetch_one::<IntegrationUser>(CompiledQuery::new(
            format!(
                "SELECT TOP (1) id, email, active, created_at \
                 FROM {table_name} WHERE email = @P1 ORDER BY id ASC"
            ),
            vec![SqlValue::String("missing@example.com".to_string())],
        ))
        .await?;

    assert_eq!(fetched, None);

    cleanup_test_table(&mut connection, &table_name, keep_tables).await?;

    Ok(())
}

#[tokio::test]
async fn sqlserver_adapter_health_check_succeeds_against_real_database() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!("skipping SQL Server integration test because {TEST_CONNECTION_ENV} is not set");
        return Ok(());
    };

    let mut connection = MssqlConnection::connect(&connection_string).await?;
    connection.health_check().await?;

    Ok(())
}

fn test_connection_string() -> Option<String> {
    std::env::var(TEST_CONNECTION_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn fixed_datetime(
    year: i32,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    second: u32,
) -> NaiveDateTime {
    NaiveDate::from_ymd_opt(year, month, day)
        .unwrap()
        .and_hms_opt(hour, minute, second)
        .unwrap()
}

fn unique_table_name() -> String {
    let table_id = NEXT_TABLE_ID.fetch_add(1, Ordering::Relaxed);
    let process_id = std::process::id();

    format!("tempdb.dbo.sql_orm_integration_{process_id}_{table_id}")
}

fn keep_test_tables() -> bool {
    matches!(
        std::env::var(KEEP_TABLES_ENV)
            .ok()
            .map(|value| value.trim().to_ascii_lowercase())
            .as_deref(),
        Some("1" | "true" | "yes" | "on")
    )
}

fn announce_test_table(table_name: &str, keep_tables: bool) {
    if keep_tables {
        eprintln!(
            "keeping SQL Server integration table `{table_name}` because {KEEP_TABLES_ENV}=1"
        );
    } else {
        eprintln!("created SQL Server integration table `{table_name}`");
    }
}

async fn create_test_table(
    connection: &mut MssqlConnection,
    table_name: &str,
) -> Result<(), OrmError> {
    connection
        .execute(CompiledQuery::new(
            format!(
                "CREATE TABLE {table_name} (\
                    id INT IDENTITY(1,1) PRIMARY KEY,\
                    email NVARCHAR(180) NOT NULL,\
                    active BIT NOT NULL,\
                    created_at DATETIME2 NOT NULL\
                )"
            ),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn drop_test_table(
    connection: &mut MssqlConnection,
    table_name: &str,
) -> Result<(), OrmError> {
    connection
        .execute(CompiledQuery::new(
            format!("IF OBJECT_ID('{table_name}', 'U') IS NOT NULL DROP TABLE {table_name}"),
            vec![],
        ))
        .await?;

    Ok(())
}

async fn cleanup_test_table(
    connection: &mut MssqlConnection,
    table_name: &str,
    keep_tables: bool,
) -> Result<(), OrmError> {
    if keep_tables {
        return Ok(());
    }

    drop_test_table(connection, table_name).await
}
