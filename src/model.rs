#![forbid(unsafe_code)]

use std::fmt;

use serde::{Deserialize, Serialize};

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
}

impl fmt::Display for ServeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ServeError::Config(e) => write!(f, "{e}"),
            ServeError::Io(e) => write!(f, "io error: {e}"),
            ServeError::JoinError(e) => write!(f, "task join error: {e}"),
        }
    }
}

impl std::error::Error for ServeError {}

impl From<ConfigError> for ServeError {
    fn from(e: ConfigError) -> Self {
        ServeError::Config(e)
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
