#![forbid(unsafe_code)]

use crate::model::{Listener, Routing, Site};

pub mod config;
pub mod http;
pub mod model;
pub mod path;

/// # Errors
///
/// Will return `Err` if the routing is invalid or cannot be parsed or validated.
#[must_use]
pub fn build_routing(listener: &[Listener]) -> Routing {
    let capacity = listener.iter().map(|l| l.sites.len()).sum();
    let mut routing = Routing {
        sites: Vec::with_capacity(capacity),
        routes: Vec::with_capacity(capacity),
    };

    for listener in listener {
        for site in &listener.sites {
            let idx = routing.sites.len();
            routing.sites.push(site.clone());
            routing.routes.push((listener.bind.clone(), idx));
        }
    }

    routing
}

#[must_use]
pub fn route<'a>(routing: &'a Routing, listener: &str, host: &str) -> Option<&'a Site> {
    routing
        .routes
        .iter()
        .find(|(l, idx)| l == listener && routing.sites[*idx].hostname.eq_ignore_ascii_case(host))
        .map(|(_, idx)| &routing.sites[*idx])
}

#[must_use]
pub fn host_header_of(headers: &[(String, String)]) -> Option<&str> {
    let mut found: Option<&str> = None;
    for (name, value) in headers {
        if name == "host" {
            if found.is_some() {
                return None;
            }
            found = Some(strip_port(value));
        }
    }
    found
}

fn strip_port(value: &str) -> &str {
    if let Some(rest) = value.strip_prefix('[') {
        return rest.find(']').map_or(value, |end| &rest[..end]);
    }
    match value.rsplit_once(':') {
        Some((host, port)) if !port.is_empty() && port.bytes().all(|b| b.is_ascii_digit()) => host,
        _ => value,
    }
}

/// Accept errors that end one connection but say nothing about the
/// listener: keep accepting. Everything else is fatal for the listener.
#[must_use]
pub fn is_transient_accept_error(kind: std::io::ErrorKind) -> bool {
    matches!(
        kind,
        std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::Interrupted
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::load_config;

    #[test]
    fn exact_host_match_routes() {
        let src = r#"
            [[listeners]]
            bind = "0.0.0.0:80"
            protocol = "http"
            [[listeners.sites]]
            hostname = "foo.com"
            root = "/var/www/foo"
            [[listeners.sites]]
            hostname = "bar.com"
            root = "/var/www/bar"
            [[listeners]]
            bind = "0.0.0.0:443"
            protocol = "https"
            cert = "/c"
            key = "/k"
            [[listeners.sites]]
            hostname = "secure.com"
            root = "/var/www/secure"
            "#;
        let cfg = load_config(src).unwrap();
        let routing = build_routing(&cfg.listeners);
        assert!(route(&routing, "0.0.0.0:80", "foo.com").is_some());
        assert_eq!(
            route(&routing, "0.0.0.0:80", "foo.com")
                .unwrap()
                .root
                .as_deref(),
            Some("/var/www/foo"),
        );
        assert!(route(&routing, "0.0.0.0:80", "bar.com").is_some());
        assert_eq!(
            route(&routing, "0.0.0.0:80", "bar.com")
                .unwrap()
                .root
                .as_deref(),
            Some("/var/www/bar"),
        );
        assert!(route(&routing, "0.0.0.0:80", "bar.example.com").is_none());
        assert_eq!(
            route(&routing, "0.0.0.0:443", "secure.com")
                .unwrap()
                .root
                .as_deref(),
            Some("/var/www/secure")
        );
    }

    #[test]
    fn test_host_header_of() {
        let headers = vec![
            ("host".to_string(), "example.com:8080".to_string()),
            ("user-agent".to_string(), "test-agent".to_string()),
        ];
        assert_eq!(host_header_of(&headers), Some("example.com"));

        let headers_with_port = vec![
            ("host".to_string(), "example.com:443".to_string()),
            ("user-agent".to_string(), "test-agent".to_string()),
        ];
        assert_eq!(host_header_of(&headers_with_port), Some("example.com"));

        let headers_without_port = vec![
            ("host".to_string(), "example.com".to_string()),
            ("user-agent".to_string(), "test-agent".to_string()),
        ];
        assert_eq!(host_header_of(&headers_without_port), Some("example.com"));

        let headers_with_ipv6 = vec![
            ("host".to_string(), "[2001:db8::1]:8080".to_string()),
            ("user-agent".to_string(), "test-agent".to_string()),
        ];
        assert_eq!(host_header_of(&headers_with_ipv6), Some("2001:db8::1"));

        let headers_with_multiple_hosts = vec![
            ("host".to_string(), "example.com:8080".to_string()),
            ("host".to_string(), "another.com:8080".to_string()),
        ];
        assert_eq!(host_header_of(&headers_with_multiple_hosts), None);
    }

    #[test]
    fn test_strip_port() {
        assert_eq!(strip_port("example.com:8080"), "example.com");
        assert_eq!(strip_port("example.com"), "example.com");
        assert_eq!(strip_port("[2001:db8::1]:8080"), "2001:db8::1");
        assert_eq!(strip_port("[2001:db8::1]"), "2001:db8::1");
        assert_eq!(strip_port("localhost:3000"), "localhost");
        assert_eq!(strip_port("localhost"), "localhost");
        assert_eq!(strip_port("example.com:abc"), "example.com:abc");
        assert_eq!(strip_port("example.com:"), "example.com:");
    }

    #[test]
    fn transient_accept_errors_do_not_kill_the_listener() {
        for kind in [
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::ConnectionReset,
            std::io::ErrorKind::Interrupted,
        ] {
            assert!(
                is_transient_accept_error(kind),
                "{kind:?} must be survivable"
            );
        }
    }

    #[test]
    fn fatal_accept_errors_propagate() {
        for kind in [
            std::io::ErrorKind::PermissionDenied,
            std::io::ErrorKind::InvalidInput,
            std::io::ErrorKind::OutOfMemory,
            std::io::ErrorKind::Other,
        ] {
            assert!(!is_transient_accept_error(kind), "{kind:?} must be fatal");
        }
    }
}
