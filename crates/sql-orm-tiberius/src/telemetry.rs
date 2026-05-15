use crate::config::{MssqlParameterLogMode, MssqlSlowQueryOptions, MssqlTracingOptions};
use crate::parameter::PreparedQuery;
use core::fmt::Display;
use sql_orm_core::OrmError;
use std::time::{Duration, Instant};
use tracing::Instrument;

pub(crate) async fn trace_connection<F, T>(
    tracing_options: MssqlTracingOptions,
    server_addr: &str,
    connect_timeout: Option<Duration>,
    future: F,
) -> Result<T, OrmError>
where
    F: core::future::Future<Output = Result<T, OrmError>>,
{
    if !tracing_options.enabled {
        return future.await;
    }

    let timeout_ms = format_timeout_ms(connect_timeout);
    let span = tracing::info_span!(
        "sql_orm.connection",
        server_addr = %server_addr,
        timeout_ms = %timeout_ms,
    );

    if tracing_options.emit_start_event {
        tracing::info!(
            target: "orm.connection.start",
            server_addr = %server_addr,
            timeout_ms = %timeout_ms,
        );
    }

    let started_at = Instant::now();
    let result = future.instrument(span).await;
    let duration_ms = started_at.elapsed().as_millis();

    match &result {
        Ok(_) if tracing_options.emit_finish_event => tracing::info!(
            target: "orm.connection.finish",
            server_addr = %server_addr,
            timeout_ms = %timeout_ms,
            duration_ms,
        ),
        Err(error) if tracing_options.emit_error_event => tracing::error!(
            target: "orm.connection.error",
            server_addr = %server_addr,
            timeout_ms = %timeout_ms,
            duration_ms,
            error = %error,
        ),
        _ => {}
    }

    result
}

pub(crate) async fn trace_query<F, T, E>(
    tracing_options: MssqlTracingOptions,
    slow_query_options: MssqlSlowQueryOptions,
    trace: QueryTrace,
    future: F,
) -> Result<T, E>
where
    F: core::future::Future<Output = Result<T, E>>,
    E: Display,
{
    if !tracing_options.enabled && !slow_query_options.enabled {
        return future.await;
    }

    if tracing_options.enabled && tracing_options.emit_start_event {
        tracing::info!(
            target: "orm.query.start",
            server_addr = %trace.server_addr,
            operation = %trace.operation,
            timeout_ms = %trace.timeout_ms,
            param_count = trace.param_count,
            sql = %trace.sql,
            params_mode = %trace.params_mode,
            params = %trace.params,
        );
    }

    let started_at = Instant::now();
    let result = if tracing_options.enabled {
        let span = tracing::info_span!(
            "sql_orm.query",
            server_addr = %trace.server_addr,
            operation = %trace.operation,
            timeout_ms = %trace.timeout_ms,
            param_count = trace.param_count,
            sql = %trace.sql,
            params_mode = %trace.params_mode,
            params = %trace.params,
        );

        future.instrument(span).await
    } else {
        future.await
    };
    let duration = started_at.elapsed();
    let duration_ms = duration.as_millis();

    match &result {
        Ok(_) if tracing_options.enabled && tracing_options.emit_finish_event => tracing::info!(
            target: "orm.query.finish",
            server_addr = %trace.server_addr,
            operation = %trace.operation,
            timeout_ms = %trace.timeout_ms,
            param_count = trace.param_count,
            sql = %trace.sql,
            params_mode = %trace.params_mode,
            params = %trace.params,
            duration_ms,
        ),
        Err(error) if tracing_options.enabled && tracing_options.emit_error_event => {
            tracing::error!(
                target: "orm.query.error",
                server_addr = %trace.server_addr,
                operation = %trace.operation,
                timeout_ms = %trace.timeout_ms,
                param_count = trace.param_count,
                sql = %trace.sql,
                params_mode = %trace.params_mode,
                params = %trace.params,
                duration_ms,
                error = %error,
            )
        }
        _ => {}
    }

    if should_emit_slow_query(duration, slow_query_options) {
        tracing::warn!(
            target: "orm.query.slow",
            server_addr = %trace.server_addr,
            operation = %trace.operation,
            timeout_ms = %trace.timeout_ms,
            threshold_ms = slow_query_options.threshold.as_millis(),
            duration_ms,
            param_count = trace.param_count,
            sql = %trace.sql,
            params_mode = %param_mode_label(slow_query_options.parameter_logging),
            params = %render_params(slow_query_options.parameter_logging),
        );
    }

    result
}

pub(crate) async fn trace_transaction_command<F, T>(
    tracing_options: MssqlTracingOptions,
    server_addr: &str,
    query_timeout: Option<Duration>,
    command: &'static str,
    future: F,
) -> Result<T, OrmError>
where
    F: core::future::Future<Output = Result<T, OrmError>>,
{
    if !tracing_options.enabled {
        return future.await;
    }

    let operation = classify_sql(command);
    let timeout_ms = format_timeout_ms(query_timeout);
    let span = tracing::info_span!(
        "sql_orm.transaction",
        server_addr = %server_addr,
        operation = %operation,
        timeout_ms = %timeout_ms,
    );

    let started_at = Instant::now();
    let result = future.instrument(span).await;
    let duration_ms = started_at.elapsed().as_millis();

    match &result {
        Ok(_) => match operation {
            "begin" => tracing::info!(
                target: "orm.transaction.begin",
                server_addr = %server_addr,
                operation = %operation,
                timeout_ms = %timeout_ms,
                duration_ms,
            ),
            "commit" => tracing::info!(
                target: "orm.transaction.commit",
                server_addr = %server_addr,
                operation = %operation,
                timeout_ms = %timeout_ms,
                duration_ms,
            ),
            "rollback" => tracing::info!(
                target: "orm.transaction.rollback",
                server_addr = %server_addr,
                operation = %operation,
                timeout_ms = %timeout_ms,
                duration_ms,
            ),
            _ => tracing::info!(
                target: "orm.transaction.unknown",
                server_addr = %server_addr,
                operation = %operation,
                timeout_ms = %timeout_ms,
                duration_ms,
            ),
        },
        Err(error) if tracing_options.emit_error_event => tracing::error!(
            target: "orm.transaction.error",
            server_addr = %server_addr,
            operation = %operation,
            timeout_ms = %timeout_ms,
            duration_ms,
            error = %error,
        ),
        _ => {}
    }

    result
}

pub(crate) struct QueryTrace {
    server_addr: String,
    operation: &'static str,
    timeout_ms: String,
    param_count: usize,
    sql: String,
    params_mode: &'static str,
    params: &'static str,
}

impl QueryTrace {
    pub(crate) fn new(
        server_addr: &str,
        query_timeout: Option<Duration>,
        tracing_options: MssqlTracingOptions,
        prepared: &PreparedQuery,
    ) -> Self {
        Self {
            server_addr: server_addr.to_string(),
            operation: classify_sql(&prepared.sql),
            timeout_ms: format_timeout_ms(query_timeout),
            param_count: prepared.params.len(),
            sql: prepared.sql.clone(),
            params_mode: param_mode_label(tracing_options.parameter_logging),
            params: render_params(tracing_options.parameter_logging),
        }
    }
}

fn render_params(mode: MssqlParameterLogMode) -> &'static str {
    match mode {
        MssqlParameterLogMode::Disabled => "disabled",
        MssqlParameterLogMode::Redacted => "[REDACTED]",
    }
}

fn param_mode_label(mode: MssqlParameterLogMode) -> &'static str {
    match mode {
        MssqlParameterLogMode::Disabled => "disabled",
        MssqlParameterLogMode::Redacted => "redacted",
    }
}

fn format_timeout_ms(duration: Option<Duration>) -> String {
    duration
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|| "none".to_string())
}

fn should_emit_slow_query(duration: Duration, slow_query_options: MssqlSlowQueryOptions) -> bool {
    slow_query_options.enabled && duration >= slow_query_options.threshold
}

pub(crate) fn classify_sql(sql: &str) -> &'static str {
    sql.split_whitespace()
        .next()
        .map(|token| token.to_ascii_uppercase())
        .as_deref()
        .map(|token| match token {
            "SELECT" => "select",
            "INSERT" => "insert",
            "UPDATE" => "update",
            "DELETE" => "delete",
            "BEGIN" => "begin",
            "COMMIT" => "commit",
            "ROLLBACK" => "rollback",
            _ => "unknown",
        })
        .unwrap_or("unknown")
}

#[cfg(test)]
mod tests {
    use super::{
        classify_sql, format_timeout_ms, param_mode_label, render_params, should_emit_slow_query,
    };
    use crate::config::{MssqlParameterLogMode, MssqlSlowQueryOptions};
    use std::time::Duration;

    #[test]
    fn classifies_known_sql_operations() {
        assert_eq!(classify_sql("SELECT * FROM [dbo].[users]"), "select");
        assert_eq!(
            classify_sql("insert into [dbo].[users] values (@P1)"),
            "insert"
        );
        assert_eq!(
            classify_sql("UPDATE [dbo].[users] SET [active] = @P1"),
            "update"
        );
        assert_eq!(classify_sql("DELETE FROM [dbo].[users]"), "delete");
        assert_eq!(classify_sql("BEGIN TRANSACTION"), "begin");
        assert_eq!(classify_sql("COMMIT TRANSACTION"), "commit");
        assert_eq!(classify_sql("ROLLBACK TRANSACTION"), "rollback");
    }

    #[test]
    fn renders_parameter_modes_without_exposing_values() {
        assert_eq!(
            param_mode_label(MssqlParameterLogMode::Disabled),
            "disabled"
        );
        assert_eq!(
            param_mode_label(MssqlParameterLogMode::Redacted),
            "redacted"
        );
        assert_eq!(render_params(MssqlParameterLogMode::Disabled), "disabled");
        assert_eq!(render_params(MssqlParameterLogMode::Redacted), "[REDACTED]");
    }

    #[test]
    fn formats_optional_timeout_as_stable_field() {
        assert_eq!(format_timeout_ms(None), "none");
        assert_eq!(format_timeout_ms(Some(Duration::from_millis(250))), "250");
    }

    #[test]
    fn only_marks_slow_queries_when_threshold_is_reached_and_enabled() {
        let enabled = MssqlSlowQueryOptions::enabled(Duration::from_millis(250));
        let disabled = MssqlSlowQueryOptions::disabled();

        assert!(!should_emit_slow_query(Duration::from_millis(249), enabled));
        assert!(should_emit_slow_query(Duration::from_millis(250), enabled));
        assert!(!should_emit_slow_query(
            Duration::from_millis(900),
            disabled
        ));
    }
}
