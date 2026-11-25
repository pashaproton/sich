use std::path::Path;

use tokio::io::{AsyncWrite, AsyncWriteExt};

use crate::{
    model::{ServeError, Status},
    path::PathError,
};

/// Extension → media type. Unknown extension is octet-stream.
#[must_use]
pub fn content_type(path: &Path) -> &'static str {
    match path
        .extension()
        .and_then(|ext| ext.to_str())
        .map(|s| s.to_ascii_lowercase())
    {
        Some(ref ext) if ext == "html" => "text/html",
        Some(ref ext) if ext == "css" => "text/css",
        Some(ref ext) if ext == "js" => "text/javascript",
        Some(ref ext) if ext == "json" => "application/json",
        Some(ref ext) if ext == "png" => "image/png",
        Some(ref ext) if ext == "jpg" || ext == "jpeg" => "image/jpeg",
        Some(ref ext) if ext == "svg" => "image/svg+xml",
        Some(ref ext) if ext == "txt" => "text/plain",
        Some(ref ext) if ext == "gz" => "application/gzip",
        _ => "application/octet-stream",
    }
}

/// PathError → (status line, empty-body length 0).
#[must_use]
pub fn error_reply(err: &PathError) -> Status {
    match err {
        PathError::Traversal => Status::Forbidden,
        PathError::OutsideRoot => Status::Forbidden,
        PathError::NotFound => Status::NotFound,
    }
}

/// Build the response head. Body length is always explicit.
#[must_use]
pub fn head(status: Status, ctype: &str, len: u64) -> String {
    format!("HTTP/1.1 {}\r\nContent-Type: {ctype}\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n", status.default_body())
}

pub async fn write_error_status<S>(socket: &mut S, status: Status) -> Result<(), ServeError>
where
    S: AsyncWrite + Unpin,
{
    let h = head(status, "text/plain", status.default_body().len() as u64);
    let b = Status::NotFound.default_body();
    socket.write_all(format!("{}{}", h, b).as_bytes()).await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use crate::path::PathError;

    use super::*;

    #[test]
    fn test_content_type() {
        assert_eq!(content_type(Path::new("a.html")), "text/html");
        assert_eq!(content_type(Path::new("a.css")), "text/css");
        assert_eq!(content_type(Path::new("a.js")), "text/javascript");
        assert_eq!(content_type(Path::new("a.json")), "application/json");
        assert_eq!(content_type(Path::new("a.png")), "image/png");
        assert_eq!(content_type(Path::new("a.jpg")), "image/jpeg");
        assert_eq!(content_type(Path::new("a.svg")), "image/svg+xml");
        assert_eq!(content_type(Path::new("a.txt")), "text/plain");
        assert_eq!(content_type(Path::new("a.PNG")), "image/png");
        assert_eq!(content_type(Path::new("noext")), "application/octet-stream");
        assert_eq!(content_type(Path::new("a.tar.gz")), "application/gzip");
    }

    #[test]
    fn test_error_reply() {
        assert_eq!(error_reply(&PathError::Traversal), Status::Forbidden);
        assert_eq!(error_reply(&PathError::OutsideRoot), Status::Forbidden);
        assert_eq!(error_reply(&PathError::NotFound), Status::NotFound);
    }

    #[test]
    fn test_head() {
        let h = head(Status::Ok, "text/plain", 12345);
        assert!(h.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(h.contains("Content-Type: text/plain\r\n"));
        assert!(h.contains("Content-Length: 12345\r\n"));
        assert!(h.contains("Connection: close\r\n\r\n"));
    }
}
