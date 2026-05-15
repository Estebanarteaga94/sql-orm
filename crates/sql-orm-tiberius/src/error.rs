use sql_orm_core::OrmError;
use tiberius::error::{Error as TiberiusError, IoErrorKind};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TiberiusErrorContext {
    ConnectTcp,
    ConfigureTcp,
    InitializeClient,
    ExecuteQuery,
    ReadRowValue,
}

pub(crate) fn map_tiberius_error(
    error: &tiberius::error::Error,
    context: TiberiusErrorContext,
) -> OrmError {
    if error.is_deadlock() {
        return OrmError::new("SQL Server deadlock detected");
    }

    match context {
        TiberiusErrorContext::ConnectTcp => {
            OrmError::new("failed to connect to SQL Server over TCP")
        }
        TiberiusErrorContext::ConfigureTcp => {
            OrmError::new("failed to configure SQL Server TCP stream")
        }
        TiberiusErrorContext::InitializeClient => {
            OrmError::new("failed to initialize Tiberius client")
        }
        TiberiusErrorContext::ExecuteQuery => {
            OrmError::new(format!("failed to execute SQL Server query: {error}"))
        }
        TiberiusErrorContext::ReadRowValue => OrmError::new("failed to read SQL Server row value"),
    }
}

pub(crate) fn is_transient_tiberius_error(error: &TiberiusError) -> bool {
    if error.is_deadlock() {
        return true;
    }

    match error {
        TiberiusError::Io { kind, .. } => matches!(
            kind,
            IoErrorKind::TimedOut
                | IoErrorKind::ConnectionReset
                | IoErrorKind::ConnectionAborted
                | IoErrorKind::BrokenPipe
                | IoErrorKind::Interrupted
                | IoErrorKind::UnexpectedEof
                | IoErrorKind::WouldBlock
                | IoErrorKind::NotConnected
        ),
        TiberiusError::Server(_) => matches!(
            error.code(),
            Some(1222 | 40197 | 40501 | 40613 | 49918 | 49919 | 49920)
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::{TiberiusErrorContext, is_transient_tiberius_error, map_tiberius_error};
    use tiberius::error::{Error, IoErrorKind};

    #[test]
    fn maps_contextual_driver_error_to_orm_error() {
        let error = Error::Conversion("boom".into());

        assert_eq!(
            map_tiberius_error(&error, TiberiusErrorContext::ExecuteQuery).message(),
            "failed to execute SQL Server query: Conversion error: boom"
        );
    }

    #[test]
    fn classifies_transient_io_errors_for_retry() {
        let error = Error::Io {
            kind: IoErrorKind::TimedOut,
            message: "timed out".to_string(),
        };

        assert!(is_transient_tiberius_error(&error));
    }

    #[test]
    fn ignores_non_transient_conversion_errors_for_retry() {
        let error = Error::Conversion("boom".into());

        assert!(!is_transient_tiberius_error(&error));
    }
}
