use std::fmt;

use serde::{Deserialize, Serialize};

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
    pub sites: Vec<Site>,
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
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone)]
#[serde(deny_unknown_fields)]
pub struct Site {
    pub hostname: String,
    pub listener: String,
    #[serde(default)]
    pub root: Option<String>,
    #[serde(default)]
    pub upstream: Option<String>,
}

#[derive(Debug, Deserialize, PartialEq, Eq, Clone, Copy)]
#[serde(default, deny_unknown_fields)]
pub struct Limits {
    pub max_connections: u32,
    pub max_header_bytes: usize,
    pub max_body_bytes: usize,
    pub idle_timeout_secs: u64,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_connections: 256,
            max_header_bytes: 8192,
            max_body_bytes: 1_048_576, // 1 Mb
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
    pub major: u8,
    pub minor: u8,
}

#[derive(Debug)]
pub struct ConfigError(pub String);

impl fmt::Display for ConfigError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "config error: {}", self.0)
    }
}

impl std::error::Error for ConfigError {}

impl From<toml::de::Error> for ConfigError {
    fn from(e: toml::de::Error) -> ConfigError {
        ConfigError(e.to_string())
    }
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

#[derive(Debug, Default)]
pub struct Routing {
    pub sites: Vec<Site>,
    pub routes: Vec<(String, String, usize)>,
}
