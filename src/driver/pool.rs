//! mssql-tds connection pool primitives.
//!
//! Pools `mssql_tiberius_bridge::Client` objects (Microsoft's `mssql-tds`
//! protocol implementation behind a tiberius-compatible API) via a custom
//! deadpool manager.
//!
//! Current authentication support is SQL Server username/password. TLS uses
//! Tabularis' shared `ssl_mode`: `disable` turns encryption off,
//! `verify-full` requires the system trust store and hostname verification,
//! `require` encrypts while accepting the server certificate, and `prefer`
//! requests encrypted local-development-compatible connections.

use std::future::Future;
use std::time::Duration;

use crate::connection::{custom_ca_error, resolve_connection_params};
use crate::models::ConnectionParams;
use crate::settings::PluginSettings;
use deadpool::managed::{Manager, Metrics, RecycleError, RecycleResult};
use mssql_tiberius_bridge::TdsClient;
use mssql_tiberius_bridge::{
    AuthMethod, Client, Config, EncryptionLevel, Error, ExecuteResult, QueryResult, ToSql,
};
use tokio::time::timeout;

/// A live bridge client with the query timeout snapshotted by its pool.
pub struct BridgeConnection {
    client: Client,
    query_timeout_seconds: Option<u32>,
}

impl BridgeConnection {
    async fn with_query_timeout<T>(
        seconds: Option<u32>,
        operation: impl Future<Output = Result<T, Error>>,
    ) -> Result<T, Error> {
        match seconds {
            Some(seconds) => timeout(Duration::from_secs(u64::from(seconds)), operation)
                .await
                .map_err(|_| {
                    Error::Conversion(format!("Query timed out after {seconds} seconds"))
                })?,
            None => operation.await,
        }
    }

    pub async fn simple_query(&mut self, sql: impl Into<String>) -> Result<QueryResult, Error> {
        let operation = self.client.simple_query(sql.into());
        Self::with_query_timeout(self.query_timeout_seconds, operation).await
    }

    pub async fn query(
        &mut self,
        sql: impl Into<String>,
        params: &[&dyn ToSql],
    ) -> Result<QueryResult, Error> {
        let operation = self.client.query(sql.into(), params);
        Self::with_query_timeout(self.query_timeout_seconds, operation).await
    }

    pub async fn execute(
        &mut self,
        sql: impl Into<String>,
        params: &[&dyn ToSql],
    ) -> Result<ExecuteResult, Error> {
        let operation = self.client.execute(sql.into(), params);
        Self::with_query_timeout(self.query_timeout_seconds, operation).await
    }

    pub fn inner_mut(&mut self) -> &mut TdsClient {
        self.client.inner_mut()
    }

    pub fn query_timeout_seconds(&self) -> Option<u32> {
        self.query_timeout_seconds
    }
}

/// Deadpool `Manager` for bridge connections.
#[derive(Debug, Clone)]
pub struct BridgeManager {
    config: Config,
    startup_script: Option<String>,
    connect_timeout: Duration,
    query_timeout_seconds: Option<u32>,
}

impl BridgeManager {
    pub fn new(config: Config, startup_script: Option<String>, settings: &PluginSettings) -> Self {
        Self {
            config,
            startup_script,
            connect_timeout: Duration::from_secs(u64::from(settings.connect_timeout_seconds)),
            query_timeout_seconds: settings
                .query_timeout()
                .map(|_| settings.query_timeout_seconds),
        }
    }

    async fn apply_startup_script(&self, conn: &mut BridgeConnection) -> Result<(), Error> {
        if let Some(script) = self.startup_script.as_deref() {
            conn.simple_query(script)
                .await
                .map_err(startup_script_error)?
                .into_results();
        }
        Ok(())
    }
}

fn startup_script_error(error: Error) -> Error {
    Error::Conversion(format!("Startup script failed: {error}"))
}

impl Manager for BridgeManager {
    type Type = BridgeConnection;

    type Error = Error;

    async fn create(&self) -> Result<Self::Type, Self::Error> {
        let client = timeout(self.connect_timeout, Client::connect(&self.config))
            .await
            .map_err(|_| {
                Error::Conversion(format!(
                    "Connection timed out after {} seconds",
                    self.connect_timeout.as_secs()
                ))
            })??;
        let mut connection = BridgeConnection {
            client,
            query_timeout_seconds: self.query_timeout_seconds,
        };
        self.apply_startup_script(&mut connection).await?;
        Ok(connection)
    }

    async fn recycle(&self, conn: &mut Self::Type, _: &Metrics) -> RecycleResult<Self::Error> {
        // Reset transaction, temporary-object, and SET state before another
        // caller receives this physical session, then restore its configured
        // startup script.
        conn.simple_query("EXEC sp_reset_connection")
            .await
            .map_err(RecycleError::Backend)?
            .into_results();
        self.apply_startup_script(conn)
            .await
            .map_err(RecycleError::Backend)?;
        Ok(())
    }
}

/// Build a `mssql_tiberius_bridge::Config` from Tabularis `ConnectionParams`.
///
/// Consumes the shared connection fields used by current Tabularis drivers.
/// SQL Server authentication is currently username/password only. TLS maps
/// the standard `ssl_mode` values onto the bridge's encryption policy.
pub fn build_config(
    params: &ConnectionParams,
    settings: &PluginSettings,
) -> Result<Config, String> {
    let params = resolve_connection_params(params)?;
    let mut cfg = Config::new();
    cfg.host(params.host.as_deref().unwrap_or("localhost"));
    cfg.port(params.port.unwrap_or(1433));
    cfg.database(params.database.primary());
    cfg.authentication(AuthMethod::sql_server(
        params.username.as_deref().unwrap_or("sa"),
        params.password.as_deref().unwrap_or(""),
    ));
    cfg.application_name(&settings.application_name);

    if params
        .ssl_ca
        .as_deref()
        .is_some_and(|path| !path.is_empty())
    {
        return Err(custom_ca_error().into());
    }
    if params
        .ssl_cert
        .as_deref()
        .is_some_and(|path| !path.is_empty())
        || params
            .ssl_key
            .as_deref()
            .is_some_and(|path| !path.is_empty())
    {
        return Err("SQL Server client certificates are not supported".into());
    }

    match params.ssl_mode.as_deref() {
        Some("disable" | "disabled") => {
            cfg.encryption(EncryptionLevel::NotSupported);
        }
        Some("require" | "required") => {
            cfg.encryption(EncryptionLevel::Required);
            cfg.trust_cert();
        }
        Some("verify-full" | "verify_identity") => {
            cfg.encryption(EncryptionLevel::Required);
        }
        Some("verify-ca" | "verify_ca") => {
            return Err(
                "SQL Server verify-ca is not supported; use verify-full/verify_identity for certificate and hostname verification"
                    .into(),
            );
        }
        _ => {
            cfg.encryption(EncryptionLevel::On);
            cfg.trust_cert();
        }
    }

    // Explicitly permit self-signed certificates even with a verifying TLS
    // mode. `require` and `prefer` already trust certificates by definition.
    if settings.trust_server_certificate {
        cfg.trust_cert();
    }

    Ok(cfg)
}

#[cfg(test)]
mod tests;
