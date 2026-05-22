#![forbid(unsafe_code)]

use clap::{ArgAction, Parser, Subcommand};
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::process::ExitCode;

use baraddur::RunOnceOptions;
use baraddur::config::{self, ConfigSource};
use baraddur::output::{DisplayConfig, Verbosity};

#[derive(Parser, Debug)]
#[command(
    name = "baraddur",
    version,
    about = "Project-agnostic file watcher that surfaces issues before CI"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// Config file path (disables walk-up discovery)
    #[arg(short = 'c', long, global = true)]
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

#[derive(Subcommand, Debug)]
enum Command {
    /// Scaffold a starter `.baraddur.toml` in the current directory.
    Init,

    /// Run the configured pipeline exactly once and exit.
    ///
    /// Exit codes: 0 on full pass, 1 on any step failure, 2 on config error.
    /// Output uses the plain (non-TTY) renderer regardless of where stdout
    /// goes, so the output is scriptable.
    Check {
        /// Skip the configured `[on_failure]` hook even if enabled.
        #[arg(long)]
        no_hook: bool,
    },
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

    if let Some(Command::Init) = cli.command {
        return run_init();
    }

    let app = match build_app(&cli) {
        Ok(app) => app,
        Err(BuildAppError::Config(msg)) => {
            eprintln!("baraddur: {msg}");
            return ExitCode::from(2);
        }
        Err(BuildAppError::Other(msg)) => {
            eprintln!("baraddur: {msg}");
            return ExitCode::from(1);
        }
    };

    match cli.command {
        Some(Command::Check { no_hook }) => match app.run_once(RunOnceOptions { no_hook }).await {
            Ok(true) => ExitCode::SUCCESS,
            Ok(false) => ExitCode::from(1),
            Err(e) => {
                eprintln!("baraddur: {e:#}");
                ExitCode::from(1)
            }
        },
        Some(Command::Init) => unreachable!("handled above"),
        None => match app.run().await {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("baraddur: {e:#}");
                ExitCode::from(1)
            }
        },
    }
}

enum BuildAppError {
    Config(String),
    Other(String),
}

fn build_app(cli: &Cli) -> Result<baraddur::App, BuildAppError> {
    let loaded =
        config::load(cli.config.as_deref()).map_err(|e| BuildAppError::Config(format!("{e}")))?;

    let is_tty = !cli.no_tty && std::io::stdout().is_terminal();
    let no_clear = cli.no_clear;
    let verbosity = cli.verbosity();

    let root = match &cli.watch_dir {
        Some(p) => p.clone(),
        None => match loaded.source {
            ConfigSource::WalkUp => loaded.config_dir.clone(),
            ConfigSource::CliOverride | ConfigSource::Global => std::env::current_dir()
                .map_err(|e| BuildAppError::Other(format!("getting current directory: {e}")))?,
        },
    };

    Ok(baraddur::App {
        config: loaded.config,
        config_path: loaded.config_path,
        root,
        display_config: DisplayConfig {
            is_tty,
            no_clear,
            verbosity,
        },
    })
}

fn run_init() -> ExitCode {
    let cwd = match std::env::current_dir() {
        Ok(p) => p,
        Err(e) => {
            eprintln!("baraddur: getting current directory: {e}");
            return ExitCode::from(1);
        }
    };
    match config::init(&cwd) {
        Ok(path) => {
            println!("created {}", path.display());
            ExitCode::SUCCESS
        }
        Err(e) => {
            eprintln!("baraddur: {e:#}");
            ExitCode::from(1)
        }
    }
}
