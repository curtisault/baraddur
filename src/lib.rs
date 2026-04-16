pub mod config;
pub mod output;
pub mod pipeline;
pub mod watcher;

use anyhow::Result;
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::time::Duration;

use crate::output::{Display, PlainDisplay, TtyDisplay};
use crate::pipeline::StepResult;

pub struct App {
    pub config: config::Config,
    pub config_path: PathBuf,
    pub root: PathBuf,
    pub no_tty: bool,
}

impl App {
    pub async fn run(self) -> Result<()> {
        let is_tty = !self.no_tty && std::io::stdout().is_terminal();

        let mut display: Box<dyn Display> = if is_tty {
            Box::new(TtyDisplay::new())
        } else {
            Box::new(PlainDisplay)
        };

        // Only print the banner in non-TTY mode (TTY mode clears the screen
        // on each run, so a startup banner would vanish immediately).
        if !is_tty {
            eprintln!(
                "baraddur: watching {}\n          (config: {})",
                self.root.display(),
                self.config_path.display(),
            );
        }

        let wcfg = watcher::WatchConfig {
            root: self.root.clone(),
            debounce: Duration::from_millis(self.config.watch.debounce_ms),
            extensions: self.config.watch.extensions.clone(),
            ignore: self.config.watch.ignore.clone(),
        };
        let mut rx = watcher::start(wcfg)?;

        // Main event loop. Each iteration runs the pipeline then idles until
        // a file change. On mid-run file change, select! drops the pipeline
        // future (killing children) and we loop back immediately.
        loop {
            let outcome = tokio::select! {
                biased;

                _ = tokio::signal::ctrl_c() => RunOutcome::Shutdown,

                maybe = rx.recv() => {
                    match maybe {
                        Some(()) => RunOutcome::FileChange,
                        None => RunOutcome::WatcherDied,
                    }
                }

                result = pipeline::run_pipeline(
                    &self.config,
                    &self.root,
                    display.as_mut(),
                ) => RunOutcome::Completed(result),
            };

            // Pipeline future is dropped here — all child processes killed,
            // all borrows released. `display` is available again.
            match outcome {
                RunOutcome::Completed(result) => {
                    result?;
                    // Pipeline finished normally — idle-wait below.
                }
                RunOutcome::FileChange => {
                    // Drain any additional pending triggers.
                    while rx.try_recv().is_ok() {}
                    display.run_cancelled();
                    continue; // restart pipeline immediately
                }
                RunOutcome::Shutdown => {
                    return self.shutdown().await;
                }
                RunOutcome::WatcherDied => {
                    eprintln!("baraddur: file watcher stopped unexpectedly. exiting.");
                    return Ok(());
                }
            }

            // ── Idle: wait for the next file change or Ctrl+C ───────────────
            tokio::select! {
                biased;

                _ = tokio::signal::ctrl_c() => {
                    return self.shutdown().await;
                }

                maybe = rx.recv() => {
                    match maybe {
                        Some(()) => {
                            while rx.try_recv().is_ok() {}
                            // fall through to loop top → rerun pipeline
                        }
                        None => {
                            eprintln!("baraddur: file watcher stopped unexpectedly. exiting.");
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    /// Handles graceful shutdown. Spawns a background task that listens for
    /// a second Ctrl+C and force-exits with code 130.
    async fn shutdown(&self) -> Result<()> {
        eprintln!("\nbaraddur: exiting...");

        // Double-tap handler: a second Ctrl+C force-exits immediately.
        tokio::spawn(async {
            tokio::signal::ctrl_c().await.ok();
            eprintln!("baraddur: force exit.");
            std::process::exit(130);
        });

        Ok(())
    }
}

enum RunOutcome {
    Completed(Result<Vec<StepResult>>),
    FileChange,
    Shutdown,
    WatcherDied,
}
