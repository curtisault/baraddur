use clap::{ArgAction, Parser};
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::process::ExitCode;

use baraddur::config::{self, ConfigSource};
use baraddur::output::{DisplayConfig, Verbosity};

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

    /// Force non-TTY (append-only) output even on a terminal
    #[arg(long)]
    no_tty: bool,

    /// Don't clear screen between runs
    #[arg(long)]
    no_clear: bool,

    /// Increase verbosity: -v shows passing output, -vv shows debug events
    #[arg(short = 'v', long, action = ArgAction::Count, conflicts_with = "quiet")]
    verbose: u8,

    /// Only show failures; suppress all other output
    #[arg(short = 'q', long, conflicts_with = "verbose")]
    quiet: bool,
}

impl Cli {
    fn verbosity(&self) -> Verbosity {
        if self.quiet {
            Verbosity::Quiet
        } else {
            match self.verbose {
                0 => Verbosity::Normal,
                1 => Verbosity::Verbose,
                _ => Verbosity::Debug,
            }
        }
    }
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

    let is_tty = !cli.no_tty && std::io::stdout().is_terminal();
    let no_clear = cli.no_clear;
    let verbosity = cli.verbosity();

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
        display_config: DisplayConfig {
            is_tty,
            no_clear,
            verbosity,
        },
    };

    match app.run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("baraddur: {e:#}");
            ExitCode::from(1)
        }
    }
}
