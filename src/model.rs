#![forbid(unsafe_code)]

use std::fmt;

use serde::{Deserialize, Serialize};
use tokio::time::error::Elapsed;

use crate::config::ConfigError;

#[derive(Debug, Deserialize, Serialize, PartialEq, Eq, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Http,
    Https,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub listeners: Vec<Listener>,
    #[serde(default)]
    pub limits: Limits,
    #[serde(default)]
    pub log: Option<Log>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Listener {
    pub bind: String,
    pub protocol: Protocol,
    #[serde(default)]
    pub cert: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    #[serde(default)]
    pub sites: Vec<Site>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Site {
    pub hostname: String,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub upstream: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_connections: usize,
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
    pub idle_timeout_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_connections: 256,
            max_header_bytes: 8192,
            max_body_bytes: 1_048_576, // 1 MiB
            idle_timeout_secs: 30,
        }
    }
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Log {
    pub access: String,
}

#[derive(Debug, PartialEq, Eq)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub major: u8,
    pub minor: u8,
}

#[derive(Debug)]
pub enum RequestParseError {
    HeaderTooLarge,
    Malformed,
}

impl fmt::Display for RequestParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RequestParseError::HeaderTooLarge => write!(f, "header too large"),
            RequestParseError::Malformed => write!(f, "malformed request"),
        }
    }
}

impl std::error::Error for RequestParseError {}

#[derive(Debug, Default, Clone)]
pub struct Routing {
    pub sites: Vec<Site>,
    pub routes: Vec<(String, usize)>,
}

#[derive(Debug)]
pub enum ServeError {
    Config(ConfigError),
    Io(std::io::Error),
    JoinError(tokio::task::JoinError),
    Path(crate::path::PathError),
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeError::Config(e) => write!(f, "{e}"),
            ServeError::Io(e) => write!(f, "io error: {e}"),
            ServeError::JoinError(e) => write!(f, "task join error: {e}"),
            ServeError::Path(e) => write!(f, "path error: {e}"),
        }
    }
}

impl std::error::Error for ServeError {}

impl From<ConfigError> for ServeError {
    fn from(e: ConfigError) -> Self {
        ServeError::Config(e)
    }
}

impl From<Elapsed> for ServeError {
    fn from(e: Elapsed) -> Self {
        ServeError::Io(std::io::Error::new(std::io::ErrorKind::TimedOut, e))
    }
}

impl From<std::io::Error> for ServeError {
    fn from(e: std::io::Error) -> Self {
        ServeError::Io(e)
    }
}

impl From<tokio::task::JoinError> for ServeError {
    fn from(e: tokio::task::JoinError) -> Self {
        ServeError::JoinError(e)
    }
}

#[repr(u16)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Ok = 200,
    NoContent = 204,
    NotModified = 304,
    BadRequest = 400,
    Forbidden = 403,
    NotFound = 404,
    MethodNotAllowed = 405,
    RequestTimeout = 408,
    MisdirectedRequest = 421,
    PayloadTooLarge = 413,
    UriTooLong = 414,
    HeaderTooLarge = 431,
    InternalServerError = 500,
    BadGateway = 502,
    ServiceUnavailable = 503,
    GatewayTimeout = 504,
}

impl Status {
    pub fn code(self) -> u16 {
        self as u16
    }

    pub fn reason(self) -> &'static str {
        match self {
            Status::Ok => "OK",
            Status::NoContent => "No Content",
            Status::NotModified => "Not Modified",
            Status::BadRequest => "Bad Request",
            Status::Forbidden => "Forbidden",
            Status::NotFound => "Not Found",
            Status::MethodNotAllowed => "Method Not Allowed",
            Status::RequestTimeout => "Request Timeout",
            Status::MisdirectedRequest => "Misdirected Request",
            Status::PayloadTooLarge => "Payload Too Large",
            Status::UriTooLong => "URI Too Long",
            Status::HeaderTooLarge => "Request Header Fields Too Large",
            Status::InternalServerError => "Internal Server Error",
            Status::BadGateway => "Bad Gateway",
            Status::ServiceUnavailable => "Service Unavailable",
            Status::GatewayTimeout => "Gateway Timeout",
        }
    }

    /// The built-in body: one line, no HTML, no personality.
    pub fn default_body(self) -> String {
        format!("{} {}", self.code(), self.reason())
    }

    /// 204/304/1xx must never carry a body or a Content-Length.
    pub fn is_bodyless(self) -> bool {
        matches!(self, Status::NoContent | Status::NotModified)
    }

    /// Config gives us strings; reject anything not in the enum.
    pub fn from_code(code: u16) -> Option<Status> {
        Some(match code {
            200 => Status::Ok,
            204 => Status::NoContent,
            304 => Status::NotModified,
            400 => Status::BadRequest,
            403 => Status::Forbidden,
            404 => Status::NotFound,
            405 => Status::MethodNotAllowed,
            408 => Status::RequestTimeout,
            421 => Status::MisdirectedRequest,
            413 => Status::PayloadTooLarge,
            414 => Status::UriTooLong,
            431 => Status::HeaderTooLarge,
            500 => Status::InternalServerError,
            502 => Status::BadGateway,
            503 => Status::ServiceUnavailable,
            504 => Status::GatewayTimeout,
            _ => return None,
        })
    }
}
