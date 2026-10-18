# Minimal Rust Web Server — M0 Specification

Version: 0.1 (draft for review)
Status: scoping
Predecessor: https://github.com/pashaproton/c-server

## 0. Purpose

The Sich — the Zaporozhian Cossack fortress that was never taken in battle.
This project is a web server with the same posture: nothing is open unless
you open it. Written in Rust. No unsafe code. Closed by default.
Serves files over HTTP and HTTPS. Supports domain-based virtual hosts: each
configured domain (or sub-domain) is routed to either a static document root
or a local upstream port.

## 1. Non-negotiable invariants

- INV-1 Voyager-level quality. Static allocation in steady state (no heap
  allocation after startup on the request path). Bounded loops everywhere.
  `#![forbid(unsafe_code)]`. `#![deny(warnings)]` in CI. Iterative parser,
  no recursion. Formal verification (Kani) on the HTTP parser and config
  interpreter.
- INV-2 Closed by default. A fresh install binds no sockets, serves nothing,
  terminates no TLS. Every listener, site, and upstream is explicit,
  opt-in config. There is no dev mode, no implicit loopback binding, no
  magic default port, no CWD-relative serving. The shipped config file has
  every block commented out.
- INV-3 Fail closed. Startup refuses on: config parse error, unknown config
  key, duplicate or ambiguous domain match, unreadable document root,
  unwritable pid/log location. Error output names the offending config line
  and nothing else. The process never falls back to a convenient default.
- INV-4 Bounded resources. Hard caps on concurrent connections, header size,
  body size, and idle time. Caps are tunable but can never be disabled.
- INV-5 Minimal attack surface. Dependency budget: <= 15 crates, each
  documented with a one-line justification. The HTTP parser is fuzzed from
  the first release (cargo-fuzz corpus in-repo).

## 2. Functional requirements (v1)

- FR-1 Listeners. The config defines explicit listeners: bind address +
  port + protocol (http or https). Multiple listeners are allowed. No
  listener exists unless configured.
- FR-2 Virtual hosts. Domains and sub-domains are mapped per listener by
  the HTTP `Host` header (HTTP) or SNI (HTTPS). Example:
  `example.com -> 127.0.0.1:8080`, `website.example.com -> 127.0.0.1:3000`.
  Matching is exact; there is no silent wildcard. A request whose Host
  matches no configured site is rejected (444-style connection close or
  421, decision in FR-8).
- FR-3 Site behavior. Each site is exactly one of:
  - `static`: serve files from an explicit document root. Path traversal,
    symlink escape, and `..` handling must be covered by tests.
  - `proxy`: forward to a local upstream (e.g. 127.0.0.1:3000). v1 scope:
    HTTP/1.1 upstream, streaming request and response bodies, no request
    body buffering to disk.
- FR-4 HTTPS. TLS via rustls. Certificates and keys are explicit file
  paths in config. ACME auto-provisioning is out of scope for v1 (M4).
- FR-5 HTTP/1.1 first. Keep-alive, chunked transfer decoding, correct
  status codes. HTTP/2 is behind a feature flag, not in v1 core.
- FR-6 Logging. Structured access log to a file, explicit path in config,
  opt-in. No logging of request bodies by default.
- FR-7 Admin surface: none. No status page, no metrics endpoint, no admin
  API in v1. A metrics endpoint is a future feature and will itself be
  bind-explicit and opt-in.
- FR-8 Error semantics. Undefined-behavior inputs (malformed request line,
  oversized headers, bad chunk framing) result in a closed connection or
  a well-defined 4xx; the server must remain fully functional for other
  connections. Exact choice per case is fixed in M1 and tested.

## 3. Non-goals (v1)

- No dynamic configuration, no config reload API (restart to apply).
- No scripting, no Lua, no plugins.
- No caching layer, no load balancing, no geo-routing.
- No HTTP/3, no QUIC.
- No CGI, PHP, or application embedding.
- No Windows support (Linux first; macOS build is a bonus).
- No web UI.

## 4. Quality gates (per milestone)

- QG-1 Every requirement has at least one automated test.
- QG-2 cargo-fuzz targets exist for: request parser, config parser,
  proxy framing. Zero crashes on a 24h fuzz run before release.
- QG-3 Kani proofs: parser never panics on any input within the bounded
  grammar; config interpreter rejects all malformed inputs.
- QG-4 Benchmark suite (wrk, fixed hardware, published numbers) against
  nginx and Caddy: static file serving and proxy pass-through.
- QG-5 Allocation check: steady-state request path performs zero heap
  allocation (verified with a counting global allocator in tests).
- QG-6 Deny-by-default audit: fresh default config starts with zero open
  sockets — asserted by an integration test that starts the server with
  the shipped config and scans for listeners (must find none).

## 5. Milestones

- M0 — this document. Exit: decisions D1–D6 confirmed (section 6).
- M1 — HTTP/1.1 static file server over tokio + hyper: config file,
  virtual hosts, correct status codes, keep-alive, INV-1..4 enforced,
  QG-1 passing for FR-1..FR-3, FR-5, FR-8.
- M2 — Hardening: fuzzing (QG-2), Kani proofs (QG-3), privilege drop,
  optional seccomp profile, QG-6.
- M3 — Performance: sendfile/io_uring static path, QG-4 published.
- M4 — TLS (rustls), then ACME. Proxy upstream completion if deferred.
- M5 — Public release: README with benchmark table, security policy,
  reproducible builds.

## 6. Open decisions

- D1 Config format: TOML. DECIDED (2026-10-17): TOML; structure must be clever
  (nested, validated, fail-closed on unknown keys).
- D2 Host-mismatch behavior: close connection vs 421 Misdirected Request.
  DECIDED (2026-10-17): close connection, no response. Rationale: silent close
  leaks no server presence; misconfiguration failing loudly is acceptable and
  consistent with fail-closed posture. Fixing docs/debugging is left to the
  operator deliberately — target audience is experienced users.
- D3 Proxy scope in v1: DECIDED (2026-10-17): in core.
- D4 License: DECIDED (2026-10-17): MIT.
- D5 Project name: DECIDED (2026-10-17): Sich (Січ), after the Zaporozhian
  Sich fortress — never taken in battle; nothing open unless you open it.
  crates.io/gem available. GitHub: github.com/pashaproton/sich (to create).
  Binary name: sich. Config dir: /etc/sich/. Repo: SPEC.md first, no code
  until M0 closed. Tagline: "the fortress that was never taken in battle".
- D6 Static file metadata caching: DECIDED (2026-10-17): none in v1,
  open-per-request; revisit in M3 only if benchmarks justify.
