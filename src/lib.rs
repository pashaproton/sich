use std::path::{Path, PathBuf};

use crate::model::{
    Config, ConfigError, Limits, PathError, Protocol, Request, RequestParseError, Routing, Site,
};

mod model;

pub fn load_config(input: &str) -> Result<Config, ConfigError> {
    let config: Config = toml::from_str(input)?;
    validate(&config)?;
    Ok(config)
}

fn validate(config: &Config) -> Result<(), ConfigError> {
    for listener in &config.listeners {
        if listener.protocol == Protocol::Https
            && (listener.cert.is_none() || listener.key.is_none())
        {
            return Err(ConfigError(format!(
                "listener {}: https requires both cert and key",
                listener.bind
            )));
        }
    }
    for site in &config.sites {
        let has_root = site.root.is_some();
        let has_upstream = site.upstream.is_some();
        if has_root == has_upstream {
            return Err(ConfigError(format!(
                "site {}: exactly one of root or upstream is required",
                site.hostname
            )));
        }
        if !config.listeners.iter().any(|l| l.bind == site.listener) {
            return Err(ConfigError(format!(
                "site {}: listener {} does not match any configured listener",
                site.hostname, site.listener
            )));
        }
        let dupes = config
            .sites
            .iter()
            .filter(|s| s.hostname == site.hostname && s.listener == site.listener)
            .count();
        if dupes > 1 {
            return Err(ConfigError(format!(
                "site {}: hostname {} is defined twice on listener {}",
                site.hostname, site.hostname, site.listener
            )));
        }
    }
    Ok(())
}

pub fn build_routing(config: &Config) -> Result<Routing, ConfigError> {
    let mut routing = Routing {
        sites: config.sites.clone(),
        routes: Vec::with_capacity(config.sites.len()),
    };
    for (idx, site) in config.sites.iter().enumerate() {
        routing
            .routes
            .push((site.listener.clone(), site.hostname.clone(), idx));
    }
    Ok(routing)
}

pub fn route<'a>(routing: &'a Routing, listener: &str, host: &str) -> Option<&'a Site> {
    routing
        .routes
        .iter()
        .find(|(l, h, _)| l == listener && h == host)
        .map(|(_, _, idx)| &routing.sites[*idx])
}

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
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let h = bytes.get(i + 1).copied().ok_or(PathError::Traversal)?;
            let l = bytes.get(i + 2).copied().ok_or(PathError::Traversal)?;
            let hex = match (hex_val(h), hex_val(l)) {
                (Some(hi), Some(lo)) => (hi << 4) | lo,
                _ => return Err(PathError::Traversal),
            };
            out.push(hex);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
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

pub fn parse_request(buf: &[u8], limits: &Limits) -> Result<Option<Request>, RequestParseError> {
    if buf.len() > limits.max_header_bytes {
        return Err(RequestParseError::HeaderTooLarge);
    }
    let Some(end) = find_header_end(buf) else {
        return Ok(None); // incomplete request; keep reading
    };
    if end > limits.max_header_bytes {
        return Err(RequestParseError::HeaderTooLarge);
    }
    parse_head(&buf[..end])
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

fn parse_head(head: &[u8]) -> Result<Option<Request>, RequestParseError> {
    let text = std::str::from_utf8(head).map_err(|_| RequestParseError::Malformed)?;
    let mut lines = text.split("\r\n");
    let request_line = lines.next().ok_or(RequestParseError::Malformed)?;
    let mut parts = request_line.split(' ');
    let method = parts.next().ok_or(RequestParseError::Malformed)?;
    let path = parts.next().ok_or(RequestParseError::Malformed)?;
    let version = parts.next().ok_or(RequestParseError::Malformed)?;
    if parts.next().is_some() {
        return Err(RequestParseError::Malformed);
    }
    let mut v = version.split('.');
    let major = v.next().ok_or(RequestParseError::Malformed)?;
    let minor = v.next().ok_or(RequestParseError::Malformed)?;
    // version arrives as "HTTP/1.1" — strip the prefix
    let major = major
        .strip_prefix("HTTP/")
        .ok_or(RequestParseError::Malformed)?;
    let major: u8 = major.parse().map_err(|_| RequestParseError::Malformed)?;
    let minor: u8 = minor.parse().map_err(|_| RequestParseError::Malformed)?;
    Ok(Some(Request {
        method: method.to_string(),
        path: path.to_string(),
        major,
        minor,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::RequestParseError;

    // M1-0 / INV-2: empty config is VALID and yields zero listeners.
    #[test]
    fn empty_config_means_nothing_bound() {
        let cfg = load_config("").unwrap();
        assert!(cfg.listeners.is_empty());
        assert!(cfg.sites.is_empty());
        let routing = build_routing(&cfg).unwrap();
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
            [[sites]]
            hostname = "example.com"
            listener = "0.0.0.0:80"
            root = "/var/www/a"
            [[sites]]
            hostname = "example.com"
            listener = "0.0.0.0:80"
            root = "/var/www/b"
        "#;
        assert!(load_config(src).is_err());
    }

    #[test]
    fn https_requires_cert_and_key() {
        let src = "[[listeners]]\nbind = \"0.0.0.0:443\"\nprotocol = \"https\"\n";
        assert!(load_config(src).is_err());
    }

    #[test]
    fn https_with_cert_and_key_is_accepted() {
        let src = "[[listeners]]\nbind = \"0.0.0.0:443\"\nprotocol = \"https\"\ncert = \"/etc/sich/tls/a.crt\"\nkey = \"/etc/sich/tls/a.key\"\n";
        assert!(load_config(src).is_ok());
    }

    #[test]
    fn exact_host_match_routes() {
        let cfg = load_config(
            "[[listeners]]\nbind = \"0.0.0.0:80\"\nprotocol = \"http\"\n[[sites]]\nhostname = \"example.com\"\nlistener = \"0.0.0.0:80\"\nroot = \"/var/www\"",
        )
        .unwrap();
        let routing = build_routing(&cfg).unwrap();
        assert!(route(&routing, "0.0.0.0:80", "example.com").is_some());
        assert!(route(&routing, "0.0.0.0:80", "sub.example.com").is_none()); // no wildcard, ever
    }

    #[test]
    fn traversal_is_refused() {
        let site = Site {
            hostname: "example.com".into(),
            listener: "0.0.0.0:80".into(),
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
    }

    #[test]
    fn parser_bounded_by_header_cap() {
        let limits = Limits::default();
        let big = vec![b'a'; limits.max_header_bytes + 1];
        let req = format!(
            "GET / HTTP/1.1\r\nX-Long: {}\r\n\r\n",
            String::from_utf8(big).unwrap()
        );
        assert!(matches!(
            parse_request(req.as_bytes(), &limits),
            Err(RequestParseError::HeaderTooLarge)
        ));
    }

    #[test]
    fn parser_parses_simple_get() {
        let req = b"GET /index.html HTTP/1.1\r\nHost: example.com\r\n\r\n";
        let parsed = parse_request(req, &Limits::default()).unwrap().unwrap();
        assert_eq!(parsed.method, "GET");
        assert_eq!(parsed.path, "/index.html");
        assert_eq!((parsed.major, parsed.minor), (1, 1));
    }

    #[test]
    fn parser_incomplete_request_needs_more_bytes() {
        let req = b"GET / HTTP/1.1\r\nHost: ex";
        assert!(matches!(parse_request(req, &Limits::default()), Ok(None)));
    }

    #[test]
    fn parser_rejects_malformed_request_line() {
        let req = b"NOT-A-REQUEST\r\n\r\n";
        assert!(matches!(
            parse_request(req, &Limits::default()),
            Err(RequestParseError::Malformed)
        ));
    }
}
