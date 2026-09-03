//! User-facing SQL Server error formatting and credential redaction.

use deadpool::managed::{PoolError, TimeoutType};
use mssql_tds::error::{Error as TdsError, SqlErrorInfo};
use mssql_tiberius_bridge::Error as BridgeError;

use crate::models::ConnectionParams;

/// Format a bridge error without losing SQL Server's structured error token.
pub fn format_bridge_error(error: &BridgeError, ssl_mode: Option<&str>) -> String {
    match error {
        BridgeError::Tds(error) => format_tds_error(error, ssl_mode),
        BridgeError::Conversion(message)
            if message
                .trim_start()
                .to_ascii_lowercase()
                .starts_with("query timed out") =>
        {
            format!("SQL Server timeout: {message}")
        }
        BridgeError::ColumnNotFound(_)
        | BridgeError::ColumnIndexOutOfBounds { .. }
        | BridgeError::Conversion(_) => format!("SQL Server data conversion failure: {error}"),
        BridgeError::Pool(message) => format!("SQL Server connection-pool failure: {message}"),
    }
}

/// Format errors from the underlying Microsoft TDS client.
pub fn format_tds_error(error: &TdsError, ssl_mode: Option<&str>) -> String {
    match error {
        TdsError::SqlServerError { errors } => format_server_errors(errors),
        TdsError::TlsError(_)
        | TdsError::TlsHandshakeError { .. }
        | TdsError::CertificateNotFound { .. }
        | TdsError::InvalidCertificateFormat { .. }
        | TdsError::CertificateExpired
        | TdsError::CertificateMismatch
        | TdsError::CertificateFileIoError { .. }
        | TdsError::NoServerCertificate => format_tls_error(error, ssl_mode),
        TdsError::TimeoutError(_) => format!("SQL Server timeout: {error}"),
        TdsError::Io(_)
        | TdsError::ConnectionError(_)
        | TdsError::ConnectionClosed(_)
        | TdsError::Redirection { .. }
        | TdsError::SessionRecoveryFailed { .. }
        | TdsError::SessionNotRecoverable(_)
        | TdsError::ReconnectionValidationFailed(_) => {
            format!("SQL Server connection failure: {error}")
        }
        TdsError::Security(_) => format!("SQL Server authentication failure: {error}"),
        TdsError::ProtocolError(_)
        | TdsError::OperationCancelledError(_)
        | TdsError::UsageError(_)
        | TdsError::ImplementationError(_)
        | TdsError::UnimplementedFeature { .. }
        | TdsError::TypeConversionError(_)
        | TdsError::UnsupportedEncoding { .. }
        | TdsError::BulkCopyError(_) => format!("SQL Server driver failure: {error}"),
    }
}

/// Format a deadpool checkout failure, preserving backend error structure.
pub fn format_pool_error(error: &PoolError<BridgeError>, ssl_mode: Option<&str>) -> String {
    match error {
        PoolError::Backend(error) => format_bridge_error(error, ssl_mode),
        PoolError::Timeout(kind) => {
            let operation = match kind {
                TimeoutType::Wait => "waiting for a pooled connection",
                TimeoutType::Create => "opening a connection",
                TimeoutType::Recycle => "resetting a pooled connection",
            };
            format!("SQL Server timeout while {operation}")
        }
        PoolError::Closed => "SQL Server connection-pool failure: pool is closed".to_string(),
        PoolError::NoRuntimeSpecified => {
            "SQL Server connection-pool failure: no async runtime is configured".to_string()
        }
        PoolError::PostCreateHook(_) => {
            "SQL Server connection-pool failure: post-create validation failed".to_string()
        }
    }
}

/// Remove connection secrets defensively before an error crosses JSON-RPC.
pub fn redact_connection_secrets(message: String, params: &ConnectionParams) -> String {
    let mut redacted = message;
    for secret in [
        params.password.as_deref(),
        params.connection_string.as_deref(),
    ]
    .into_iter()
    .flatten()
    .filter(|secret| !secret.is_empty())
    {
        redacted = redacted.replace(secret, "<redacted>");
    }
    redacted
}

/// A server, timeout, or transport failure can leave unread TDS packets
/// behind. Such a connection must be discarded instead of reset and reused.
pub fn bridge_error_requires_discard(error: &BridgeError) -> bool {
    match error {
        BridgeError::Conversion(message) => message
            .trim_start()
            .to_ascii_lowercase()
            .starts_with("query timed out"),
        BridgeError::Tds(error) => matches!(
            error,
            TdsError::Io(_)
                | TdsError::ConnectionError(_)
                | TdsError::SqlServerError { .. }
                | TdsError::ConnectionClosed(_)
                | TdsError::ProtocolError(_)
                | TdsError::TlsError(_)
                | TdsError::TlsHandshakeError { .. }
                | TdsError::TimeoutError(_)
                | TdsError::OperationCancelledError(_)
                | TdsError::SessionRecoveryFailed { .. }
                | TdsError::SessionNotRecoverable(_)
                | TdsError::ReconnectionValidationFailed(_)
        ),
        _ => false,
    }
}

fn format_server_errors(errors: &[SqlErrorInfo]) -> String {
    if errors.is_empty() {
        return "SQL Server error: no server error details were returned".to_string();
    }
    errors
        .iter()
        .map(format_server_error)
        .collect::<Vec<_>>()
        .join("; ")
}

fn format_server_error(error: &SqlErrorInfo) -> String {
    let mut details = vec![
        server_error_category(error.number).to_string(),
        format!("severity {}", error.class),
        format!("state {}", error.state),
    ];
    if let Some(procedure) = error.proc_name.as_deref().filter(|name| !name.is_empty()) {
        details.push(format!("procedure '{procedure}'"));
    }
    if let Some(line) = error.line_number.filter(|line| *line > 0) {
        details.push(format!("line {line}"));
    }
    format!(
        "SQL Server error {}: {} [{}]",
        error.number,
        error.message.trim(),
        details.join("; ")
    )
}

fn server_error_category(number: u32) -> &'static str {
    match number {
        1205 => "deadlock victim",
        18452 | 18456 | 4060 => "authentication failure",
        229 | 230 | 262 | 300 | 916 => "permission denial",
        515 | 547 | 1505 | 2601 | 2627 => "constraint violation",
        1222 => "timeout",
        102 | 105 | 156 => "syntax error",
        _ => "statement failure",
    }
}

fn format_tls_error(error: &TdsError, ssl_mode: Option<&str>) -> String {
    let ssl_mode = ssl_mode.unwrap_or("prefer");
    let advice = match ssl_mode {
        "verify-full" | "verify_identity" => {
            "install a certificate trusted by the system trust store, or try ssl_mode 'require' for a self-signed development server"
        }
        "disable" | "disabled" => {
            "try ssl_mode 'require' if the server requires encryption"
        }
        _ => {
            "check the server TLS configuration, or try ssl_mode 'disable' only on a trusted development network"
        }
    };
    format!(
        "SQL Server TLS negotiation failure with ssl_mode '{ssl_mode}': {error}. To recover, {advice}."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::DatabaseSelection;

    fn server_error(number: u32, line: i32) -> BridgeError {
        BridgeError::Tds(TdsError::from_sql_error(SqlErrorInfo {
            message: "fixture message".into(),
            state: 2,
            class: 16,
            number,
            server_name: Some("fixture-server".into()),
            proc_name: Some("fixture_proc".into()),
            line_number: Some(line),
        }))
    }

    #[test]
    fn structured_server_error_leads_with_number_text_and_keeps_details() {
        let message = format_bridge_error(&server_error(102, 7), None);
        assert!(message.starts_with("SQL Server error 102: fixture message"));
        assert!(message.contains("syntax error"));
        assert!(message.contains("severity 16"));
        assert!(message.contains("state 2"));
        assert!(message.contains("procedure 'fixture_proc'"));
        assert!(message.contains("line 7"));
    }

    #[test]
    fn server_error_categories_are_distinct() {
        for (number, category) in [
            (18456, "authentication failure"),
            (229, "permission denial"),
            (2627, "constraint violation"),
            (1205, "deadlock victim"),
            (1222, "timeout"),
            (102, "syntax error"),
        ] {
            assert!(
                format_bridge_error(&server_error(number, 1), None).contains(category),
                "error {number}"
            );
        }
    }

    #[test]
    fn tls_error_names_mode_and_action() {
        let error = BridgeError::Tds(TdsError::CertificateMismatch);
        let message = format_bridge_error(&error, Some("verify-full"));
        assert!(message.contains("TLS negotiation failure"));
        assert!(message.contains("ssl_mode 'verify-full'"));
        assert!(message.contains("ssl_mode 'require'"));
    }

    #[test]
    fn redaction_removes_password_and_connection_string() {
        let connection_string = "Server=db;User Id=sa;Password=NeverExposeThis!;Encrypt=true";
        let params = ConnectionParams {
            password: Some("NeverExposeThis!".into()),
            connection_string: Some(connection_string.into()),
            database: DatabaseSelection::Single("master".into()),
            ..Default::default()
        };
        let message = redact_connection_secrets(
            format!("failed with NeverExposeThis! from {connection_string}"),
            &params,
        );
        assert!(!message.contains("NeverExposeThis!"));
        assert!(!message.contains(connection_string));
        assert!(message.contains("<redacted>"));
    }
}
