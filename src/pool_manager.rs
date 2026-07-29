//! Connection-pool registry.
//!
//! Pools are cached per connection key so repeated RPC calls reuse warm
//! sessions instead of paying a fresh TDS handshake each time.

use std::collections::HashMap;
use std::sync::Arc;

use deadpool::managed::Pool as DeadPool;
use once_cell::sync::Lazy;
use tokio::sync::RwLock;

use crate::driver::pool::{build_config, BridgeManager};
use crate::models::ConnectionParams;

pub type SqlServerPool = DeadPool<BridgeManager>;
type SqlServerPoolMap = Arc<RwLock<HashMap<String, SqlServerPool>>>;

static SQLSERVER_POOLS: Lazy<SqlServerPoolMap> =
    Lazy::new(|| Arc::new(RwLock::new(HashMap::new())));

/// Stable cache key for a set of connection params.
///
/// Prefers the host-assigned `connection_id`; falls back to
/// host:port:user:database for ad-hoc connections. The username is essential:
/// bastions multiplex many targets behind a single host:port and pick the
/// backend from the username, so without it two different targets would share
/// one pool. TLS settings are folded in so switching `ssl_mode` never reuses
/// a pool built under a different policy.
fn build_connection_key(params: &ConnectionParams) -> String {
    let ssl_mode = params.ssl_mode.as_deref().unwrap_or("prefer");
    let base_key = if let Some(conn_id) = params.connection_id.as_deref() {
        format!("{}:conn:{}:{}", params.driver, conn_id, params.database)
    } else {
        format!(
            "{}:{}:{}:{}:{}",
            params.driver,
            params.host.as_deref().unwrap_or("localhost"),
            params.port.unwrap_or(0),
            params.username.as_deref().unwrap_or(""),
            params.database
        )
    };
    format!("{base_key}:ssl:{ssl_mode}")
}

fn startup_script(params: &ConnectionParams) -> Option<String> {
    params
        .startup_script
        .as_ref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
}

pub async fn get_sqlserver_pool(params: &ConnectionParams) -> Result<SqlServerPool, String> {
    let key = build_connection_key(params);
    let mut pools = SQLSERVER_POOLS.write().await;
    if let Some(pool) = pools.get(&key).cloned() {
        return Ok(pool);
    }

    let manager = BridgeManager::new(build_config(params)?, startup_script(params));
    let pool = DeadPool::builder(manager)
        .max_size(10)
        .build()
        .map_err(|error| error.to_string())?;
    pools.insert(key, pool.clone());
    Ok(pool)
}

/// Drop pools that currently have no checked-out connections. Called
/// periodically so long-idle sessions don't linger for the plugin's lifetime.
pub async fn cleanup_idle_pools() {
    let mut pools = SQLSERVER_POOLS.write().await;
    pools.retain(|_, pool| pool.status().size > pool.status().available);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(connection_id: Option<&str>) -> ConnectionParams {
        ConnectionParams {
            driver: "sqlserver".into(),
            host: Some("localhost".into()),
            port: Some(1433),
            username: Some("sa".into()),
            database: crate::models::DatabaseSelection::Single("master".into()),
            connection_id: connection_id.map(str::to_owned),
            ..Default::default()
        }
    }

    #[test]
    fn key_prefers_connection_id() {
        let key = build_connection_key(&params(Some("abc")));
        assert!(key.contains(":conn:abc:"));
    }

    #[test]
    fn key_includes_user_and_ssl_mode_for_adhoc_connections() {
        let mut p = params(None);
        p.ssl_mode = Some("require".into());
        let key = build_connection_key(&p);
        assert_eq!(key, "sqlserver:localhost:1433:sa:master:ssl:require");
    }
}
