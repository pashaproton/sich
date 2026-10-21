#![forbid(unsafe_code)]

use std::process::ExitCode;

const EXIT_CONFIG: u8 = 2;

fn main() -> ExitCode {
    let Some(config_path) = parse_args(std::env::args().skip(1)) else {
        return ExitCode::from(EXIT_CONFIG);
    };

    let config_text = match std::fs::read_to_string(&config_path) {
        Ok(text) => text,
        Err(e) => {
            eprintln!("sich [error]: cannot read {config_path}: {e}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    let config = match sich::load_config(&config_text) {
        Ok(config) => config,
        Err(e) => {
            eprintln!("sich [error]: {e}");
            return ExitCode::from(EXIT_CONFIG);
        }
    };

    if config.listeners.is_empty() {
        return ExitCode::SUCCESS;
    }

    match serve(&config) {
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

fn serve(_config: &sich::model::Config) -> Result<(), std::io::Error> {
    todo!("M1: tokio runtime, bind listeners from config, static serving")
}
