//! Process-wide plugin settings received through the `initialize` RPC.
//!
//! The host initializes a plugin once, before sending connection requests.
//! Pools snapshot these values when they are created; updating this state does
//! not mutate live pooled sessions.

use std::sync::RwLock;
use std::time::Duration;

use once_cell::sync::Lazy;
use serde_json::{Map, Value};
use tokio::sync::watch;

pub const DEFAULT_MAX_POOL_SIZE: usize = 10;
pub const DEFAULT_CONNECT_TIMEOUT_SECONDS: u32 = 15;
pub const DEFAULT_QUERY_TIMEOUT_SECONDS: u32 = 0;
pub const DEFAULT_APPLICATION_NAME: &str = "Tabularis";
pub const DEFAULT_TRUST_SERVER_CERTIFICATE: bool = false;
pub const DEFAULT_POOL_IDLE_EVICTION_MINUTES: u32 = 10;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSettings {
    pub max_pool_size: usize,
    pub connect_timeout_seconds: u32,
    pub query_timeout_seconds: u32,
    pub application_name: String,
    pub trust_server_certificate: bool,
    pub pool_idle_eviction_minutes: u32,
}

impl Default for PluginSettings {
    fn default() -> Self {
        Self {
            max_pool_size: DEFAULT_MAX_POOL_SIZE,
            connect_timeout_seconds: DEFAULT_CONNECT_TIMEOUT_SECONDS,
            query_timeout_seconds: DEFAULT_QUERY_TIMEOUT_SECONDS,
            application_name: DEFAULT_APPLICATION_NAME.to_owned(),
            trust_server_certificate: DEFAULT_TRUST_SERVER_CERTIFICATE,
            pool_idle_eviction_minutes: DEFAULT_POOL_IDLE_EVICTION_MINUTES,
        }
    }
}

impl PluginSettings {
    fn from_initialize_params(params: &Value) -> Self {
        let defaults = Self::default();
        let Some(settings) = params.get("settings") else {
            return defaults;
        };
        let Some(settings) = settings.as_object() else {
            eprintln!("invalid plugin setting 'settings': expected an object, using all defaults");
            return defaults;
        };

        Self {
            max_pool_size: positive_usize(settings, "max_pool_size", defaults.max_pool_size),
            connect_timeout_seconds: positive_u32(
                settings,
                "connect_timeout_seconds",
                defaults.connect_timeout_seconds,
            ),
            query_timeout_seconds: nonnegative_u32(
                settings,
                "query_timeout_seconds",
                defaults.query_timeout_seconds,
            ),
            application_name: string_setting(
                settings,
                "application_name",
                &defaults.application_name,
            ),
            trust_server_certificate: boolean_setting(
                settings,
                "trust_server_certificate",
                defaults.trust_server_certificate,
            ),
            pool_idle_eviction_minutes: positive_u32(
                settings,
                "pool_idle_eviction_minutes",
                defaults.pool_idle_eviction_minutes,
            ),
        }
    }

    pub fn query_timeout(&self) -> Option<Duration> {
        (self.query_timeout_seconds > 0)
            .then(|| Duration::from_secs(u64::from(self.query_timeout_seconds)))
    }

    pub fn pool_idle_eviction_interval(&self) -> Duration {
        Duration::from_secs(u64::from(self.pool_idle_eviction_minutes) * 60)
    }
}

static SETTINGS: Lazy<RwLock<PluginSettings>> =
    Lazy::new(|| RwLock::new(PluginSettings::default()));
static SETTINGS_VERSION: Lazy<watch::Sender<u64>> = Lazy::new(|| {
    let (sender, _) = watch::channel(0);
    sender
});

/// Apply one forgiving `initialize` payload. Unknown keys are deliberately
/// ignored and each malformed known value falls back independently.
pub fn initialize(params: &Value) {
    let new_settings = PluginSettings::from_initialize_params(params);
    match SETTINGS.write() {
        Ok(mut settings) => *settings = new_settings,
        Err(poisoned) => *poisoned.into_inner() = new_settings,
    }
    SETTINGS_VERSION.send_modify(|version| *version = version.wrapping_add(1));
}

/// Return a snapshot suitable for a newly created pool.
pub fn current() -> PluginSettings {
    match SETTINGS.read() {
        Ok(settings) => settings.clone(),
        Err(poisoned) => poisoned.into_inner().clone(),
    }
}

/// Notify long-lived maintenance tasks when `initialize` changes settings.
pub fn subscribe() -> watch::Receiver<u64> {
    SETTINGS_VERSION.subscribe()
}

fn positive_usize(settings: &Map<String, Value>, key: &str, default: usize) -> usize {
    match settings.get(key) {
        None => default,
        Some(value) => match value.as_u64().and_then(|value| usize::try_from(value).ok()) {
            Some(value) if value > 0 => value,
            _ => {
                warn_fallback(key, value, default);
                default
            }
        },
    }
}

fn positive_u32(settings: &Map<String, Value>, key: &str, default: u32) -> u32 {
    match settings.get(key) {
        None => default,
        Some(value) => match value.as_u64().and_then(|value| u32::try_from(value).ok()) {
            Some(value) if value > 0 => value,
            _ => {
                warn_fallback(key, value, default);
                default
            }
        },
    }
}

fn nonnegative_u32(settings: &Map<String, Value>, key: &str, default: u32) -> u32 {
    match settings.get(key) {
        None => default,
        Some(value) => match value.as_u64().and_then(|value| u32::try_from(value).ok()) {
            Some(value) => value,
            None => {
                warn_fallback(key, value, default);
                default
            }
        },
    }
}

fn string_setting(settings: &Map<String, Value>, key: &str, default: &str) -> String {
    match settings.get(key) {
        None => default.to_owned(),
        Some(value) => match value.as_str() {
            Some(value) => value.to_owned(),
            None => {
                warn_fallback(key, value, default);
                default.to_owned()
            }
        },
    }
}

fn boolean_setting(settings: &Map<String, Value>, key: &str, default: bool) -> bool {
    match settings.get(key) {
        None => default,
        Some(value) => match value.as_bool() {
            Some(value) => value,
            None => {
                warn_fallback(key, value, default);
                default
            }
        },
    }
}

fn warn_fallback(key: &str, value: &Value, default: impl std::fmt::Display) {
    eprintln!("invalid plugin setting '{key}': got {value}, using default {default}");
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn defaults_apply_when_initialize_is_never_called() {
        assert_eq!(
            PluginSettings::default(),
            PluginSettings {
                max_pool_size: 10,
                connect_timeout_seconds: 15,
                query_timeout_seconds: 0,
                application_name: "Tabularis".into(),
                trust_server_certificate: false,
                pool_idle_eviction_minutes: 10,
            }
        );
    }

    #[test]
    fn empty_initialize_settings_use_defaults() {
        assert_eq!(
            PluginSettings::from_initialize_params(&json!({})),
            PluginSettings::default()
        );
        assert_eq!(
            PluginSettings::from_initialize_params(&json!({ "settings": {} })),
            PluginSettings::default()
        );
    }

    #[test]
    fn initialize_settings_override_every_default() {
        let parsed = PluginSettings::from_initialize_params(&json!({
            "settings": {
                "max_pool_size": 24,
                "connect_timeout_seconds": 7,
                "query_timeout_seconds": 90,
                "application_name": "Tabularis CI",
                "trust_server_certificate": true,
                "pool_idle_eviction_minutes": 3,
                "future_setting": "ignored"
            }
        }));

        assert_eq!(
            parsed,
            PluginSettings {
                max_pool_size: 24,
                connect_timeout_seconds: 7,
                query_timeout_seconds: 90,
                application_name: "Tabularis CI".into(),
                trust_server_certificate: true,
                pool_idle_eviction_minutes: 3,
            }
        );
    }

    #[test]
    fn malformed_initialize_values_fall_back_independently() {
        let parsed = PluginSettings::from_initialize_params(&json!({
            "settings": {
                "max_pool_size": 0,
                "connect_timeout_seconds": "soon",
                "query_timeout_seconds": -1,
                "application_name": false,
                "trust_server_certificate": "yes",
                "pool_idle_eviction_minutes": 1.5
            }
        }));

        assert_eq!(parsed, PluginSettings::default());
    }
}
