# Sich (Січ)

The fortress that was never taken in battle.

Sich is a minimalistic web server written in Rust, focused on speed,
stability, and security. It serves static files and proxies to local
upstreams over HTTP and HTTPS.

## Posture

- **Closed by default.** A fresh install binds no sockets, serves nothing,
  terminates no TLS. Every listener, site, and upstream is explicit config.
- **Fail closed.** Bad config, unknown keys, ambiguous routes — the server
  refuses to start. It never falls back to a convenient default.
- **No unsafe code.** `#![forbid(unsafe_code)]`, enforced in CI.
- **Voyager-grade discipline.** Static allocation in steady state, bounded
  loops, iterative parser, formal verification of the HTTP parser.
- **Small.** Dependency budget: ≤ 15 crates, each documented.

## Status

M0 (specification) complete. See [SPEC.md](SPEC.md). No production code yet.

## License

MIT
