#![forbid(unsafe_code)]

use clap::{ArgAction, Parser, Subcommand};
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::process::ExitCode;

use baraddur::RunOnceOptions;
use baraddur::config::{self, ConfigSource};
use baraddur::output::{DisplayConfig, OutputFormat, Verbosity};

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

    /// Run only the steps in the named profile (defined under `[profiles]`
    /// in the config). Applies to `watch`, `check`, and `gate`.
    #[arg(short = 'p', long, global = true, value_name = "NAME")]
    profile: Option<String>,

    /// Directory to watch [default: directory containing the discovered config]
    #[arg(short = 'w', long)]
    watch_dir: Option<PathBuf>,

    /// Force non-TTY (append-only) output even on a terminal
    #[arg(long, global = true, conflicts_with = "format")]
    no_tty: bool,

    /// Output format. `auto` (default) renders for the terminal; `json` emits
    /// one NDJSON event per line on stdout (see `docs/json-events.md`).
    /// Implies non-interactive behavior — no spinner, no browse mode.
    #[arg(long, global = true, value_enum, default_value_t = OutputFormatArg::Auto, value_name = "FORMAT")]
    format: OutputFormatArg,

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

#[derive(Copy, Clone, Debug, PartialEq, Eq, clap::ValueEnum)]
enum OutputFormatArg {
    Auto,
    Json,
}

impl From<OutputFormatArg> for OutputFormat {
    fn from(v: OutputFormatArg) -> Self {
        match v {
            OutputFormatArg::Auto => OutputFormat::Auto,
            OutputFormatArg::Json => OutputFormat::Json,
        }
    }
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

    let format: OutputFormat = cli.format.into();
    // JSON mode forces non-interactive rendering — no spinner, no browse
    // mode. Treating is_tty as false here keeps the run loop's existing
    // gating untouched.
    let is_tty = format == OutputFormat::Auto && !cli.no_tty && std::io::stdout().is_terminal();
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

    let mut config = loaded.config;
    if let Some(name) = cli.profile.as_deref() {
        baraddur::apply_profile(&mut config, name)
            .map_err(|e| BuildAppError::Config(format!("{e}")))?;
    }

    Ok(baraddur::App {
        config,
        config_path: loaded.config_path,
        root,
        display_config: DisplayConfig {
            is_tty,
            no_clear,
            verbosity,
            format,
        },
        profile: cli.profile.clone(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// `run_init` writes a `.baraddur.toml` into the current working
    /// directory. The test temporarily relocates cwd into a tempdir so the
    /// real project file is never touched. cwd is restored before any
    /// assertions so a failure here doesn't poison sibling tests.
    #[test]
    fn run_init_writes_starter_config_into_cwd() {
        let td = TempDir::new().unwrap();
        let original = std::env::current_dir().unwrap();
        std::env::set_current_dir(td.path()).unwrap();

        let _exit = run_init();

        std::env::set_current_dir(&original).unwrap();

        let written = td.path().join(".baraddur.toml");
        assert!(
            written.exists(),
            "run_init should write a starter config at {}",
            written.display()
        );
        assert!(std::fs::metadata(&written).unwrap().len() > 0);
    }
}
