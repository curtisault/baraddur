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

pub type WatchEvent = Vec<PathBuf>;

/// Starts the file watcher on a dedicated OS thread.
/// Many FS events collapse into one `WatchEvent`; the runner drains the channel after each run.
pub fn start(cfg: WatchConfig) -> Result<mpsc::Receiver<WatchEvent>> {
    let (tx, rx) = mpsc::channel::<WatchEvent>(8);

    std::thread::Builder::new()
        .name("baraddur-watcher".into())
        .spawn(move || watcher_thread(cfg, tx))
        .context("spawning watcher thread")?;

    Ok(rx)
}

fn watcher_thread(cfg: WatchConfig, tx: mpsc::Sender<WatchEvent>) {
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

        let paths: Vec<PathBuf> = events
            .into_iter()
            .filter(|ev| matches_filters(&ev.path, &cfg))
            .map(|ev| ev.path)
            .collect();

        if !paths.is_empty() && tx.blocking_send(paths).is_err() {
            // Receiver dropped — app is shutting down.
            break;
        }
    }
}

fn matches_filters(path: &std::path::Path, cfg: &WatchConfig) -> bool {
    let rel = path.strip_prefix(&cfg.root).unwrap_or(path);

    for ignored in &cfg.ignore {
        if ignored.contains('/') {
            // Slash entries match by relative path prefix; plain names match any component.
            if rel.starts_with(ignored.as_str()) {
                return false;
            }
        } else {
            // Component entry: match any segment of the path.
            if path.components().any(|c| c.as_os_str() == ignored.as_str()) {
                return false;
            }
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

    fn cfg_with_root(root: &str, exts: &[&str], ignore: &[&str]) -> WatchConfig {
        WatchConfig {
            root: PathBuf::from(root),
            debounce: Duration::from_millis(1000),
            extensions: exts.iter().map(|s| s.to_string()).collect(),
            ignore: ignore.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn path_style_ignore_rejects_specific_file() {
        let c = cfg_with_root("/project", &["ex"], &["lib/tss_web/storybook.ex"]);
        assert!(!matches_filters(
            std::path::Path::new("/project/lib/tss_web/storybook.ex"),
            &c
        ));
    }

    #[test]
    fn path_style_ignore_allows_sibling_file() {
        let c = cfg_with_root("/project", &["ex"], &["lib/tss_web/storybook.ex"]);
        assert!(matches_filters(
            std::path::Path::new("/project/lib/tss_web/other.ex"),
            &c
        ));
    }

    #[test]
    fn path_style_ignore_rejects_subtree() {
        let c = cfg_with_root("/project", &["ex"], &["lib/generated"]);
        assert!(!matches_filters(
            std::path::Path::new("/project/lib/generated/foo.ex"),
            &c
        ));
    }
}
