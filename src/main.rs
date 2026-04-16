use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use baraddur::config::{self, ConfigSource};

#[derive(Parser, Debug)]
#[command(
    name = "baraddur",
    version,
    about = "Project-agnostic file watcher that surfaces issues before CI"
)]
struct Cli {
    /// Config file path (disables walk-up discovery)
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Directory to watch [default: directory containing the discovered config]
    #[arg(short = 'w', long)]
    watch_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let loaded = match config::load(cli.config.as_deref()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("baraddur: {e}");
            return ExitCode::from(2);
        }
    };

    let root = match cli.watch_dir {
        Some(p) => p,
        None => match loaded.source {
            ConfigSource::WalkUp => loaded.config_dir.clone(),
            ConfigSource::CliOverride | ConfigSource::Global => match std::env::current_dir() {
                Ok(cwd) => cwd,
                Err(e) => {
                    eprintln!("baraddur: getting current directory: {e}");
                    return ExitCode::from(1);
                }
            },
        },
    };

    let app = baraddur::App {
        config: loaded.config,
        config_path: loaded.config_path,
        root,
    };

    match app.run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("baraddur: {e:#}");
            ExitCode::from(1)
        }
    }
}
