//! SQL Server connection-string parsing and reconciliation.
//!
//! Tabularis may send discrete connection fields, a connection string, or a
//! mixture of both. Connection-string values are authoritative, while
//! discrete values fill fields omitted by the string. Supplying two different
//! values for the same field is rejected instead of choosing one silently.

use crate::models::{ConnectionParams, DatabaseSelection};

const CUSTOM_CA_ERROR: &str =
    "SQL Server custom CA files are not supported; use verify-full with the system trust store";

#[derive(Debug, Default, PartialEq)]
struct ParsedConnectionString {
    host: Option<String>,
    port: Option<u16>,
    username: Option<String>,
    password: Option<String>,
    database: Option<String>,
    ssl_mode: Option<String>,
    ssl_ca: Option<String>,
    ssl_cert: Option<String>,
    ssl_key: Option<String>,
    encrypt: Option<EncryptSetting>,
    trust_server_certificate: Option<bool>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EncryptSetting {
    Disabled,
    Enabled,
    Strict,
}

/// Return canonical connection fields suitable for both config construction
/// and pool-cache keying.
pub fn resolve_connection_params(params: &ConnectionParams) -> Result<ConnectionParams, String> {
    let mut resolved = params.clone();
    if resolved.driver.trim().is_empty() {
        resolved.driver = "sqlserver".into();
    }
    resolved.ssl_mode = non_empty(resolved.ssl_mode.take())
        .map(|mode| normalize_ssl_mode(&mode))
        .transpose()?;

    let Some(connection_string) = params
        .connection_string
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
    else {
        return Ok(resolved);
    };

    let parsed = ParsedConnectionString::parse(connection_string)?;
    reconcile_string(
        "host",
        &mut resolved.host,
        parsed.host,
        |left, right| left.eq_ignore_ascii_case(right),
        false,
    )?;
    reconcile_value("port", &mut resolved.port, parsed.port)?;
    reconcile_string(
        "username",
        &mut resolved.username,
        parsed.username,
        str::eq,
        false,
    )?;
    reconcile_string(
        "password",
        &mut resolved.password,
        parsed.password,
        str::eq,
        true,
    )?;

    if let Some(database) = parsed.database {
        let discrete = resolved.database.primary().trim();
        if !discrete.is_empty() && discrete != database {
            return Err(contradiction("database", discrete, &database, false));
        }
        resolved.database = DatabaseSelection::Single(database);
    }

    reconcile_string(
        "ssl_mode",
        &mut resolved.ssl_mode,
        parsed.ssl_mode,
        str::eq,
        false,
    )?;
    reconcile_string(
        "ssl_ca",
        &mut resolved.ssl_ca,
        parsed.ssl_ca,
        str::eq,
        false,
    )?;
    reconcile_string(
        "ssl_cert",
        &mut resolved.ssl_cert,
        parsed.ssl_cert,
        str::eq,
        false,
    )?;
    reconcile_string(
        "ssl_key",
        &mut resolved.ssl_key,
        parsed.ssl_key,
        str::eq,
        false,
    )?;

    Ok(resolved)
}

impl ParsedConnectionString {
    fn parse(input: &str) -> Result<Self, String> {
        let mut parsed = if starts_with_ignore_ascii_case(input, "sqlserver://") {
            Self::parse_url(input)?
        } else if input.contains("://") {
            return Err(
                "invalid SQL Server connection string: URL scheme must be sqlserver".to_string(),
            );
        } else {
            Self::parse_keywords(input)?
        };
        parsed.finish_tls()?;
        Ok(parsed)
    }

    fn parse_url(input: &str) -> Result<Self, String> {
        let rest = &input["sqlserver://".len()..];
        if rest.contains('#') {
            return Err(
                "invalid SQL Server URL connection string: fragments are not supported".into(),
            );
        }
        let (authority_and_path, query) = rest.split_once('?').unwrap_or((rest, ""));
        let (authority, path) = authority_and_path
            .split_once('/')
            .map_or((authority_and_path, None), |(authority, path)| {
                (authority, Some(path))
            });
        if authority.is_empty() {
            return Err("invalid SQL Server URL connection string: host is missing".into());
        }

        let mut parsed = Self::default();
        let host_port = if let Some((userinfo, host_port)) = authority.rsplit_once('@') {
            if userinfo.is_empty() {
                return Err("invalid SQL Server URL connection string: username is empty".into());
            }
            let (username, password) = userinfo
                .split_once(':')
                .map_or((userinfo, None), |(username, password)| {
                    (username, Some(password))
                });
            parsed.username = Some(percent_decode(username, false)?);
            if let Some(password) = password {
                parsed.password = Some(percent_decode(password, false).map_err(|_| {
                    "invalid percent encoding in SQL Server URL password".to_string()
                })?);
            }
            host_port
        } else {
            authority
        };
        let (host, port) = parse_url_host_port(host_port)?;
        parsed.host = Some(percent_decode(&host, false)?);
        parsed.port = port;

        if let Some(path) = path.filter(|path| !path.is_empty()) {
            if path.contains('/') {
                return Err(
                    "invalid SQL Server URL connection string: database must be one path segment"
                        .into(),
                );
            }
            parsed.database = Some(percent_decode(path, false)?);
        }

        if !query.is_empty() {
            for pair in query.split('&') {
                if pair.is_empty() {
                    continue;
                }
                let (key, value) = pair.split_once('=').ok_or_else(|| {
                    format!(
                        "invalid SQL Server URL connection string: query parameter '{pair}' has no value"
                    )
                })?;
                parsed.apply_keyword(&percent_decode(key, true)?, percent_decode(value, true)?)?;
            }
        }
        Ok(parsed)
    }

    fn parse_keywords(input: &str) -> Result<Self, String> {
        let pairs = parse_keyword_pairs(input)?;
        if pairs.is_empty() {
            return Err("invalid SQL Server keyword connection string: no key/value pairs".into());
        }
        let mut parsed = Self::default();
        for (key, value) in pairs {
            parsed.apply_keyword(&key, value)?;
        }
        Ok(parsed)
    }

    fn apply_keyword(&mut self, key: &str, value: String) -> Result<(), String> {
        let canonical = canonical_key(key);
        match canonical.as_str() {
            "server" | "datasource" | "address" | "addr" | "networkaddress" | "host" => {
                let (host, port) = parse_server_value(&value)?;
                set_string(&mut self.host, host, "server")?;
                if let Some(port) = port {
                    set_value(&mut self.port, port, "port")?;
                }
            }
            "port" => {
                let port = parse_port(&value)?;
                set_value(&mut self.port, port, "port")?;
            }
            "database" | "initialcatalog" => {
                set_string(&mut self.database, value, "database")?;
            }
            "userid" | "uid" | "user" | "username" => {
                set_string(&mut self.username, value, "username")?;
            }
            "password" | "pwd" => set_sensitive_string(&mut self.password, value, "password")?,
            "encrypt" => {
                let encrypt = parse_encrypt(&value)?;
                set_value(&mut self.encrypt, encrypt, "Encrypt")?;
            }
            "trustservercertificate" => {
                let trust = parse_bool("TrustServerCertificate", &value)?;
                set_value(
                    &mut self.trust_server_certificate,
                    trust,
                    "TrustServerCertificate",
                )?;
            }
            "sslmode" => {
                let mode = normalize_ssl_mode(&value)?;
                set_string(&mut self.ssl_mode, mode, "ssl_mode")?;
            }
            "sslca" | "cafile" | "truststore" | "servercertificate" => {
                set_string(&mut self.ssl_ca, value, "ssl_ca")?;
            }
            "sslcert" | "clientcertificate" => {
                set_string(&mut self.ssl_cert, value, "ssl_cert")?;
            }
            "sslkey" | "clientkey" => set_string(&mut self.ssl_key, value, "ssl_key")?,
            "integratedsecurity" | "trustedconnection" => {
                if parse_bool(key, &value)? {
                    return Err(
                        "SQL Server Integrated Authentication is not supported; use User Id and Password"
                            .into(),
                    );
                }
            }
            "authentication" => {
                if !value.eq_ignore_ascii_case("SqlPassword")
                    && !value.eq_ignore_ascii_case("NotSpecified")
                {
                    return Err(format!(
                        "SQL Server authentication mode '{value}' is not supported; use SqlPassword"
                    ));
                }
            }
            // These common client-side options do not change the server,
            // credentials, database, or TLS identity represented by a pool.
            "driver"
            | "applicationname"
            | "connecttimeout"
            | "connectiontimeout"
            | "timeout"
            | "multipleactiveresultsets"
            | "marsconnection"
            | "persistsecurityinfo"
            | "pooling" => {}
            _ => {
                return Err(format!(
                    "unsupported SQL Server connection string keyword '{key}'"
                ));
            }
        }
        Ok(())
    }

    fn finish_tls(&mut self) -> Result<(), String> {
        let from_keywords = match (self.encrypt, self.trust_server_certificate) {
            (Some(EncryptSetting::Disabled), _) => Some("disable"),
            (Some(EncryptSetting::Enabled), Some(true)) => Some("require"),
            (Some(EncryptSetting::Enabled), _) => Some("verify-full"),
            (Some(EncryptSetting::Strict), Some(true)) => {
                return Err(
                    "invalid SQL Server TLS settings: Encrypt=Strict contradicts TrustServerCertificate=true"
                        .into(),
                );
            }
            (Some(EncryptSetting::Strict), _) => Some("verify-full"),
            (None, Some(true)) => Some("prefer"),
            (None, Some(false)) => Some("verify-full"),
            (None, None) => None,
        };
        if let Some(mode) = from_keywords {
            set_string(&mut self.ssl_mode, mode.to_string(), "ssl_mode")?;
        }
        Ok(())
    }
}

fn parse_keyword_pairs(input: &str) -> Result<Vec<(String, String)>, String> {
    let bytes = input.as_bytes();
    let mut index = 0;
    let mut pairs = Vec::new();

    while index < bytes.len() {
        while index < bytes.len() && (bytes[index] == b';' || bytes[index].is_ascii_whitespace()) {
            index += 1;
        }
        if index == bytes.len() {
            break;
        }

        let key_start = index;
        while index < bytes.len() && bytes[index] != b'=' && bytes[index] != b';' {
            index += 1;
        }
        if index == bytes.len() || bytes[index] != b'=' {
            let segment = input[key_start..index].trim();
            return Err(format!(
                "invalid SQL Server keyword connection string: '{segment}' has no '='"
            ));
        }
        let key = input[key_start..index].trim();
        if key.is_empty() {
            return Err("invalid SQL Server keyword connection string: empty keyword".into());
        }
        index += 1;
        while index < bytes.len() && bytes[index].is_ascii_whitespace() {
            index += 1;
        }

        let value = if index < bytes.len() && bytes[index] == b'{' {
            index += 1;
            let mut value = String::new();
            let mut closed = false;
            while index < bytes.len() {
                if bytes[index] == b'}' {
                    if index + 1 < bytes.len() && bytes[index + 1] == b'}' {
                        value.push('}');
                        index += 2;
                    } else {
                        index += 1;
                        closed = true;
                        break;
                    }
                } else {
                    let character = input[index..]
                        .chars()
                        .next()
                        .expect("index is within the input");
                    value.push(character);
                    index += character.len_utf8();
                }
            }
            if !closed {
                return Err(format!(
                    "invalid SQL Server keyword connection string: unclosed braced value for '{key}'"
                ));
            }
            while index < bytes.len() && bytes[index].is_ascii_whitespace() {
                index += 1;
            }
            if index < bytes.len() && bytes[index] != b';' {
                return Err(format!(
                    "invalid SQL Server keyword connection string: unexpected text after braced value for '{key}'"
                ));
            }
            value
        } else {
            let value_start = index;
            while index < bytes.len() && bytes[index] != b';' {
                index += 1;
            }
            input[value_start..index].trim().to_string()
        };
        pairs.push((key.to_string(), value));
        if index < bytes.len() {
            index += 1;
        }
    }

    Ok(pairs)
}

fn parse_url_host_port(value: &str) -> Result<(String, Option<u16>), String> {
    if let Some(rest) = value.strip_prefix('[') {
        let closing = rest.find(']').ok_or_else(|| {
            "invalid SQL Server URL connection string: unclosed IPv6 host".to_string()
        })?;
        let host = &rest[..closing];
        if host.is_empty() {
            return Err("invalid SQL Server URL connection string: host is empty".into());
        }
        let suffix = &rest[closing + 1..];
        let port = if suffix.is_empty() {
            None
        } else {
            let raw = suffix.strip_prefix(':').ok_or_else(|| {
                "invalid SQL Server URL connection string: unexpected text after host".to_string()
            })?;
            Some(parse_port(raw)?)
        };
        return Ok((host.to_string(), port));
    }

    if value.matches(':').count() > 1 {
        return Err(
            "invalid SQL Server URL connection string: IPv6 hosts must use brackets".into(),
        );
    }
    let (host, port) = value
        .rsplit_once(':')
        .map_or((value, None), |(host, port)| (host, Some(port)));
    if host.is_empty() {
        return Err("invalid SQL Server URL connection string: host is empty".into());
    }
    Ok((host.to_string(), port.map(parse_port).transpose()?))
}

fn parse_server_value(value: &str) -> Result<(String, Option<u16>), String> {
    let value = value.trim();
    let value = if starts_with_ignore_ascii_case(value, "tcp:") {
        &value[4..]
    } else {
        value
    };
    if value.is_empty() {
        return Err("invalid SQL Server connection string: Server is empty".into());
    }
    if value.contains('\\') {
        return Err(
            "SQL Server named instances are not supported; specify Server=host,port".into(),
        );
    }
    let (host, port) = value
        .rsplit_once(',')
        .map_or((value, None), |(host, port)| (host.trim(), Some(port)));
    if host.is_empty() {
        return Err("invalid SQL Server connection string: Server host is empty".into());
    }
    Ok((host.to_string(), port.map(parse_port).transpose()?))
}

fn parse_port(value: &str) -> Result<u16, String> {
    value.trim().parse::<u16>().map_err(|_| {
        format!(
            "invalid SQL Server connection string port '{}'",
            value.trim()
        )
    })
}

fn parse_encrypt(value: &str) -> Result<EncryptSetting, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" | "mandatory" => Ok(EncryptSetting::Enabled),
        "false" | "no" | "off" | "0" | "optional" => Ok(EncryptSetting::Disabled),
        "strict" => Ok(EncryptSetting::Strict),
        _ => Err(format!(
            "invalid Encrypt value '{value}'; expected true, false, optional, mandatory, or strict"
        )),
    }
}

fn parse_bool(key: &str, value: &str) -> Result<bool, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "yes" | "on" | "1" => Ok(true),
        "false" | "no" | "off" | "0" => Ok(false),
        _ => Err(format!(
            "invalid {key} value '{value}'; expected true or false"
        )),
    }
}

fn normalize_ssl_mode(value: &str) -> Result<String, String> {
    match value.trim().to_ascii_lowercase().as_str() {
        "disable" | "disabled" => Ok("disable".into()),
        "prefer" | "preferred" => Ok("prefer".into()),
        "require" | "required" => Ok("require".into()),
        "verify-full" | "verify_identity" => Ok("verify-full".into()),
        "verify-ca" | "verify_ca" => Ok("verify-ca".into()),
        _ => Err(format!("unsupported SQL Server ssl_mode '{value}'")),
    }
}

fn percent_decode(input: &str, plus_as_space: bool) -> Result<String, String> {
    let bytes = input.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        match bytes[index] {
            b'%' => {
                if index + 2 >= bytes.len() {
                    return Err(format!(
                        "invalid percent encoding in SQL Server URL component '{input}'"
                    ));
                }
                let high = hex_value(bytes[index + 1]);
                let low = hex_value(bytes[index + 2]);
                let (Some(high), Some(low)) = (high, low) else {
                    return Err(format!(
                        "invalid percent encoding in SQL Server URL component '{input}'"
                    ));
                };
                decoded.push((high << 4) | low);
                index += 3;
            }
            b'+' if plus_as_space => {
                decoded.push(b' ');
                index += 1;
            }
            value => {
                decoded.push(value);
                index += 1;
            }
        }
    }
    String::from_utf8(decoded)
        .map_err(|_| "SQL Server URL contains percent-encoded non-UTF-8 data".to_string())
}

fn hex_value(value: u8) -> Option<u8> {
    match value {
        b'0'..=b'9' => Some(value - b'0'),
        b'a'..=b'f' => Some(value - b'a' + 10),
        b'A'..=b'F' => Some(value - b'A' + 10),
        _ => None,
    }
}

fn canonical_key(key: &str) -> String {
    key.chars()
        .filter(|character| !character.is_ascii_whitespace() && !matches!(character, '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    value
        .get(..prefix.len())
        .is_some_and(|start| start.eq_ignore_ascii_case(prefix))
}

fn set_string(slot: &mut Option<String>, value: String, field: &str) -> Result<(), String> {
    if let Some(existing) = slot {
        if existing != &value {
            return Err(format!(
                "SQL Server connection string specifies conflicting {field} values '{existing}' and '{value}'"
            ));
        }
    } else {
        *slot = Some(value);
    }
    Ok(())
}

fn set_sensitive_string(
    slot: &mut Option<String>,
    value: String,
    field: &str,
) -> Result<(), String> {
    if slot.as_ref().is_some_and(|existing| existing != &value) {
        return Err(format!(
            "SQL Server connection string specifies conflicting {field} values '<redacted>' and '<redacted>'"
        ));
    }
    if slot.is_none() {
        *slot = Some(value);
    }
    Ok(())
}

fn set_value<T>(slot: &mut Option<T>, value: T, field: &str) -> Result<(), String>
where
    T: Copy + PartialEq + std::fmt::Debug,
{
    if let Some(existing) = slot {
        if existing != &value {
            return Err(format!(
                "SQL Server connection string specifies conflicting {field} values '{existing:?}' and '{value:?}'"
            ));
        }
    } else {
        *slot = Some(value);
    }
    Ok(())
}

fn reconcile_string(
    field: &str,
    discrete: &mut Option<String>,
    from_string: Option<String>,
    equals: impl Fn(&str, &str) -> bool,
    sensitive: bool,
) -> Result<(), String> {
    let Some(from_string) = from_string else {
        *discrete = non_empty(discrete.take());
        return Ok(());
    };
    if let Some(value) = discrete.as_deref().map(str::trim).filter(|v| !v.is_empty()) {
        if !equals(value, &from_string) {
            return Err(contradiction(field, value, &from_string, sensitive));
        }
    }
    *discrete = Some(from_string);
    Ok(())
}

fn reconcile_value<T>(
    field: &str,
    discrete: &mut Option<T>,
    from_string: Option<T>,
) -> Result<(), String>
where
    T: Copy + PartialEq + std::fmt::Display,
{
    if let Some(from_string) = from_string {
        if let Some(discrete) = discrete {
            if *discrete != from_string {
                return Err(contradiction(
                    field,
                    &discrete.to_string(),
                    &from_string.to_string(),
                    false,
                ));
            }
        }
        *discrete = Some(from_string);
    }
    Ok(())
}

fn contradiction(field: &str, discrete: &str, from_string: &str, sensitive: bool) -> String {
    let (discrete, from_string) = if sensitive {
        ("<redacted>", "<redacted>")
    } else {
        (discrete, from_string)
    };
    format!(
        "connection parameter '{field}' contradicts the connection string: discrete value '{discrete}', connection-string value '{from_string}'"
    )
}

fn non_empty(value: Option<String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty())
}

pub fn custom_ca_error() -> &'static str {
    CUSTOM_CA_ERROR
}

#[cfg(test)]
mod tests {
    use super::*;

    fn params(connection_string: &str) -> ConnectionParams {
        ConnectionParams {
            connection_string: Some(connection_string.into()),
            ..Default::default()
        }
    }

    #[test]
    fn parses_url_with_percent_encoded_credentials_and_tls_query() {
        let resolved = resolve_connection_params(&params(
            "sqlserver://user:p%40ss%3Aword@db.example:1444/catalog%20name?Encrypt=true&TrustServerCertificate=true",
        ))
        .unwrap();

        assert_eq!(resolved.driver, "sqlserver");
        assert_eq!(resolved.host.as_deref(), Some("db.example"));
        assert_eq!(resolved.port, Some(1444));
        assert_eq!(resolved.username.as_deref(), Some("user"));
        assert_eq!(resolved.password.as_deref(), Some("p@ss:word"));
        assert_eq!(resolved.database.primary(), "catalog name");
        assert_eq!(resolved.ssl_mode.as_deref(), Some("require"));
    }

    #[test]
    fn url_allows_omitted_port_and_database_and_uses_discrete_fallbacks() {
        let mut input = params("sqlserver://url-user:url-password@db.example");
        input.port = Some(1433);
        input.database = DatabaseSelection::Single("fallback_db".into());
        input.ssl_mode = Some("required".into());

        let resolved = resolve_connection_params(&input).unwrap();
        assert_eq!(resolved.host.as_deref(), Some("db.example"));
        assert_eq!(resolved.port, Some(1433));
        assert_eq!(resolved.database.primary(), "fallback_db");
        assert_eq!(resolved.ssl_mode.as_deref(), Some("require"));
    }

    #[test]
    fn parses_case_insensitive_keyword_aliases_and_braced_semicolon() {
        let resolved = resolve_connection_params(&params(
            "Data Source=tcp:db.example,1444;Initial Catalog=app;UID=sa;PWD={p;a}}ss};Encrypt=YES;Trust Server Certificate=TRUE;",
        ))
        .unwrap();

        assert_eq!(resolved.host.as_deref(), Some("db.example"));
        assert_eq!(resolved.port, Some(1444));
        assert_eq!(resolved.database.primary(), "app");
        assert_eq!(resolved.username.as_deref(), Some("sa"));
        assert_eq!(resolved.password.as_deref(), Some("p;a}ss"));
        assert_eq!(resolved.ssl_mode.as_deref(), Some("require"));
    }

    #[test]
    fn tls_keywords_map_to_canonical_ssl_modes() {
        let cases = [
            ("Encrypt=false", "disable"),
            ("Encrypt=true;TrustServerCertificate=true", "require"),
            ("Encrypt=true;TrustServerCertificate=false", "verify-full"),
            ("Encrypt=strict", "verify-full"),
            ("TrustServerCertificate=true", "prefer"),
        ];
        for (connection_string, expected) in cases {
            let mut input = params(connection_string);
            input.host = Some("localhost".into());
            let resolved = resolve_connection_params(&input).unwrap();
            assert_eq!(resolved.ssl_mode.as_deref(), Some(expected));
        }
    }

    #[test]
    fn equal_discrete_values_are_accepted_and_missing_values_fill_in() {
        let mut input = params("Server=db.example,1433;Database=app;User Id=sa;Password=secret");
        input.host = Some("DB.EXAMPLE".into());
        input.port = Some(1433);
        input.username = Some("sa".into());
        input.password = Some("secret".into());
        input.ssl_mode = Some("require".into());

        let resolved = resolve_connection_params(&input).unwrap();
        assert_eq!(resolved.database.primary(), "app");
        assert_eq!(resolved.ssl_mode.as_deref(), Some("require"));
    }

    #[test]
    fn contradictory_values_name_both_sources() {
        let mut input = params("Server=from-string;Database=app");
        input.host = Some("from-discrete".into());

        let error = resolve_connection_params(&input).unwrap_err();
        assert!(error.contains("host"));
        assert!(error.contains("from-discrete"));
        assert!(error.contains("from-string"));
    }

    #[test]
    fn password_parse_errors_never_echo_credentials() {
        for (connection_string, secret) in [
            (
                "Server=localhost;Password=FirstSecret!;Password=SecondSecret!",
                "FirstSecret!",
            ),
            (
                "sqlserver://sa:Malformed%ZZSecret@localhost/master",
                "Malformed",
            ),
        ] {
            let error = resolve_connection_params(&params(connection_string)).unwrap_err();
            assert!(!error.contains(secret), "{error}");
            assert!(!error.contains(connection_string), "{error}");
        }
    }

    #[test]
    fn malformed_connection_strings_are_rejected() {
        for connection_string in [
            "not-a-connection-string",
            "postgres://sa:secret@localhost/master",
            "sqlserver://sa:bad%ZZ@localhost/master",
            "sqlserver://sa:secret@localhost:not-a-port/master",
            "Server=localhost;Password={unclosed",
            "Server=localhost;Encrypt=perhaps",
            "Server=localhost;UnknownSetting=true",
        ] {
            assert!(
                resolve_connection_params(&params(connection_string)).is_err(),
                "expected malformed input to fail: {connection_string}"
            );
        }
    }

    #[test]
    fn custom_ca_keyword_is_preserved_for_the_config_rejection_path() {
        let resolved =
            resolve_connection_params(&params("Server=localhost;SslCa=/tmp/custom-ca.pem"))
                .unwrap();
        assert_eq!(resolved.ssl_ca.as_deref(), Some("/tmp/custom-ca.pem"));
        assert_eq!(custom_ca_error(), CUSTOM_CA_ERROR);
    }
}
