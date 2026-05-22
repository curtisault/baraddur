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

        /// Restrict the pipeline to files currently staged for commit
        /// (`git diff --cached`). Mutually exclusive with `--since`.
        #[arg(long, conflicts_with = "since")]
        staged: bool,

        /// Restrict the pipeline to files that changed since the given
        /// git ref (`git diff <ref>...HEAD`, plus untracked-not-ignored).
        /// Mutually exclusive with `--staged`.
        #[arg(long, value_name = "REF", conflicts_with = "staged")]
        since: Option<String>,
    },

    /// Run the pipeline; on success, exec the wrapped command.
    ///
    /// Everything after `gate` is captured as the wrapped command, so no
    /// `--` separator is needed: `baraddur gate git push origin main`.
    /// Exit code is the wrapped command's on success, 1 on pipeline failure,
    /// 2 on config error.
    Gate {
        /// Skip the configured `[on_failure]` hook even if enabled.
        #[arg(long)]
        no_hook: bool,

        /// Restrict the pipeline to files currently staged for commit.
        #[arg(long, conflicts_with = "since")]
        staged: bool,

        /// Restrict the pipeline to files changed since the given git ref.
        #[arg(long, value_name = "REF", conflicts_with = "staged")]
        since: Option<String>,

        /// The command (and arguments) to execute on pipeline success.
        #[arg(trailing_var_arg = true, required = true, num_args = 1..)]
        wrapped: Vec<String>,
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
        Some(Command::Check {
            no_hook,
            staged,
            since,
        }) => {
            let initial_trigger = match resolve_trigger(&app.root, staged, since.as_deref()).await {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("baraddur: {e:#}");
                    return ExitCode::from(1);
                }
            };
            let opts = RunOnceOptions {
                no_hook,
                initial_trigger,
            };
            match app.run_once(opts).await {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::from(1),
                Err(e) => {
                    eprintln!("baraddur: {e:#}");
                    ExitCode::from(1)
                }
            }
        }
        Some(Command::Gate {
            no_hook,
            staged,
            since,
            wrapped,
        }) => {
            let initial_trigger = match resolve_trigger(&app.root, staged, since.as_deref()).await {
                Ok(t) => t,
                Err(e) => {
                    eprintln!("baraddur: {e:#}");
                    return ExitCode::from(1);
                }
            };
            let opts = RunOnceOptions {
                no_hook,
                initial_trigger,
            };
            match app.gate(wrapped, opts).await {
                Ok(code) => ExitCode::from(u8::try_from(code).unwrap_or(1)),
                Err(e) => {
                    eprintln!("baraddur: {e:#}");
                    ExitCode::from(1)
                }
            }
        }
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

/// Resolves `--staged` / `--since` into a list of paths relative to
/// `app_root`. Returns `Ok(None)` when neither flag is set, so the pipeline
/// runs unfiltered.
async fn resolve_trigger(
    app_root: &std::path::Path,
    staged: bool,
    since: Option<&str>,
) -> anyhow::Result<Option<Vec<PathBuf>>> {
    if !staged && since.is_none() {
        return Ok(None);
    }

    use anyhow::Context as _;
    let repo = baraddur::git::repo_root(app_root).await?;
    let app_root_canon = app_root
        .canonicalize()
        .with_context(|| format!("canonicalizing {}", app_root.display()))?;

    let raw = if staged {
        baraddur::git::staged_paths(&repo).await?
    } else {
        // `conflicts_with` keeps these mutually exclusive; the else branch
        // is reached only when `since` is `Some`.
        baraddur::git::diff_since(&repo, since.expect("since is Some")).await?
    };

    Ok(Some(baraddur::git::rebase_for_app(
        &repo,
        &app_root_canon,
        &raw,
    )))
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
