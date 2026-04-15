use anyhow::{Context, Result};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use std::path::PathBuf;
use std::time::Duration;
use tokio::sync::mpsc;

pub struct WatchConfig {
    pub root: PathBuf,
    pub debounce: Duration,
    pub extensions: Vec<String>,
    pub ignore: Vec<String>,
}

/// Starts the file watcher on a dedicated OS thread and returns an async receiver
/// that yields `()` whenever a relevant debounced batch of file events arrives.
///
/// The channel is small by design — many events collapsing into one `()` is
/// the intended behavior. The runner drains the channel after each pipeline run.
pub fn start(cfg: WatchConfig) -> Result<mpsc::Receiver<()>> {
    let (tx, rx) = mpsc::channel::<()>(8);

    std::thread::Builder::new()
        .name("baraddur-watcher".into())
        .spawn(move || watcher_thread(cfg, tx))
        .context("spawning watcher thread")?;

    Ok(rx)
}

fn watcher_thread(cfg: WatchConfig, tx: mpsc::Sender<()>) {
    let (sync_tx, sync_rx) = std::sync::mpsc::channel();

    let mut debouncer = match new_debouncer(cfg.debounce, sync_tx) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("baraddur: failed to start debouncer: {e}");
            return;
        }
    };

    if let Err(e) = debouncer
        .watcher()
        .watch(&cfg.root, RecursiveMode::Recursive)
    {
        eprintln!("baraddur: failed to watch {}: {e}", cfg.root.display());
        return;
    }

    for batch in sync_rx {
        let events = match batch {
            Ok(events) => events,
            Err(err) => {
                eprintln!("baraddur: watch error: {err}");
                continue;
            }
        };

        let relevant = events.iter().any(|ev| matches_filters(&ev.path, &cfg));

        if relevant && tx.blocking_send(()).is_err() {
            // Receiver dropped — app is shutting down.
            break;
        }
    }
}

fn matches_filters(path: &std::path::Path, cfg: &WatchConfig) -> bool {
    for ignored in &cfg.ignore {
        if path.components().any(|c| c.as_os_str() == ignored.as_str()) {
            return false;
        }
    }

    if cfg.extensions.is_empty() {
        return true;
    }

    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => cfg.extensions.iter().any(|want| want == ext),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(exts: &[&str], ignore: &[&str]) -> WatchConfig {
        WatchConfig {
            root: PathBuf::from("."),
            debounce: Duration::from_millis(1000),
            extensions: exts.iter().map(|s| s.to_string()).collect(),
            ignore: ignore.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn matches_wanted_extension() {
        assert!(matches_filters(
            std::path::Path::new("src/foo.rs"),
            &cfg(&["rs"], &[])
        ));
    }

    #[test]
    fn rejects_unwanted_extension() {
        assert!(!matches_filters(
            std::path::Path::new("README.md"),
            &cfg(&["rs"], &[])
        ));
    }

    #[test]
    fn rejects_ignored_dir() {
        assert!(!matches_filters(
            std::path::Path::new("target/debug/foo.rs"),
            &cfg(&["rs"], &["target"])
        ));
    }

    #[test]
    fn empty_extensions_matches_all() {
        assert!(matches_filters(
            std::path::Path::new("anything"),
            &cfg(&[], &[])
        ));
    }

    #[test]
    fn ignores_dir_anywhere_in_path() {
        assert!(!matches_filters(
            std::path::Path::new("a/b/_build/c/foo.ex"),
            &cfg(&["ex"], &["_build"])
        ));
    }
}
