#![forbid(unsafe_code)]

use std::{process::ExitCode, sync::Arc, time::Duration};

use sich::{
    build_routing, host_header_of,
    http::parse_request,
    is_transient_accept_error,
    model::{Limits, Listener, Routing, ServeError},
    route,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
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

            let body = format!("hello from server\n{site:#?}");
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = socket.write_all(response.as_bytes()).await;
        });
    }
}
