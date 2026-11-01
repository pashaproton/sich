#![forbid(unsafe_code)]

pub mod config;
pub mod http;
pub mod model;
pub mod path;
pub mod router;

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
