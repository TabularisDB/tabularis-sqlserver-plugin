use super::*;
use crate::models::{ConnectionParams, DatabaseSelection};
use crate::settings::PluginSettings;

fn base_params(host: Option<&str>, port: Option<u16>, db: &str) -> ConnectionParams {
    ConnectionParams {
        driver: "sqlserver".into(),
        host: host.map(String::from),
        port,
        username: Some("sa".into()),
        password: Some("Strong!Pass123".into()),
        database: DatabaseSelection::Single(db.into()),
        ..Default::default()
    }
}

#[test]
fn build_config_uses_explicit_host_port() {
    let cfg = build_config(
        &base_params(Some("db.internal"), Some(1445), "master"),
        &PluginSettings::default(),
    )
    .expect("config builds");
    assert_eq!(cfg.get_addr(), "db.internal:1445");
}

#[test]
fn build_config_defaults_host_to_localhost() {
    let cfg = build_config(
        &base_params(None, Some(1433), "master"),
        &PluginSettings::default(),
    )
    .expect("config builds");
    assert_eq!(cfg.get_addr(), "localhost:1433");
}

#[test]
fn build_config_defaults_port_to_1433() {
    let cfg = build_config(
        &base_params(Some("localhost"), None, "master"),
        &PluginSettings::default(),
    )
    .expect("config builds");
    assert_eq!(cfg.get_addr(), "localhost:1433");
}

#[test]
fn build_config_empty_credentials_do_not_panic() {
    let mut params = base_params(Some("localhost"), Some(1433), "master");
    params.username = None;
    params.password = None;
    assert!(build_config(&params, &PluginSettings::default()).is_ok());
}

#[test]
fn manager_is_clone_send_sync() {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}
    fn assert_clone<T: Clone>() {}
    assert_send::<BridgeManager>();
    assert_sync::<BridgeManager>();
    assert_clone::<BridgeManager>();
}

#[test]
fn manager_new_stores_config() {
    let settings = PluginSettings::default();
    let cfg = build_config(
        &base_params(Some("example.com"), Some(1433), "master"),
        &settings,
    )
    .expect("config builds");
    let mgr = BridgeManager::new(cfg, Some("SET NOCOUNT ON".into()), "prefer", &settings);
    let cloned = mgr.clone();
    let original = format!("{:?}", mgr);
    let cloned_dbg = format!("{:?}", cloned);
    assert_eq!(original, cloned_dbg);
}

#[test]
fn build_config_applies_application_name_and_certificate_override() {
    let settings = PluginSettings {
        application_name: "Tabularis Test".into(),
        trust_server_certificate: true,
        ..PluginSettings::default()
    };
    let mut params = base_params(Some("localhost"), Some(1433), "master");
    params.ssl_mode = Some("verify-full".into());

    let debug = format!(
        "{:?}",
        build_config(&params, &settings).expect("config builds")
    );
    assert!(debug.contains("Tabularis Test"));
    assert!(debug.contains("trust_cert: true"));
}

#[test]
fn manager_snapshots_timeout_settings() {
    let settings = PluginSettings {
        connect_timeout_seconds: 8,
        query_timeout_seconds: 42,
        ..PluginSettings::default()
    };
    let cfg = build_config(
        &base_params(Some("localhost"), Some(1433), "master"),
        &settings,
    )
    .expect("config builds");
    let manager = BridgeManager::new(cfg, None, "prefer", &settings);

    assert_eq!(manager.connect_timeout, Duration::from_secs(8));
    assert_eq!(manager.query_timeout_seconds, Some(42));
}

#[test]
fn build_config_accepts_supported_tls_modes() {
    for mode in [
        "disable",
        "disabled",
        "prefer",
        "preferred",
        "require",
        "required",
        "verify-full",
        "verify_identity",
    ] {
        let mut params = base_params(Some("localhost"), Some(1433), "master");
        params.ssl_mode = Some(mode.into());
        let cfg = build_config(&params, &PluginSettings::default()).expect("config builds");
        assert_eq!(cfg.get_addr(), "localhost:1433");
    }
}

#[test]
fn build_config_rejects_unsupported_tls_inputs() {
    let mut verify_ca = base_params(Some("localhost"), Some(1433), "master");
    verify_ca.ssl_mode = Some("verify_ca".into());
    assert!(build_config(&verify_ca, &PluginSettings::default())
        .unwrap_err()
        .contains("verify-ca"));

    let mut custom_ca = base_params(Some("localhost"), Some(1433), "master");
    custom_ca.ssl_ca = Some("/tmp/ca.pem".into());
    assert!(build_config(&custom_ca, &PluginSettings::default())
        .unwrap_err()
        .contains("custom CA"));

    let mut client_cert = base_params(Some("localhost"), Some(1433), "master");
    client_cert.ssl_cert = Some("/tmp/client.pem".into());
    assert!(build_config(&client_cert, &PluginSettings::default())
        .unwrap_err()
        .contains("client certificates"));
}
