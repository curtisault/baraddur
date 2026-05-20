//! End-to-end tests for `App::run_until`.
//!
//! Drives the real watch loop on a temp directory with a fast, trivial pipeline.
//! Uses `run_until` instead of `run` so the test can inject a shutdown future
//! without sending SIGINT to the test runner.

use std::time::Duration;
use tempfile::TempDir;

use baraddur::App;
use baraddur::config::{Config, OutputConfig, Step, SummarizeConfig, WatchConfig};
use baraddur::output::{DisplayConfig, Verbosity};

fn trivial_app(td: &TempDir, step_cmd: &str) -> App {
    let root = td.path().to_path_buf();
    let config = Config {
        watch: WatchConfig {
            extensions: vec!["rs".into()],
            debounce_ms: 100,
            ignore: vec![],
        },
        output: OutputConfig::default(),
        summarize: SummarizeConfig::default(),
        steps: vec![Step {
            name: "noop".into(),
            cmd: step_cmd.into(),
            parallel: false,
        }],
    };
    App {
        config,
        config_path: root.join(".baraddur.toml"),
        root,
        display_config: DisplayConfig {
            is_tty: false,
            no_clear: true,
            verbosity: Verbosity::Quiet,
        },
    }
}

/// The loop must exit promptly when the stop signal resolves, and must have
/// completed at least one pipeline iteration first (last-run.log on disk).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_until_exits_on_stop_signal() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let app = trivial_app(&td, "true");

    let stop = async {
        tokio::time::sleep(Duration::from_millis(400)).await;
    };

    let result = tokio::time::timeout(Duration::from_secs(5), app.run_until(stop))
        .await
        .expect("run_until did not return within 5s");

    result.expect("run_until returned an error");

    // The initial pipeline run should have completed and written the log.
    let log = root.join(".baraddur").join("last-run.log");
    assert!(
        log.exists(),
        "expected {} to exist after one pipeline run",
        log.display()
    );
    let contents = std::fs::read_to_string(&log).unwrap();
    assert!(
        contents.contains("noop"),
        "log should mention the step name; got:\n{contents}"
    );
    assert!(
        contents.contains("pass"),
        "log should mark the step as passing; got:\n{contents}"
    );
}

/// A failing step must surface in the log so the user knows what broke.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_until_records_failures() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let app = trivial_app(&td, "false");

    let stop = async {
        tokio::time::sleep(Duration::from_millis(400)).await;
    };

    let _ = tokio::time::timeout(Duration::from_secs(5), app.run_until(stop))
        .await
        .expect("run_until did not return within 5s");

    let log = root.join(".baraddur").join("last-run.log");
    let contents = std::fs::read_to_string(&log).unwrap();
    assert!(
        contents.contains("FAIL"),
        "log should mark the step as failing; got:\n{contents}"
    );
}

/// The loop must surrender control quickly enough that an immediate stop
/// signal doesn't get blocked behind an arbitrary watcher event or pipeline
/// completion. Asserts the loop exits within ~2 seconds of the signal firing.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_until_exits_promptly() {
    let td = TempDir::new().unwrap();
    let app = trivial_app(&td, "true");

    let start = std::time::Instant::now();
    let stop = async {
        // Fire almost immediately. The select! should observe it on the next
        // poll, no matter what stage of the loop we're in.
        tokio::time::sleep(Duration::from_millis(50)).await;
    };

    let _ = tokio::time::timeout(Duration::from_secs(5), app.run_until(stop))
        .await
        .expect("run_until did not return within 5s");

    let elapsed = start.elapsed();
    assert!(
        elapsed < Duration::from_secs(2),
        "expected prompt shutdown; took {elapsed:?}"
    );
}
