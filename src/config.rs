#![forbid(unsafe_code)]

use std::fmt;

use crate::model::{Config, Protocol};

#[derive(Debug)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config error: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> ConfigError {
        ConfigError(e.to_string())
    }
}

/// # Errors
///
/// Will return `Err` if the config is invalid or cannot be parsed or validated.
pub fn load_config(input: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(input)?;
    validate(&config)?;
    Ok(config)
}

fn validate(config: &Config) -> Result<(), ConfigError> {
    for listener in &config.listeners {
        match listener.protocol {
            Protocol::Https if !(listener.cert.is_some() && listener.key.is_some()) => {
                return Err(ConfigError("https requires both cert and key".to_string()))
            }
            Protocol::Http if listener.cert.is_some() || listener.key.is_some() => {
                return Err(ConfigError(
                    "cert/key are only meaningful for https".to_string(),
                ))
            }
            _ => {}
        }

        let sites = &listener.sites;

        if sites.is_empty() {
            return Err(ConfigError(
                "no sites; a listener that can only close connections is refused".to_string(),
            ));
        }

        for site in sites {
            match (site.root.is_some(), site.upstream.is_some()) {
                (true, true) => {
                    return Err(ConfigError(format!(
                        "listener {}: site {} sets both root and upstream",
                        listener.bind, site.hostname
                    )))
                }
                (false, false) => {
                    return Err(ConfigError(format!(
                        "listener {}: site {} sets neither root nor upstream",
                        listener.bind, site.hostname
                    )))
                }
                _ => {}
            }
        }

        let dupes = sites
            .iter()
            .map(|s| s.hostname.to_ascii_lowercase())
            .collect::<std::collections::HashSet<_>>()
            .len()
            != sites.len();
        if dupes {
            return Err(ConfigError(format!(
                "duplicate hostnames in listener {}",
                listener.bind
            )));
        }
    }

    let limits = config.limits;

    if limits.idle_timeout_secs == 0
        || limits.max_body_bytes == 0
        || limits.max_connections == 0
        || limits.max_header_bytes == 0
    {
        return Err(ConfigError(
            "all limits must be positive (INV-1)".to_string(),
        ));
    }

    if config.log.is_some() {
        return Err(ConfigError(
            "log section is not implemented yet".to_string(),
        ));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{build_routing, route};

    // M1-0 / INV-2: empty config is VALID and yields zero listeners.
    #[test]
    fn empty_config_means_nothing_bound() {
        let cfg = load_config("").unwrap();
        assert!(cfg.listeners.is_empty());
        let routing = build_routing(&cfg.listeners);
        assert!(route(&routing, "0.0.0.0:80", "example.com").is_none());
    }

    #[test]
    fn unknown_config_key_is_startup_error() {
        let err = load_config("[[listeners]]\nbind = \"x\"\nprotocal = \"http\"").unwrap_err();
        assert!(
            format!("{err}").contains("protocal"),
            "error must name the offending key"
        );
    }

    #[test]
    fn duplicate_hostname_is_startup_error() {
        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/a"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/b"
        "#;
        assert!(load_config(src).is_err());
    }

    #[test]
    fn refuse_closed_connections_config() {
        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
        "#;
        assert!(load_config(src).is_err());
    }

    #[test]
    fn https_requires_cert_and_key() {
        let src = "[[listeners]]\nbind = \"0.0.0.0:443\"\nprotocol = \"https\"\n";
        assert!(load_config(src).is_err());
    }

    #[test]
    fn https_with_cert_and_key() {
        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:443"
            protocol = "https"
            cert = "/etc/sich/tls/a.crt"
            key = "/etc/sich/tls/a.key"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/example.com"
        "#;
        assert!(load_config(src).is_ok());

        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:443"
            protocol = "https"
            cert = "/etc/sich/tls/a.crt"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/example.com"
        "#;
        assert!(load_config(src).is_err());

        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:443"
            protocol = "https"
            key = "/etc/sich/tls/a.key"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/example.com"
        "#;
        assert!(load_config(src).is_err());

        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            cert = "/etc/sich/tls/a.crt"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/example.com"
        "#;
        assert!(load_config(src).is_err());

        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            key = "/etc/sich/tls/a.key"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/example.com"
        "#;
        assert!(load_config(src).is_err());
    }

    #[test]
    fn root_and_upstream_specified() {
        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/example.com"
            upstream = localhost:3000"
        "#;
        assert!(load_config(src).is_err());
    }

    #[test]
    fn root_and_or_upstream() {
        // when root specified the upstream should be None
        // or when upstream specified the root should be None
        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/example.com"
            upstream = "localhost:3000"
        "#;
        assert!(load_config(src).is_err());

        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            [[listeners.sites]]
            hostname = "example.com"
            root = "/var/www/example.com"
        "#;
        assert!(load_config(src).is_ok());

        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            [[listeners.sites]]
            hostname = "example.com"
            upstream = "localhost:3000"
        "#;
        assert!(load_config(src).is_ok());

        // the root or upstream should be specified, if both are None, it should be an error
        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            [[listeners.sites]]
            hostname = "example.com"
        "#;
        assert!(load_config(src).is_err());
    }

    #[test]
    fn limits_are_positive() {
        let src = r#"
            [limits]
            max_connections = 0
            max_header_bytes = 8192
            max_body_bytes = 1048576
            idle_timeout_secs = 30
        "#;
        let result = load_config(src);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "config error: all limits must be positive (INV-1)"
        );

        let src = r#"
            [limits]
            max_connections = 512
            max_header_bytes = 0
            max_body_bytes = 1048576
            idle_timeout_secs = 30
        "#;
        let result = load_config(src);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "config error: all limits must be positive (INV-1)"
        );

        let src = r#"
            [limits]
            max_connections = 512
            max_header_bytes = 8192
            max_body_bytes = 0
            idle_timeout_secs = 30
        "#;
        let result = load_config(src);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "config error: all limits must be positive (INV-1)"
        );

        let src = r#"
            [limits]
            max_connections = 512
            max_header_bytes = 8192
            max_body_bytes = 1048576
            idle_timeout_secs = 0
        "#;
        let result = load_config(src);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "config error: all limits must be positive (INV-1)"
        );

        let src = r#"
            [limits]
            max_connections = 512
            max_header_bytes = 8192
            max_body_bytes = 1048576
            idle_timeout_secs = 30
        "#;
        assert!(load_config(src).is_ok());
    }
}
