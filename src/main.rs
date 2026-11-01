use std::io::IsTerminal;
use std::process::ExitCode;

use clap::Parser;
use tracing_subscriber::EnvFilter;

use runspace::cli::Cli;
use runspace::{commands, ui};

#[tokio::main]
async fn main() -> ExitCode {
    runspace::platform::init();
    let args: Vec<String> = std::env::args().collect();
    if let Some(code) = runspace::platform::internal_mode(&args) {
        return code;
    }

    let cli = Cli::parse_from(args);
    init_tracing(cli.verbose, cli.quiet);

    match commands::run(cli.command, cli.quiet).await {
        Ok(code) => code,
        Err(error) => {
            eprintln!("{} {error:#}", ui::red("error:"));
            ExitCode::FAILURE
        }
    }
}

fn init_tracing(verbose: u8, quiet: bool) {
    let default_level = if quiet {
        "error"
    } else {
        match verbose {
            0 => "warn",
            1 => "info",
            2 => "debug",
            _ => "trace",
        }
    };
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(default_level));
    let ansi = std::io::stderr().is_terminal();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .with_target(false)
        .without_time()
        .with_ansi(ansi)
        .init();
}
