#![forbid(unsafe_code)]

use std::{
    fmt,
    path::{Path, PathBuf},
};

use crate::model::Site;

#[derive(Debug)]
pub enum PathError {
    Traversal,
    OutsideRoot,
    NotFound,
}

impl fmt::Display for PathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PathError::Traversal => write!(f, "path traversal refused"),
            PathError::OutsideRoot => write!(f, "path escapes document root"),
            PathError::NotFound => write!(f, "not found"),
        }
    }
}

impl std::error::Error for PathError {}

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
