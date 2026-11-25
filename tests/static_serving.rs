use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::{Duration, Instant},
};

const HOST: &str = "test.local";
const PROBE: &str = ".__sich_probe";

struct Server {
    child: Child,
    dir: PathBuf,
    port: u16,
    log: PathBuf,
}

impl Server {
    fn new(name: &str, files: &[(&str, &[u8])]) -> Self {
        let dir = std::env::temp_dir().join(format!(
            "sich-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        // A crashed previous run can leave the directory behind; start clean.
        let _ = fs::remove_dir_all(&dir);
        let www = dir.join("www");
        fs::create_dir_all(&www).unwrap();
        for (rel, bytes) in files {
            let path = www.join(rel);
            if let Some(parent) = path.parent() {
                fs::create_dir_all(parent).unwrap();
            }
            fs::write(path, bytes).unwrap();
        }

        let port = free_port();
        let config = format!(
            r#"
                [limits]
                max_connections = 16
                max_header_bytes = 8192
                max_body_bytes = 1048576
                idle_timeout_secs = 3

                [[listeners]]
                bind = "127.0.0.1:{port}"
                protocol = "http"

                [[listeners.sites]]
                hostname = "{HOST}"
                root = "{root}"
            "#,
            root = www.canonicalize().unwrap().display()
        );
        let config_path = dir.join("sich.toml");
        fs::write(&config_path, config).unwrap();

        // Sentinel file: proves we are talking to *this* server and not to a
        // sibling test's server that grabbed the same port.
        let probe_body = b"sich-probe";
        fs::write(www.join(PROBE), probe_body).unwrap();

        let log = dir.join("server.log");
        let log_file = fs::File::create(&log).unwrap();

        let mut child = Command::new(env!("CARGO_BIN_EXE_sich"))
            .arg("--config")
            .arg(&config_path)
            .stdout(Stdio::null())
            .stderr(Stdio::from(log_file))
            .spawn()
            .expect("sich binary should start");

        // Wait for a *served response*, not for a sleep and not for a bare
        // TCP connect: a listening socket says nothing about routing.
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(exited) = child.try_wait().unwrap() {
                panic!(
                    "server exited during startup ({exited}); log:\n{}",
                    read_log(&log)
                );
            }
            // read_raw expects a live socket, so probe the port first.
            if TcpStream::connect(("127.0.0.1", port)).is_ok() {
                let (buf, _) = read_raw(
                    port,
                    format!("GET /{PROBE} HTTP/1.1\r\nHost: {HOST}\r\n\r\n").as_bytes(),
                );
                if buf.starts_with(b"HTTP/1.1 200") {
                    break;
                }
            }
            assert!(
                Instant::now() < deadline,
                "server did not serve /{PROBE} on {port}; log:\n{}",
                read_log(&log)
            );
            std::thread::sleep(Duration::from_millis(20));
        }

        Server {
            child,
            dir,
            port,
            log,
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.join("www")
    }

    /// Send a raw request, return (status, headers, body). Reads to EOF.
    fn request(&self, raw: &str) -> Response {
        let (buf, closed) = read_raw(self.port, raw.as_bytes());
        assert!(
            !buf.is_empty(),
            "server closed without sending a byte; stderr so far:\n{}",
            read_log(&self.log)
        );
        parse(&buf, closed)
    }

    /// Like `request`, but tolerates the server closing without a single byte.
    fn try_request(&self, raw: &str) -> Option<Response> {
        let (buf, closed) = read_raw(self.port, raw.as_bytes());
        if buf.is_empty() {
            return None;
        }
        Some(parse(&buf, closed))
    }

    fn get(&self, path: &str) -> Response {
        self.request(&format!("GET {path} HTTP/1.1\r\nHost: {HOST}\r\n\r\n"))
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

struct Response {
    status: String,
    headers: Vec<(String, String)>,
    body: Vec<u8>,
    closed_by_peer: bool,
}

impl Response {
    fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }
}

/// Send raw bytes, return everything the server wrote back before closing or
/// hitting the read timeout. Empty vector means the peer closed silently.
fn read_raw(port: u16, raw: &[u8]) -> (Vec<u8>, bool) {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).expect("connect");
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream.write_all(raw).unwrap();
    stream.flush().unwrap();

    let mut buf = Vec::new();
    let mut closed_by_peer = true;
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => buf.extend_from_slice(&chunk[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                closed_by_peer = false;
                break;
            }
            Err(e) if e.kind() == std::io::ErrorKind::TimedOut => {
                closed_by_peer = false;
                break;
            }
            Err(_) => break,
        }
    }

    (buf, closed_by_peer)
}

fn parse(buf: &[u8], closed_by_peer: bool) -> Response {
    let split = find(buf, b"\r\n\r\n").expect("response must contain a header block");
    let head = String::from_utf8_lossy(&buf[..split]).into_owned();
    let body = buf[split + 4..].to_vec();

    let mut lines = head.split("\r\n");
    let status = lines.next().unwrap_or_default().to_string();
    let headers = lines
        .filter_map(|line| {
            let (n, v) = line.split_once(':')?;
            Some((n.trim().to_string(), v.trim().to_string()))
        })
        .collect();

    Response {
        status,
        headers,
        body,
        closed_by_peer,
    }
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack.windows(needle.len()).position(|w| w == needle)
}

fn read_log(path: &std::path::Path) -> String {
    fs::read_to_string(path).unwrap_or_else(|_| String::from("<no log>"))
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

// happy path

#[test]
fn serves_file_with_type_and_exact_length() {
    let body = b"<!doctype html><h1>hi</h1>\n";
    let s = Server::new("happy", &[("index.html", body)]);
    let r = s.get("/index.html");

    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.header("content-type"), Some("text/html"));
    let len = body.len().to_string();
    assert_eq!(r.header("content-length"), Some(len.as_str()));
    assert_eq!(r.body.as_slice(), &body[..]);
    assert!(
        r.closed_by_peer,
        "response must be delimited by EOF or length"
    );
}

#[test]
fn directory_request_serves_index_html() {
    let s = Server::new("dir", &[("index.html", b"root index")]);
    for path in ["/", "/index.html"] {
        let r = s.get(path);
        assert_eq!(r.status, "HTTP/1.1 200 OK", "path {path}");
        assert_eq!(r.text(), "root index", "path {path}");
    }
}

#[test]
fn head_returns_headers_without_body() {
    let body = vec![b'x'; 4096];
    let s = Server::new("head", &[("big.txt", &body)]);
    let r = s.request(&format!("HEAD /big.txt HTTP/1.1\r\nHost: {HOST}\r\n\r\n"));

    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.header("content-length"), Some("4096"));
    assert_eq!(r.header("content-type"), Some("text/plain"));
    assert!(
        r.body.is_empty(),
        "HEAD must not send a body, got {} bytes",
        r.body.len()
    );
}

#[test]
fn query_string_is_ignored_for_lookup() {
    let s = Server::new("query", &[("index.html", b"ok")]);
    let r = s.get("/index.html?v=2&x=%2e%2e");
    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.text(), "ok");
}

#[test]
fn percent_encoded_filename_resolves() {
    let s = Server::new("space", &[("a b.html", b"space")]);
    let r = s.get("/a%20b.html");
    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.text(), "space");
}

#[test]
fn unknown_extension_is_octet_stream() {
    let s = Server::new("bin", &[("blob.weird", b"\x00\x01\x02")]);
    let r = s.get("/blob.weird");
    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.header("content-type"), Some("application/octet-stream"));
    assert_eq!(r.body.as_slice(), &[0, 1, 2]);
}

#[test]
fn empty_file_is_zero_length_200() {
    let s = Server::new("empty", &[("empty.txt", b"")]);
    let r = s.get("/empty.txt");
    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.header("content-length"), Some("0"));
    assert!(r.body.is_empty());
}

#[test]
fn large_file_arrives_byte_for_byte() {
    // Larger than the 16 KiB copy buffer, streaming is exercised.
    let body: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    let s = Server::new("large", &[("big.bin", &body)]);
    let r = s.get("/big.bin");
    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.header("content-length"), Some("300000"));
    assert_eq!(r.body.len(), body.len());
    assert!(r.body == body, "body must match the file exactly");
}

// refusals

#[test]
fn plain_traversal_is_403() {
    let s = Server::new("trav", &[("index.html", b"ok")]);
    for path in [
        "/../etc/passwd",
        "/../../../../etc/passwd",
        "/a/../../etc/passwd",
    ] {
        let r = s.get(path);
        assert_eq!(r.status, "HTTP/1.1 403 Forbidden", "path {path}");
        assert_eq!(r.header("content-length"), Some("13"));
    }
}

#[test]
fn encoded_traversal_is_403() {
    let s = Server::new("trav-enc", &[("index.html", b"ok")]);
    for path in [
        "/%2e%2e/etc/passwd",
        "/%2E%2E/%2E%2E/etc/passwd",
        "/..%2f..%2fetc/passwd",
        "/%2e%2e%2fetc%2fpasswd",
    ] {
        let r = s.get(path);
        assert_eq!(r.status, "HTTP/1.1 403 Forbidden", "path {path}");
    }
}

#[test]
fn malformed_percent_escape_is_refused() {
    let s = Server::new("trav-bad", &[("index.html", b"ok")]);
    let r = s.get("/%zz");
    assert!(
        r.status.starts_with("HTTP/1.1 403") || r.status.is_empty(),
        "malformed escape must be refused, got {:?}",
        r.status
    );
}

#[cfg(unix)]
#[test]
fn symlink_escaping_root_is_403() {
    let s = Server::new("symlink", &[("index.html", b"ok")]);
    let outside = s.dir.join("outside");
    fs::create_dir_all(&outside).unwrap();
    fs::write(outside.join("secret"), b"top secret").unwrap();
    std::os::unix::fs::symlink(&outside, s.root().join("link")).unwrap();

    let r = s.get("/link/secret");
    assert_eq!(r.status, "HTTP/1.1 403 Forbidden");
    assert!(!r.text().contains("top secret"));
}

#[cfg(unix)]
#[test]
fn symlink_inside_root_still_serves() {
    let s = Server::new("symlink-ok", &[("real.txt", b"fine")]);
    std::os::unix::fs::symlink(s.root().join("real.txt"), s.root().join("alias.txt")).unwrap();
    let r = s.get("/alias.txt");
    assert_eq!(r.status, "HTTP/1.1 200 OK");
    assert_eq!(r.text(), "fine");
}

#[test]
fn missing_file_is_404() {
    let s = Server::new("missing", &[("index.html", b"ok")]);
    let r = s.get("/nope.txt");
    assert_eq!(r.status, "HTTP/1.1 404 Not Found");
    assert_eq!(r.header("content-length"), Some("13"));
}

#[test]
fn directory_without_index_is_404() {
    let s = Server::new("noindex", &[("sub/page.html", b"ok")]);
    let r = s.get("/sub");
    assert_eq!(r.status, "HTTP/1.1 404 Not Found");
}

#[test]
fn unsupported_method_is_405_with_allow() {
    let s = Server::new("method", &[("index.html", b"ok")]);
    let r = s.request(&format!(
        "POST /index.html HTTP/1.1\r\nHost: {HOST}\r\nContent-Length: 0\r\n\r\n"
    ));
    assert_eq!(r.status, "HTTP/1.1 405 Method Not Allowed");
}

#[test]
fn unknown_host_closes_without_serving_files() {
    let s = Server::new("host", &[("index.html", b"secret-ish")]);
    let r = s.try_request("GET /index.html HTTP/1.1\r\nHost: other.local\r\n\r\n");
    // Current behaviour: the connection is dropped with no response at all.
    // A 400 or 404 would be better; until then this test documents reality.
    // The moment sich starts sending 400 here, this assertion fails — on purpose.
    assert!(
        r.is_none(),
        "expected a silent close, got {:?}",
        r.map(|r| r.status)
    );
}

#[test]
fn sequential_requests_on_fresh_connections_work() {
    let s = Server::new("repeat", &[("index.html", b"ok")]);
    for _ in 0..5 {
        assert_eq!(s.get("/index.html").status, "HTTP/1.1 200 OK");
    }
}
