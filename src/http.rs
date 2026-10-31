#![forbid(unsafe_code)]

use crate::model::{Limits, Request, RequestParseError};

/// # Errors
///
/// Will return `Err` if the request is malformed or exceeds the header size limits.
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

    let mut headers = Vec::new();

    for line in lines {
        if line.is_empty() {
            break;
        }

        let (name, value) = line.split_once(':').ok_or(RequestParseError::Malformed)?;
        let name = name.trim();

        if name.is_empty() {
            eprintln!("header name is empty");
            return Err(RequestParseError::Malformed);
        }

        headers.push((name.to_ascii_lowercase(), value.trim().to_string()));
    }

    Ok(Some(Request {
        method: method.to_string(),
        path: path.to_string(),
        headers,
        major,
        minor,
    }))
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n").map(|p| p + 4)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn header_cap_is_exact() {
        let limits = Limits::default();
        let prefix = "GET / HTTP/1.1\r\nHost: a.com\r\nX-Pad: ";
        let suffix = "\r\n\r\n";
        let pad = limits.max_header_bytes - prefix.len() - suffix.len();
        let req = format!("{prefix}{}{suffix}", "a".repeat(pad));
        assert_eq!(req.len(), limits.max_header_bytes);
        assert!(matches!(
            parse_request(req.as_bytes(), &limits),
            Ok(Some(_))
        ));
    }

    #[test]
    fn oversized_incomplete_head_is_refused() {
        let limits = Limits::default();
        let buf = vec![b'a'; limits.max_header_bytes + 1];
        assert!(matches!(
            parse_request(&buf, &limits),
            Err(RequestParseError::HeaderTooLarge)
        ));
    }

    #[test]
    fn test_find_header_end() {
        let buf = b"GET / HTTP/1.1\r\nHost: example.com\r\n\r\n";
        assert_eq!(find_header_end(buf), Some(buf.len()));

        let buf = b"GET / HTTP/1.1\r\nHost: example.com\r\n";
        assert_eq!(find_header_end(buf), None);

        let buf = b"GET / HTTP/1.1\r\nHost: example.com\r\nX-Header: value\r\n\r\n";
        assert_eq!(find_header_end(buf), Some(buf.len()));
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
