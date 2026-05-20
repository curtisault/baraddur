pub mod config;
pub mod output;
pub mod pipeline;
pub mod watcher;

use anyhow::Result;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::output::style::{Theme, should_color};
use crate::output::{BrowseAction, Display, DisplayConfig, PlainDisplay, TtyDisplay};
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
            Box::new(TtyDisplay::new(
                Theme::new(color),
                dc.verbosity,
                dc.no_clear,
            ))
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

        // Single long-lived OS thread drains crossterm events into a channel.
        // Per-iteration spawn_blocking readers leaked: their JoinHandles were
        // dropped on file-change cancellation but the threads kept blocking
        // inside event::read(), then stole the next keystroke and exited.
        let mut key_rx = if dc.is_tty {
            Some(spawn_key_reader())
        } else {
            None
        };

        if dc.verbosity == output::Verbosity::Debug {
            eprintln!("[debug] watcher started, running initial pipeline");
        }

        'main: loop {
            let outcome = tokio::select! {
                biased;

                _ = tokio::signal::ctrl_c() => RunOutcome::Shutdown,

                maybe = rx.recv() => {
                    match maybe {
                        Some(paths) => RunOutcome::FileChange(paths),
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
                RunOutcome::FileChange(paths) => {
                    while rx.try_recv().is_ok() {}
                    if dc.verbosity == output::Verbosity::Debug {
                        eprintln!("[debug] file change — restarting pipeline");
                        for p in &paths {
                            eprintln!("[debug]   triggered by: {}", p.display());
                        }
                    }
                    display.run_cancelled();
                    display.set_trigger(&rel_paths(&paths, &self.root));
                    continue;
                }
                RunOutcome::Shutdown => {
                    return self.shutdown();
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
                let key_rx = key_rx
                    .as_mut()
                    .expect("key_rx initialized when dc.is_tty is true");

                // Discard any keystrokes that queued up during the pipeline run.
                while key_rx.try_recv().is_ok() {}

                display.enter_browse_mode();

                loop {
                    tokio::select! {
                        biased;

                        _ = tokio::signal::ctrl_c() => {
                            display.exit_browse_mode();
                            return self.shutdown();
                        }

                        maybe = rx.recv() => {
                            display.exit_browse_mode();
                            match maybe {
                                Some(paths) => {
                                    while rx.try_recv().is_ok() {}
                                    if dc.verbosity == output::Verbosity::Debug {
                                        eprintln!("[debug] file change — triggering pipeline");
                                        for p in &paths {
                                            eprintln!("[debug]   triggered by: {}", p.display());
                                        }
                                    }
                                    display.set_trigger(&rel_paths(&paths, &self.root));
                                    continue 'main;
                                }
                                None => {
                                    eprintln!("baraddur: file watcher stopped unexpectedly. exiting.");
                                    return Ok(());
                                }
                            }
                        }

                        maybe_key = key_rx.recv() => {
                            match maybe_key {
                                Some(key) => match display.handle_key(key) {
                                    BrowseAction::Noop => {}
                                    BrowseAction::Redraw => display.browse_redraw_if_active(),
                                    BrowseAction::Quit => {
                                        display.exit_browse_mode();
                                        return self.shutdown();
                                    }
                                },
                                None => {
                                    display.exit_browse_mode();
                                    eprintln!("baraddur: keyboard reader stopped unexpectedly. exiting.");
                                    return Ok(());
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
                    return self.shutdown();
                }

                maybe = rx.recv() => {
                    match maybe {
                        Some(paths) => {
                            while rx.try_recv().is_ok() {}
                            if dc.verbosity == output::Verbosity::Debug {
                                eprintln!("[debug] file change — triggering pipeline");
                                for p in &paths {
                                    eprintln!("[debug]   triggered by: {}", p.display());
                                }
                            }
                            display.set_trigger(&rel_paths(&paths, &self.root));
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

    fn shutdown(&self) -> Result<()> {
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

/// Spawns a dedicated OS thread that loops on `crossterm::event::read()` and
/// forwards key events onto a channel. One reader per process — the channel
/// outlives any single browse-mode session, so file-change cancellation no
/// longer leaks blocked readers that steal subsequent keystrokes.
///
/// The thread exits when the receiver is dropped or `event::read()` errors.
fn spawn_key_reader() -> tokio::sync::mpsc::Receiver<crossterm::event::KeyEvent> {
    let (tx, rx) = tokio::sync::mpsc::channel(16);
    let _ = std::thread::Builder::new()
        .name("baraddur-keys".into())
        .spawn(move || {
            loop {
                match crossterm::event::read() {
                    Ok(crossterm::event::Event::Key(k)) => {
                        if tx.blocking_send(k).is_err() {
                            return;
                        }
                    }
                    Ok(_) => {}
                    Err(_) => return,
                }
            }
        });
    rx
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
        let _ = writeln!(
            content,
            "═══ {} ({}) ═══",
            r.name,
            if r.success { "pass" } else { "FAIL" }
        );
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

fn rel_paths(paths: &[PathBuf], root: &Path) -> Vec<PathBuf> {
    paths
        .iter()
        .map(|p| p.strip_prefix(root).unwrap_or(p).to_path_buf())
        .collect()
}

enum RunOutcome {
    Completed(Result<Vec<StepResult>>),
    FileChange(Vec<PathBuf>),
    Shutdown,
    WatcherDied,
}
