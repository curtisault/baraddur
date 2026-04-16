pub mod config;
pub mod output;
pub mod pipeline;
pub mod watcher;

use anyhow::Result;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::output::{BrowseAction, Display, DisplayConfig, PlainDisplay, TtyDisplay};
use crate::output::style::{should_color, Theme};
use crate::pipeline::StepResult;

pub struct App {
    pub config: config::Config,
    pub config_path: PathBuf,
    pub root: PathBuf,
    pub display_config: DisplayConfig,
}

impl App {
    pub async fn run(self) -> Result<()> {
        let dc = &self.display_config;
        let color = should_color(dc.is_tty);

        let spinner_interval = if dc.is_tty {
            Some(Duration::from_millis(80))
        } else {
            None
        };

        let mut display: Box<dyn Display> = if dc.is_tty {
            Box::new(TtyDisplay::new(Theme::new(color), dc.verbosity, dc.no_clear))
        } else {
            Box::new(PlainDisplay::new(Theme::new(color), dc.verbosity))
        };

        // Show startup banner once before the first run.
        display.banner(&self.root, &self.config_path, self.config.steps.len());

        let wcfg = watcher::WatchConfig {
            root: self.root.clone(),
            debounce: Duration::from_millis(self.config.watch.debounce_ms),
            extensions: self.config.watch.extensions.clone(),
            ignore: self.config.watch.ignore.clone(),
        };
        let mut rx = watcher::start(wcfg)?;

        if dc.verbosity == output::Verbosity::Debug {
            eprintln!("[debug] watcher started, running initial pipeline");
        }

        'main: loop {
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
                    spinner_interval,
                ) => RunOutcome::Completed(result),
            };

            // Pipeline future is dropped here — all child processes killed,
            // all borrows released. `display` is available again.
            match outcome {
                RunOutcome::Completed(result) => {
                    let results = result?;
                    write_run_log(&self.root, &results);
                }
                RunOutcome::FileChange => {
                    while rx.try_recv().is_ok() {}
                    if dc.verbosity == output::Verbosity::Debug {
                        eprintln!("[debug] file change — restarting pipeline");
                    }
                    display.run_cancelled();
                    continue;
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
            if dc.verbosity == output::Verbosity::Debug {
                eprintln!("[debug] idle — waiting for file change");
            }

            // In TTY mode, enter interactive browse mode so the user can
            // navigate steps and expand output with vim-style keybindings.
            if dc.is_tty {
                display.enter_browse_mode();

                loop {
                    let key_fut = next_key_event();
                    tokio::pin!(key_fut);

                    tokio::select! {
                        biased;

                        _ = tokio::signal::ctrl_c() => {
                            display.exit_browse_mode();
                            return self.shutdown().await;
                        }

                        maybe = rx.recv() => {
                            display.exit_browse_mode();
                            match maybe {
                                Some(()) => {
                                    while rx.try_recv().is_ok() {}
                                    if dc.verbosity == output::Verbosity::Debug {
                                        eprintln!("[debug] file change — triggering pipeline");
                                    }
                                    continue 'main;
                                }
                                None => {
                                    eprintln!("baraddur: file watcher stopped unexpectedly. exiting.");
                                    return Ok(());
                                }
                            }
                        }

                        maybe_key = &mut key_fut => {
                            if let Some(key) = maybe_key {
                                match display.handle_key(key) {
                                    BrowseAction::Noop => {}
                                    BrowseAction::Redraw => display.browse_redraw_if_active(),
                                    BrowseAction::Quit => {
                                        display.exit_browse_mode();
                                        return self.shutdown().await;
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Plain idle wait: non-TTY mode only (TTY mode loops inside browse above).
            tokio::select! {
                biased;

                _ = tokio::signal::ctrl_c() => {
                    return self.shutdown().await;
                }

                maybe = rx.recv() => {
                    match maybe {
                        Some(()) => {
                            while rx.try_recv().is_ok() {}
                            if dc.verbosity == output::Verbosity::Debug {
                                eprintln!("[debug] file change — triggering pipeline");
                            }
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

/// Reads the next keyboard event from stdin without blocking the async executor.
/// Resize events, mouse events, and other non-key events are silently skipped.
/// Returns `None` only on terminal read error.
async fn next_key_event() -> Option<crossterm::event::KeyEvent> {
    tokio::task::spawn_blocking(|| loop {
        match crossterm::event::read() {
            Ok(crossterm::event::Event::Key(k)) => return Some(k),
            Ok(_) => continue,
            Err(_) => return None,
        }
    })
    .await
    .unwrap_or(None)
}

/// Writes all step output for the last run to `.baraddur/last-run.log`.
/// Silently no-ops if the directory cannot be created or the file cannot be written.
fn write_run_log(root: &Path, results: &[StepResult]) {
    let log_dir = root.join(".baraddur");
    if std::fs::create_dir_all(&log_dir).is_err() {
        return;
    }

    let mut content = String::new();
    for r in results {
        content.push_str(&format!(
            "═══ {} ({}) ═══\n",
            r.name,
            if r.success { "pass" } else { "FAIL" }
        ));
        if !r.stdout.is_empty() {
            content.push_str(&r.stdout);
            if !r.stdout.ends_with('\n') {
                content.push('\n');
            }
        }
        if !r.stderr.is_empty() {
            content.push_str("--- stderr ---\n");
            content.push_str(&r.stderr);
            if !r.stderr.ends_with('\n') {
                content.push('\n');
            }
        }
        content.push('\n');
    }

    let _ = std::fs::write(log_dir.join("last-run.log"), &content);
}

enum RunOutcome {
    Completed(Result<Vec<StepResult>>),
    FileChange,
    Shutdown,
    WatcherDied,
}
