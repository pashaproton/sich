#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};

use crate::model::{Listener, PathError, Routing, Site};

pub mod config;
pub mod http;
pub mod model;

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

/// # Errors
///
/// Will return `Err` if path not found, traversal or outside root.
pub fn resolve_path(site: &Site, request_path: &str) -> Result<PathBuf, PathError> {
    let root = site.root.as_ref().ok_or(PathError::NotFound)?;

    let decoded = percent_decode(request_path)?;
    for segment in decoded.split('/') {
        if segment == ".." || segment.contains('\0') {
            return Err(PathError::Traversal);
        }
    }

    let candidate = Path::new(root).join(decoded.trim_start_matches('/'));
    let canonical = candidate.canonicalize().map_err(|_| PathError::NotFound)?;
    if !canonical.starts_with(Path::new(root)) {
        return Err(PathError::OutsideRoot);
    }
    Ok(canonical)
}

/// Decode a percent-encoded URL path into its raw bytes.
///
/// A URL path may arrive with characters written as `%XX`, where XX is
/// the two-digit hex value of the byte: `%2e` is `.`, `%2f` is `/`.
/// Path safety checks (refusal of `..`, NUL) MUST run on the decoded
/// form, otherwise `/../../etc/passwd` hides itself as `/%2e%2e/..`.
///
/// Fail closed: a malformed escape (`%ZZ`, a trailing `%` with fewer
/// than two bytes after it, or a result that is not valid UTF-8) is an
/// error, never a lenient pass-through. Exactly one decode is performed
/// — no iterative decoding — so no layered-encoding bypass exists.
fn percent_decode(input: &str) -> Result<String, PathError> {
    let mut out = Vec::with_capacity(input.len());
    let mut it = input.bytes();
    while let Some(b) = it.next() {
        if b == b'%' {
            let hi = it.next().and_then(hex_val).ok_or(PathError::Traversal)?;
            let lo = it.next().and_then(hex_val).ok_or(PathError::Traversal)?;
            out.push(hi * 16 + lo);
        } else {
            out.push(b);
        }
    }
    String::from_utf8(out).map_err(|_| PathError::Traversal)
}

/// Convert one ASCII hex digit ('0'-'9', 'a'-'f', 'A'-'F') to its
/// numeric value: 0..=15.
///
/// Returns None for any byte that is not a hex digit; the caller treats
/// that as a malformed escape and refuses the path (fail closed).
/// Two digits combine as `(high << 4) | low` — see `percent_decode`.
fn hex_val(b: u8) -> Option<u8> {
    match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    }
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
    fn test_resolve_path() {
        let site = Site {
            hostname: "example.com".into(),
            root: Some("/var/www".into()),
            upstream: None,
        };
        assert!(matches!(
            resolve_path(&site, "/../etc/passwd"),
            Err(PathError::Traversal)
        ));
        assert!(matches!(
            resolve_path(&site, "/%2e%2e/etc/passwd"),
            Err(PathError::Traversal)
        ));
        assert!(matches!(
            resolve_path(&site, "/foo"),
            Err(PathError::NotFound)
        ));
    }

    #[test]
    #[cfg(unix)]
    fn test_resolve_path_symlink_escape_is_outside_root() {
        let base = std::env::temp_dir().join(format!("sich-{}", std::process::id()));
        let root = base.join("root");
        let outside = base.join("outside");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        std::fs::write(outside.join("secret"), b"x").unwrap();
        std::os::unix::fs::symlink(&outside, root.join("link")).unwrap();

        let site = Site {
            hostname: "example.com".into(),
            root: Some(root.to_string_lossy().into_owned()),
            upstream: None,
        };

        assert!(matches!(
            resolve_path(&site, "/link/secret"),
            Err(PathError::OutsideRoot)
        ));

        std::fs::remove_dir_all(&base).unwrap();
    }

    #[test]
    fn test_percent_decode() {
        assert_eq!(percent_decode("/foo/bar").unwrap(), "/foo/bar");
        assert_eq!(percent_decode("/foo%2Fbar").unwrap(), "/foo/bar");
        assert_eq!(percent_decode("/foo%2fbar").unwrap(), "/foo/bar");
        assert_eq!(percent_decode("/foo%20bar").unwrap(), "/foo bar");
        assert_eq!(percent_decode("/foo%2Ebar").unwrap(), "/foo.bar");
        assert_eq!(percent_decode("/foo%2ebar").unwrap(), "/foo.bar");
        assert_eq!(percent_decode("/foo%2F%2Ebar").unwrap(), "/foo/.bar");
        assert!(matches!(
            percent_decode("/foo%ZZbar"),
            Err(PathError::Traversal)
        ));
        assert!(matches!(percent_decode("/foo%"), Err(PathError::Traversal)));
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
