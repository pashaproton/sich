#![forbid(unsafe_code)]

use std::{process::ExitCode, sync::Arc, time::Duration};

use sich::{
    files::{content_type, error_reply, head, write_error_status},
    http::parse_request,
    is_transient_accept_error,
    model::{Limits, Listener, Request, Routing, ServeError, Site, Status},
    path::{resolve_path, PathError},
    router::{build_routing, host_header_of, route},
};
use tokio::{
    io::{AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpListener,
    sync::Semaphore,
    task::JoinSet,
    time::timeout,
};

const EXIT_CONFIG: u8 = 2;

#[tokio::main]
async fn main() -> ExitCode {
    let Some(config_path) = parse_args(std::env::args().skip(1)) else {
        eprintln!("sich [error]: missing --config argument");
        return ExitCode::from(EXIT_CONFIG);
    };

    let config_text = match std::fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("sich [error]: cannot read {config_path}: {e}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    let config = match sich::config::load_config(&config_text) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("sich [error]: {e}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    if config.listeners.is_empty() {
        eprintln!("sich [info]: no listeners found in config, exiting");
        return ExitCode::SUCCESS;
    }

    match serve(&config).await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("sich [error]: {e}");
            ExitCode::from(EXIT_CONFIG)
        }
    }
}

fn parse_args(args: impl Iterator<Item = String>) -> Option<String> {
    let mut it = args.peekable();
    match it.next()?.as_ref() {
        "--config" => it.next(),
        _ => None,
    }
}

async fn serve(config: &sich::model::Config) -> Result<(), ServeError> {
    let limits = Arc::new(config.limits);
    let routing = Arc::new(build_routing(&config.listeners));
    let semaphore = Arc::new(Semaphore::new(config.limits.max_connections));
    let mut tasks = JoinSet::new();

    for listener in &config.listeners {
        tasks.spawn(serve_listener(
            limits.clone(),
            routing.clone(),
            listener.clone(),
            semaphore.clone(),
        ));
    }

    while let Some(res) = tasks.join_next().await {
        res.map_err(ServeError::JoinError)??;
    }

    Ok(())
}

async fn serve_listener(
    limits: Arc<Limits>,
    routing: Arc<Routing>,
    listener: Listener,
    semaphore: Arc<Semaphore>,
) -> Result<(), ServeError> {
    let listener_bind = Arc::new(listener.bind.clone());
    let tcp_listener = TcpListener::bind(&listener.bind).await?;
    println!("listening {}", listener_bind);

    loop {
        let permit = Arc::clone(&semaphore)
            .acquire_owned()
            .await
            .expect("semaphore should not be closed");

        let limits = Arc::clone(&limits);
        let routing = Arc::clone(&routing);
        let listener_bind = Arc::clone(&listener_bind);

        let (mut socket, _) = match tcp_listener.accept().await {
            Ok(pair) => pair,
            Err(e) if is_transient_accept_error(e.kind()) => continue,
            Err(e) => return Err(ServeError::Io(e)),
        };

        tokio::spawn(async move {
            let _permit = permit;

            let idle = Duration::from_secs(limits.idle_timeout_secs);
            let mut buf: Vec<u8> = Vec::with_capacity(1024);
            let mut chunk = [0u8; 1024];

            let request = loop {
                let n = match timeout(idle, socket.read(&mut chunk)).await {
                    Ok(Ok(0)) => return,
                    Ok(Ok(n)) => n,
                    Ok(Err(e)) => {
                        eprintln!("failed to read from socket: {e}");
                        return;
                    }
                    Err(_) => {
                        eprintln!("connection timed out");
                        return;
                    }
                };

                buf.extend_from_slice(&chunk[..n]);

                match parse_request(&buf, &limits) {
                    Ok(Some(request)) => break request,
                    Ok(None) => {}
                    Err(e) => {
                        eprintln!("failed to read from socket: {e}");
                        return;
                    }
                }
            };

            let Some(host) = host_header_of(&request.headers) else {
                eprintln!("missing Host header");
                return;
            };
            let Some(site) = route(&routing, &listener_bind, host) else {
                eprintln!("no route for host {host} on listener {listener_bind}");
                return;
            };

            let _ = serve_static(&mut socket, &request, site, &limits).await;
        });
    }
}

pub async fn serve_static<S>(
    socket: &mut S,
    request: &Request,
    site: &Site,
    limits: &Limits,
) -> Result<(), ServeError>
where
    S: AsyncWrite + Unpin,
{
    if request.method != "GET" && request.method != "HEAD" {
        write_error_status(socket, Status::MethodNotAllowed).await?; // with Allow: GET, HEAD
        return Ok(());
    }

    let path = request.path.split('?').next().unwrap_or("");

    let mut file_path = match resolve_path(site, path) {
        Ok(p) => p,
        Err(e) => {
            let status = error_reply(&e);
            socket
                .write_all(
                    head(status, "text/plain", status.default_body().len() as u64).as_bytes(),
                )
                .await?;
            socket.write_all(status.default_body().as_bytes()).await?;
            return Err(ServeError::Path(e));
        }
    };

    let mut meta = match tokio::fs::metadata(&file_path).await {
        Ok(m) => m,
        Err(e) => {
            write_error_status(socket, Status::NotFound).await?;
            return Err(ServeError::Io(e));
        }
    };

    if meta.is_dir() {
        file_path = file_path.join("index.html");
        meta = match tokio::fs::metadata(&file_path).await {
            Ok(m) => m,
            Err(e) => {
                write_error_status(socket, Status::NotFound).await?;
                return Err(ServeError::Io(e));
            }
        };
        if meta.is_dir() {
            write_error_status(socket, Status::NotFound).await?;
            return Err(ServeError::Path(PathError::NotFound));
        }
    }

    let ctype = content_type(&file_path);
    socket
        .write_all(head(Status::Ok, ctype, meta.len()).as_bytes())
        .await?;

    if request.method == "HEAD" {
        return Ok(());
    }

    let mut file = tokio::fs::File::open(&file_path).await?;
    let mut buf = vec![0u8; 16 * 1024];
    let idle = Duration::from_secs(limits.idle_timeout_secs);
    loop {
        let n = timeout(idle, file.read(&mut buf)).await??;
        if n == 0 {
            break Ok(());
        }
        timeout(idle, socket.write_all(&buf[..n])).await??;
    }
}
