use sql_orm::prelude::*;
use sql_orm::query::{CompiledQuery, Expr, SqlFunction};
use sql_orm::sqlserver::SqlServerCompiler;

const TEST_CONNECTION_ENV: &str = "SQL_ORM_TEST_CONNECTION_STRING";
const KEEP_TABLES_ENV: &str = "KEEP_TEST_TABLES";
const TEST_TABLE_NAME: &str = "dbo.sql_orm_public_projections";

#[allow(dead_code)]
#[derive(Entity, Debug, Clone, PartialEq)]
#[orm(table = "sql_orm_public_projections", schema = "dbo")]
struct ProjectionUser {
    #[orm(primary_key)]
    #[orm(identity)]
    id: i64,
    #[orm(length = 120)]
    name: String,
    active: bool,
}

#[derive(DbContext)]
struct ProjectionDb {
    pub users: DbSet<ProjectionUser>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
struct ProjectionSummary {
    id: i64,
    name: String,
    display_name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, FromRow)]
struct LowerNameProjection {
    #[orm(column = "lower_name")]
    value: String,
}

#[test]
fn public_projection_sql_preserves_aliases_and_parameter_order() {
    let compiled = SqlServerCompiler::compile_select(
        &sql_orm::query::SelectQuery::from_entity::<ProjectionUser>()
            .select(vec![
                SelectProjection::column(ProjectionUser::id),
                SelectProjection::expr_as(
                    Expr::function(SqlFunction::Lower, vec![Expr::from(ProjectionUser::name)]),
                    "lower_name",
                ),
            ])
            .filter(
                ProjectionUser::active
                    .eq(true)
                    .and(ProjectionUser::name.contains("a")),
            )
            .order_by(ProjectionUser::id.asc())
            .paginate(PageRequest::new(2, 10).to_pagination()),
    )
    .unwrap();

    assert_eq!(
        render_snapshot(&compiled),
        "SQL: SELECT [dbo].[sql_orm_public_projections].[id] AS [id], \
LOWER([dbo].[sql_orm_public_projections].[name]) AS [lower_name] \
FROM [dbo].[sql_orm_public_projections] \
WHERE (([dbo].[sql_orm_public_projections].[active] = @P1) \
AND ([dbo].[sql_orm_public_projections].[name] LIKE @P2 ESCAPE N'\\')) \
ORDER BY [dbo].[sql_orm_public_projections].[id] ASC \
OFFSET @P3 ROWS FETCH NEXT @P4 ROWS ONLY\n\
Params:\n\
1: Bool(true)\n\
2: String(\"%a%\")\n\
3: I64(10)\n\
4: I64(10)"
    );
}

#[tokio::test]
async fn public_projection_api_materializes_dtos_against_real_sql_server() -> Result<(), OrmError> {
    let Some(connection_string) = test_connection_string() else {
        eprintln!(
            "skipping public projection integration test because {TEST_CONNECTION_ENV} is not set"
        );
        return Ok(());
    };

    let keep_tables = keep_test_tables();
    let db = ProjectionDb::connect(&connection_string).await?;

    reset_test_table(&db).await?;
    announce_test_table(keep_tables);

    let result = async {
        db.raw_exec(format!(
            "INSERT INTO {TEST_TABLE_NAME} (name, active) VALUES (@P1, @P2)"
        ))
        .params(("Ana", true))
        .execute()
        .await?;

        db.raw_exec(format!(
            "INSERT INTO {TEST_TABLE_NAME} (name, active) VALUES (@P1, @P2)"
        ))
        .params(("BRUNO", true))
        .execute()
        .await?;

        db.raw_exec(format!(
            "INSERT INTO {TEST_TABLE_NAME} (name, active) VALUES (@P1, @P2)"
        ))
        .params(("Carla", false))
        .execute()
        .await?;

        let active_users = db
            .users
            .query()
            .select((
                ProjectionUser::id,
                ProjectionUser::name,
                SelectProjection::expr_as(
                    Expr::function(SqlFunction::Lower, vec![Expr::from(ProjectionUser::name)]),
                    "display_name",
                ),
            ))
            .filter(ProjectionUser::active.eq(true))
            .order_by(ProjectionUser::id.asc())
            .all_as::<ProjectionSummary>()
            .await?;

        assert_eq!(
            active_users,
            vec![
                ProjectionSummary {
                    id: 1,
                    name: "Ana".to_string(),
                    display_name: Some("ana".to_string()),
                },
                ProjectionSummary {
                    id: 2,
                    name: "BRUNO".to_string(),
                    display_name: Some("bruno".to_string()),
                },
            ]
        );

        let lower_name = db
            .users
            .query()
            .select(SelectProjection::expr_as(
                Expr::function(SqlFunction::Lower, vec![Expr::from(ProjectionUser::name)]),
                "lower_name",
            ))
            .filter(ProjectionUser::name.eq("BRUNO".to_string()))
            .first_as::<LowerNameProjection>()
            .await?;

        assert_eq!(
            lower_name,
            Some(LowerNameProjection {
                value: "bruno".to_string(),
            })
        );

        Ok(())
    }
    .await;

    cleanup_test_table(&db, keep_tables).await?;

    result
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
            "keeping public projection integration table `{TEST_TABLE_NAME}` because {KEEP_TABLES_ENV}=1"
        );
    } else {
        eprintln!("created public projection integration table `{TEST_TABLE_NAME}`");
    }
}

async fn reset_test_table(db: &ProjectionDb) -> Result<(), OrmError> {
    db.raw_exec(format!(
        "IF OBJECT_ID('{TEST_TABLE_NAME}', 'U') IS NOT NULL DROP TABLE {TEST_TABLE_NAME}"
    ))
    .execute()
    .await?;

    db.raw_exec(format!(
        "CREATE TABLE {TEST_TABLE_NAME} (\
            id BIGINT IDENTITY(1,1) PRIMARY KEY,\
            name NVARCHAR(120) NOT NULL,\
            active BIT NOT NULL\
        )"
    ))
    .execute()
    .await?;

    Ok(())
}

async fn cleanup_test_table(db: &ProjectionDb, keep_tables: bool) -> Result<(), OrmError> {
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

fn render_snapshot(compiled: &CompiledQuery) -> String {
    let params = compiled
        .params
        .iter()
        .enumerate()
        .map(|(index, value)| format!("{}: {}", index + 1, render_sql_value(value)))
        .collect::<Vec<_>>();

    if params.is_empty() {
        format!("SQL: {}\nParams:\n<none>", compiled.sql)
    } else {
        format!("SQL: {}\nParams:\n{}", compiled.sql, params.join("\n"))
    }
}

fn render_sql_value(value: &SqlValue) -> String {
    match value {
        SqlValue::Null => "Null".to_string(),
        SqlValue::TypedNull(sql_type) => format!("TypedNull({sql_type:?})"),
        SqlValue::Bool(value) => format!("Bool({value})"),
        SqlValue::I32(value) => format!("I32({value})"),
        SqlValue::I64(value) => format!("I64({value})"),
        SqlValue::F64(value) => format!("F64({value})"),
        SqlValue::String(value) => format!("String({value:?})"),
        SqlValue::Bytes(value) => format!("Bytes({value:?})"),
        SqlValue::Uuid(value) => format!("Uuid({value})"),
        SqlValue::Decimal(value) => format!("Decimal({value})"),
        SqlValue::Date(value) => format!("Date({value})"),
        SqlValue::DateTime(value) => format!("DateTime({value})"),
    }
}
