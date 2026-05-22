use sql_orm::prelude::*;

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const TEST_TABLE_NAME: &str = "dbo.sql_orm_public_aggregations";

#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_public_aggregations", schema = "dbo")]
struct AggregateOrder {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    customer_id: i64,
    total_cents: i64,
    tax_rate: f64,
    active: bool,
}

#[derive(DbContext)]
struct AggregateDb {
    pub orders: DbSet<AggregateOrder>,
}

#[derive(Debug, Clone, PartialEq)]
struct CustomerTotals {
    customer_id: i64,
    order_count: i64,
    total_cents: Option<i64>,
    average_tax_rate: Option<f64>,
}

impl FromRow for CustomerTotals {
    fn from_row<R: Row>(row: &R) -> Result<Self, OrmError> {
        Ok(Self {
            customer_id: row.get_required_typed::<i64>("customer_id")?,
            order_count: read_count(row, "order_count")?,
            total_cents: row.try_get_typed::<Option<i64>>("total_cents")?.flatten(),
            average_tax_rate: row
                .try_get_typed::<Option<f64>>("average_tax_rate")?
                .flatten(),
        })
    }
}

#[tokio::test]
async fn public_aggregation_api_roundtrips_against_real_sql_server() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public aggregation integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let db = AggregateDb::connect(&connection_string).await?;

    reset_test_table(&db).await?;
    announce_test_table(keep_tables);

    let result = async {
        seed_rows(&db).await?;

        let active_count = db
            .orders
            .query()
            .filter(AggregateOrder::active.eq(true))
            .count()
            .await?;
        assert_eq!(active_count, 3);

        let has_large_order = db
            .orders
            .query()
            .filter(AggregateOrder::total_cents.gt(2_000_i64))
            .exists()
            .await?;
        assert!(has_large_order);

        let has_missing_order = db
            .orders
            .query()
            .filter(AggregateOrder::total_cents.gt(10_000_i64))
            .any()
            .await?;
        assert!(!has_missing_order);

        let total_active_cents = db
            .orders
            .query()
            .filter(AggregateOrder::active.eq(true))
            .sum::<i64>(AggregateOrder::total_cents)
            .await?;
        assert_eq!(total_active_cents, Some(4_500));

        let average_active_tax = db
            .orders
            .query()
            .filter(AggregateOrder::active.eq(true))
            .avg::<f64>(AggregateOrder::tax_rate)
            .await?;
        assert_float_eq(average_active_tax.expect("average tax rate"), 0.2);

        let min_active_total = db
            .orders
            .query()
            .filter(AggregateOrder::active.eq(true))
            .min::<i64>(AggregateOrder::total_cents)
            .await?;
        assert_eq!(min_active_total, Some(1_000));

        let max_active_total = db
            .orders
            .query()
            .filter(AggregateOrder::active.eq(true))
            .max::<i64>(AggregateOrder::total_cents)
            .await?;
        assert_eq!(max_active_total, Some(2_000));

        let grouped = db
            .orders
            .query()
            .filter(AggregateOrder::active.eq(true))
            .group_by(AggregateOrder::customer_id)?
            .try_select_aggregate((
                AggregateProjection::group_key(AggregateOrder::customer_id),
                AggregateProjection::count_as("order_count"),
                AggregateProjection::sum_as(AggregateOrder::total_cents, "total_cents"),
                AggregateProjection::avg_as(AggregateOrder::tax_rate, "average_tax_rate"),
            ))?
            .having(AggregatePredicate::gte(
                AggregateExpr::sum(AggregateOrder::total_cents),
                SqlValue::I64(1_500),
            ))
            .order_by(AggregateOrderBy::asc(AggregateExpr::group_key(
                AggregateOrder::customer_id,
            )))
            .all_as::<CustomerTotals>()
            .await?;

        assert_eq!(grouped.len(), 2);
        assert_eq!(grouped[0].customer_id, 10);
        assert_eq!(grouped[0].order_count, 2);
        assert_eq!(grouped[0].total_cents, Some(3_000));
        assert_float_eq(
            grouped[0]
                .average_tax_rate
                .expect("customer 10 average tax rate"),
            0.15,
        );
        assert_eq!(grouped[1].customer_id, 20);
        assert_eq!(grouped[1].order_count, 1);
        assert_eq!(grouped[1].total_cents, Some(1_500));
        assert_float_eq(
            grouped[1]
                .average_tax_rate
                .expect("customer 20 average tax rate"),
            0.3,
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&db, keep_tables).await?;

    result
}

fn read_count<R: Row>(row: &R, column: &str) -> Result<i64, OrmError> {
    match row.get_required(column)? {
        SqlValue::I32(value) => Ok(i64::from(value)),
        SqlValue::I64(value) => Ok(value),
        value => Err(OrmError::mapping(format!(
            "expected `{column}` as SQL Server COUNT integer, got {value:?}"
        ))),
    }
}

fn assert_float_eq(actual: f64, expected: f64) {
    let difference = (actual - expected).abs();
    assert!(
        difference < 0.000_000_1,
        "expected {actual} to be within tolerance of {expected}"
    );
}

async fn seed_rows(db: &AggregateDb) -> Result<(), OrmError> {
    for (customer_id, total_cents, tax_rate, active) in [
        (10_i64, 1_000_i64, 0.1_f64, true),
        (10, 2_000, 0.2, true),
        (20, 1_500, 0.3, true),
        (30, 9_000, 0.4, false),
    ] {
        db.raw_exec(format!(
            "INSERT INTO {TEST_TABLE_NAME} (customer_id, total_cents, tax_rate, active) VALUES (@P1, @P2, @P3, @P4)"
        ))
        .params((customer_id, total_cents, tax_rate, active))
        .execute()
        .await?;
    }

    Ok(())
}

fn test_connection_string() -> Option<String> {
    std::env::var(TEST_CONNECTION_ENV)
        .ok()
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
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

fn announce_test_table(keep_tables: bool) {
    if keep_tables {
        eprintln!(
            "keeping public aggregation integration table `{TEST_TABLE_NAME}` because {KEEP_TABLES_ENV}=1"
        );
    } else {
        eprintln!("created public aggregation integration table `{TEST_TABLE_NAME}`");
    }
}

async fn reset_test_table(db: &AggregateDb) -> Result<(), OrmError> {
    db.raw_exec(format!(
        "IF OBJECT_ID('{TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {TEST_TABLE_NAME}"
    ))
    .execute()
    .await?;

    db.raw_exec(format!(
        "CREATE TABLE {TEST_TABLE_NAME} (\
            id BIGINT IDENTITY(1,1) PRIMARY KEY,\
            customer_id BIGINT NOT NULL,\
            total_cents BIGINT NOT NULL,\
            tax_rate FLOAT NOT NULL,\
            active BIT NOT NULL\
        )"
    ))
    .execute()
    .await?;

    Ok(())
}

async fn cleanup_test_table(db: &AggregateDb, keep_tables: bool) -> Result<(), OrmError> {
    if keep_tables {
        return Ok(());
    }

    db.raw_exec(format!(
        "IF OBJECT_ID('{TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {TEST_TABLE_NAME}"
    ))
    .execute()
    .await?;

    Ok(())
}
