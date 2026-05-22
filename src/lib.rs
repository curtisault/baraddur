#![forbid(unsafe_code)]

pub mod config;
pub mod git;
pub mod output;
pub mod pipeline;
pub mod watcher;

use anyhow::Result;
use std::fmt::Write as _;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::task::JoinHandle;

use crate::output::style::{Theme, should_color};
use crate::output::{BrowseAction, Display, DisplayConfig, PlainDisplay, TtyDisplay};
use crate::pipeline::StepResult;

/// Result of the spawned on_failure hook task. Outer Result is the join
/// result; inner is `run_hook`'s return — `Some(text)` if the hook produced
/// output, `None` if it was suppressed (timeout, non-zero exit, empty stdout).
type HookHandle = JoinHandle<Result<Option<String>>>;

pub struct App {
    pub config: config::Config,
    pub config_path: PathBuf,
    pub root: PathBuf,
    pub display_config: DisplayConfig,
}

/// Options for a one-shot `App::run_once` invocation.
#[derive(Debug, Default, Clone)]
pub struct RunOnceOptions {
    /// Skip the configured `[on_failure]` hook even if enabled.
    pub no_hook: bool,

    /// Initial trigger paths (relative to `App.root`). When `Some`, the
    /// pipeline filters steps by `if_changed` and substitutes `{files}`
    /// exactly as watch-mode does on a file-change. `None` runs every step.
    pub initial_trigger: Option<Vec<PathBuf>>,
}

impl App {
    /// Convenience wrapper that uses Ctrl+C as the shutdown signal.
    /// Production entry point.
    pub async fn run(self) -> Result<()> {
        let stop = async {
            let _ = tokio::signal::ctrl_c().await;
        };
        self.run_until(stop).await
    }

    /// Constructs the display the watch loop uses based on `display_config`.
    /// `TtyDisplay` when attached to a terminal, `PlainDisplay` otherwise.
    fn build_display(&self) -> Box<dyn Display> {
        let dc = &self.display_config;
        let color = should_color(dc.is_tty);
        if dc.is_tty {
            Box::new(TtyDisplay::new(
                Theme::new(color),
                dc.verbosity,
                dc.no_clear,
            ))
        } else {
            Box::new(PlainDisplay::new(Theme::new(color), dc.verbosity))
        }
    }

    /// Runs the configured pipeline exactly once and returns whether every
    /// step passed. Uses `PlainDisplay` unconditionally (no spinner, no browse
    /// mode) so output is scriptable. Writes `.baraddur/last-run.log` and,
    /// unless `opts.no_hook`, runs the configured `[on_failure]` hook
    /// synchronously when the run fails.
    pub async fn run_once(self, opts: RunOnceOptions) -> Result<bool> {
        let dc = &self.display_config;
        let color = should_color(dc.is_tty);
        let mut display: Box<dyn Display> =
            Box::new(PlainDisplay::new(Theme::new(color), dc.verbosity));

        display.banner(&self.root, &self.config_path, self.config.steps.len());

        if let Some(paths) = opts.initial_trigger.as_deref() {
            display.set_trigger(paths);
        }

        let results = pipeline::run_pipeline(
            &self.config,
            &self.root,
            display.as_mut(),
            None,
            opts.initial_trigger.as_deref(),
            None,
        )
        .await?;

        write_run_log(&self.root, &results);

        let success = results.iter().all(|r| r.success);

        if !success && !opts.no_hook && self.config.on_failure.enabled {
            display.hook_started();
            let combined = pipeline::combine_failed_output(&results);
            match pipeline::run_hook(&self.config.on_failure, &self.root, &combined).await {
                Ok(Some(text)) => display.hook_output(&text),
                _ => display.hook_finished(),
            }
        }

        Ok(success)
    }

    /// Runs the watch loop until either `stop` resolves or the watcher dies.
    /// Exposed so tests can drive the loop without sending SIGINT to the test runner.
    // `last_failed_steps` is initialized to None as a placeholder; the first
    // read only happens after at least one Completed has overwritten it, so
    // the initial None is correctly diagnosed as unused. The placeholder is
    // required for control-flow init.
    #[allow(unused_assignments)]
    pub async fn run_until<F>(self, stop: F) -> Result<()>
    where
        F: Future<Output = ()>,
    {
        tokio::pin!(stop);
        let dc = &self.display_config;

        let spinner_interval = if dc.is_tty {
            Some(Duration::from_millis(80))
        } else {
            None
        };

        let mut display: Box<dyn Display> = self.build_display();

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

        // In-flight on_failure hook task, set after a failing run and cleared
        // when it completes, the user saves a file, or shutdown begins.
        let mut hook_handle: Option<HookHandle> = None;

        // Relative paths of files that triggered the current pipeline run.
        // `None` on the initial run; set on each `FileChange` and consumed by
        // the next `run_pipeline` invocation for path-based step filtering.
        let mut trigger_paths: Option<Vec<PathBuf>> = None;

        // Names of steps that failed in the most recent completed run.
        // Captured on `Completed`; consumed by the browse-mode `f` key, which
        // narrows the next run to just these steps via `rerun_filter`. `None`
        // until at least one run has completed.
        let mut last_failed_steps: Option<Vec<String>> = None;

        // Name filter for the next pipeline run. `Some(names)` runs only
        // matching steps (browse-mode `f`); `None` runs everything that
        // path-filtering left. Reset to `None` after each run completes.
        let mut rerun_filter: Option<Vec<String>> = None;

        'main: loop {
            let outcome = tokio::select! {
                biased;

                _ = &mut stop => RunOutcome::Shutdown,

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
                    trigger_paths.as_deref(),
                    rerun_filter.as_deref(),
                ) => RunOutcome::Completed(result),
            };

            // Pipeline future is dropped here — all child processes killed,
            // all borrows released. `display` is available again.
            match outcome {
                RunOutcome::Completed(result) => {
                    let results = result?;
                    write_run_log(&self.root, &results);

                    // Remember which steps failed so the browse-mode `f` key
                    // can rerun them; reset the filter so the next iteration
                    // doesn't accidentally re-apply it.
                    last_failed_steps = Some(
                        results
                            .iter()
                            .filter(|r| !r.success)
                            .map(|r| r.name.clone())
                            .collect(),
                    );
                    rerun_filter = None;

                    // Cancel any leftover hook from a prior run; spawn a new
                    // one if this run failed and the user has on_failure enabled.
                    if let Some(h) = hook_handle.take() {
                        h.abort();
                        display.hook_finished();
                    }
                    if self.config.on_failure.enabled && results.iter().any(|r| !r.success) {
                        let cfg = self.config.on_failure.clone();
                        let cwd = self.root.clone();
                        let combined = pipeline::combine_failed_output(&results);
                        hook_handle = Some(tokio::spawn(async move {
                            pipeline::run_hook(&cfg, &cwd, &combined).await
                        }));
                        display.hook_started();
                    }
                }
                RunOutcome::FileChange(paths) => {
                    while rx.try_recv().is_ok() {}
                    if dc.verbosity == output::Verbosity::Debug {
                        eprintln!("[debug] file change — restarting pipeline");
                        for p in &paths {
                            eprintln!("[debug]   triggered by: {}", p.display());
                        }
                    }
                    if let Some(h) = hook_handle.take() {
                        h.abort();
                        display.hook_finished();
                    }
                    let rel = rel_paths(&paths, &self.root);
                    display.run_cancelled();
                    display.set_trigger(&rel);
                    trigger_paths = Some(rel);
                    continue;
                }
                RunOutcome::Shutdown => {
                    if let Some(h) = hook_handle.take() {
                        h.abort();
                        display.hook_finished();
                    }
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

                        _ = &mut stop => {
                            if let Some(h) = hook_handle.take() { h.abort(); display.hook_finished(); }
                            display.exit_browse_mode();
                            return self.shutdown();
                        }

                        maybe = rx.recv() => {
                            if let Some(h) = hook_handle.take() { h.abort(); display.hook_finished(); }
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
                                    let rel = rel_paths(&paths, &self.root);
                                    display.set_trigger(&rel);
                                    trigger_paths = Some(rel);
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
                                        if let Some(h) = hook_handle.take() { h.abort(); display.hook_finished(); }
                                        display.exit_browse_mode();
                                        return self.shutdown();
                                    }
                                    BrowseAction::Rerun => {
                                        if let Some(h) = hook_handle.take() { h.abort(); display.hook_finished(); }
                                        display.exit_browse_mode();
                                        trigger_paths = None;
                                        rerun_filter = None;
                                        continue 'main;
                                    }
                                    BrowseAction::RerunFailed => {
                                        if let Some(h) = hook_handle.take() { h.abort(); display.hook_finished(); }
                                        display.exit_browse_mode();
                                        trigger_paths = None;
                                        rerun_filter = last_failed_steps.clone();
                                        continue 'main;
                                    }
                                },
                                None => {
                                    display.exit_browse_mode();
                                    eprintln!("baraddur: keyboard reader stopped unexpectedly. exiting.");
                                    return Ok(());
                                }
                            }
                        }

                        // Hook task completed: forward its output (if any) into
                        // the browse-mode footer. Never breaks the loop — user
                        // stays in browse until a file change or `q`.
                        res = await_hook(&mut hook_handle), if hook_handle.is_some() => {
                            hook_handle = None;
                            match res {
                                Ok(Ok(Some(text))) => display.hook_output(&text),
                                _ => display.hook_finished(),
                            }
                        }
                    }
                }
            }

            // Plain idle wait: non-TTY mode only (TTY mode loops inside browse above).
            // Loops so that an in-flight hook completion doesn't fall through —
            // we keep waiting for the next file-change or shutdown after it fires.
            'idle: loop {
                tokio::select! {
                    biased;

                    _ = &mut stop => {
                        if let Some(h) = hook_handle.take() { h.abort(); }
                        return self.shutdown();
                    }

                    maybe = rx.recv() => {
                        if let Some(h) = hook_handle.take() { h.abort(); }
                        match maybe {
                            Some(paths) => {
                                while rx.try_recv().is_ok() {}
                                if dc.verbosity == output::Verbosity::Debug {
                                    eprintln!("[debug] file change — triggering pipeline");
                                    for p in &paths {
                                        eprintln!("[debug]   triggered by: {}", p.display());
                                    }
                                }
                                let rel = rel_paths(&paths, &self.root);
                                display.set_trigger(&rel);
                                trigger_paths = Some(rel);
                                break 'idle; // fall through to loop top → rerun pipeline
                            }
                            None => {
                                eprintln!("baraddur: file watcher stopped unexpectedly. exiting.");
                                return Ok(());
                            }
                        }
                    }

                    res = await_hook(&mut hook_handle), if hook_handle.is_some() => {
                        hook_handle = None;
                        match res {
                            Ok(Ok(Some(text))) => display.hook_output(&text),
                            _ => display.hook_finished(),
                        }
                        // continue idle loop — wait for next event
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

/// Awaits the on_failure hook task if one is in flight; otherwise pends
/// forever. Used as a select! arm so the main loop can react to hook
/// completion without blocking when no hook is active.
async fn await_hook(
    handle: &mut Option<HookHandle>,
) -> std::result::Result<Result<Option<String>>, tokio::task::JoinError> {
    match handle.as_mut() {
        Some(h) => h.await,
        None => std::future::pending().await,
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
