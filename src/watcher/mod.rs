use anyhow::{Context, Result};
use notify::{EventKind, RecursiveMode, Watcher};
use std::path::PathBuf;
use std::sync::mpsc::RecvTimeoutError;
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
    let (raw_tx, raw_rx) = std::sync::mpsc::channel();

    // Raw notify rather than a debouncer: we need the per-event `EventKind` to
    // drop read/open (`Access`) events. notify-debouncer-mini collapses every
    // kind to `Any`, so a build that merely *reads* every source file looked
    // identical to a write and self-triggered an endless restart loop on
    // Linux/inotify (macOS/FSEvents never reports opens). We debounce here.
    let mut watcher = match notify::recommended_watcher(move |res| {
        let _ = raw_tx.send(res);
    }) {
        Ok(w) => w,
        Err(e) => {
            eprintln!("baraddur: failed to start watcher: {e}");
            return;
        }
    };

    if let Err(e) = watcher.watch(&cfg.root, RecursiveMode::Recursive) {
        eprintln!("baraddur: failed to watch {}: {e}", cfg.root.display());
        return;
    }

    // Accumulate matching mutation paths; flush them as one batch after a quiet
    // gap of `cfg.debounce`. While nothing is pending we block indefinitely.
    let mut pending: Vec<PathBuf> = Vec::new();

    loop {
        let received = if pending.is_empty() {
            raw_rx.recv().map_err(|_| RecvTimeoutError::Disconnected)
        } else {
            match raw_rx.recv_timeout(cfg.debounce) {
                Ok(ev) => Ok(ev),
                Err(RecvTimeoutError::Timeout) => {
                    let batch = std::mem::take(&mut pending);
                    if tx.blocking_send(batch).is_err() {
                        return; // receiver dropped — app is shutting down
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => Err(RecvTimeoutError::Disconnected),
            }
        };

        match received {
            Ok(Ok(event)) => {
                if !is_mutation(&event.kind) {
                    continue;
                }
                for path in event.paths {
                    if matches_filters(&path, &cfg) && !pending.contains(&path) {
                        pending.push(path);
                    }
                }
            }
            Ok(Err(e)) => eprintln!("baraddur: watch error: {e}"),
            Err(_) => return, // watcher dropped / channel closed
        }
    }
}

/// Returns `false` for read/open (`Access`) events — those are not changes we
/// should rebuild on. Everything else (Create/Modify/Remove/Rename, plus the
/// generic `Any`/`Other` that fallback backends emit) counts as a mutation so
/// non-inotify platforms keep triggering normally.
fn is_mutation(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(_))
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
            extensions: exts.iter().map(|s| (*s).into()).collect(),
            ignore: ignore.iter().map(|s| (*s).into()).collect(),
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
            extensions: exts.iter().map(|s| (*s).into()).collect(),
            ignore: ignore.iter().map(|s| (*s).into()).collect(),
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

    #[test]
    fn access_events_are_not_mutations() {
        use notify::event::{AccessKind, AccessMode};
        // A build reading source files emits Access(Open/Read/Close); these
        // must never trigger a rerun, or the pipeline loops on its own reads.
        assert!(!is_mutation(&EventKind::Access(AccessKind::Open(
            AccessMode::Any
        ))));
        assert!(!is_mutation(&EventKind::Access(AccessKind::Read)));
        assert!(!is_mutation(&EventKind::Access(AccessKind::Any)));
    }

    #[test]
    fn writes_creates_and_fallback_kinds_are_mutations() {
        use notify::event::{CreateKind, ModifyKind, RemoveKind};
        assert!(is_mutation(&EventKind::Modify(ModifyKind::Data(
            notify::event::DataChange::Content
        ))));
        assert!(is_mutation(&EventKind::Create(CreateKind::File)));
        assert!(is_mutation(&EventKind::Remove(RemoveKind::File)));
        // Fallback/polling backends report `Any`; keep triggering on those.
        assert!(is_mutation(&EventKind::Any));
    }
}
