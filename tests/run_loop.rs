//! End-to-end tests for `App::run_until`.
//!
//! Drives the real watch loop on a temp directory with a fast, trivial pipeline.
//! Uses `run_until` instead of `run` so the test can inject a shutdown future
//! without sending SIGINT to the test runner.

use std::path::PathBuf;
use std::time::Duration;
use tempfile::TempDir;

use baraddur::App;
use baraddur::RunOnceOptions;
use baraddur::config::{Config, OnFailureConfig, OutputConfig, Step, WatchConfig};
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
        on_failure: OnFailureConfig::default(),
        steps: vec![Step {
            name: "noop".into(),
            cmd: step_cmd.into(),
            parallel: false,
            if_changed: Vec::new(),
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

/// When `on_failure` is enabled and a step fails, the hook command must
/// receive the failing step's output on stdin and its stdout written somewhere
/// observable. The test uses `tee` to redirect stdin to a file, which is the
/// most portable way to verify "the hook actually ran with the expected input"
/// without relying on the Display trait being instrumentable from a test.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn on_failure_hook_runs_after_failing_run() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let sentinel = root.join("hook-input.txt");

    // `tee` reads stdin and writes it to both the named file and stdout. We
    // care about the file (proves the hook saw the failed-step output).
    let mut app = trivial_app(&td, "false");
    app.config.on_failure = OnFailureConfig {
        enabled: true,
        cmd: format!("tee {}", sentinel.display()),
        prompt: "PROMPT_LINE".into(),
        timeout_secs: 5,
    };

    let stop = async {
        // Give the pipeline time to fail, then the hook time to run.
        tokio::time::sleep(Duration::from_millis(800)).await;
    };

    let _ = tokio::time::timeout(Duration::from_secs(10), app.run_until(stop))
        .await
        .expect("run_until did not return within 10s");

    let captured = std::fs::read_to_string(&sentinel)
        .unwrap_or_else(|_| panic!("expected {} to exist", sentinel.display()));
    assert!(
        captured.contains("PROMPT_LINE"),
        "hook stdin missing prompt prefix; got:\n{captured}"
    );
    assert!(
        captured.contains("noop"),
        "hook stdin missing failed step name; got:\n{captured}"
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

/// A one-shot `run_once` against a passing pipeline returns `Ok(true)` and
/// writes the standard run log.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_once_returns_success_on_passing_pipeline() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let app = trivial_app(&td, "true");

    let success = tokio::time::timeout(
        Duration::from_secs(5),
        app.run_once(RunOnceOptions::default()),
    )
    .await
    .expect("run_once did not return within 5s")
    .expect("run_once returned an error");

    assert!(
        success,
        "run_once should return true for a passing pipeline"
    );

    let log = root.join(".baraddur").join("last-run.log");
    assert!(log.exists(), "expected {} to exist", log.display());
    let contents = std::fs::read_to_string(&log).unwrap();
    assert!(contents.contains("noop"));
    assert!(contents.contains("pass"));
}

/// `run_once` with an `initial_trigger` drawn from `git diff --cached`
/// applies `if_changed` filtering exactly like watch mode: only steps whose
/// globs match the staged paths should run. End-to-end through a real git
/// repo so the staged-paths discovery is exercised, not mocked.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_once_with_staged_paths_filters_step_subset() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();

    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "test"]);

    std::fs::write(root.join("foo.rs"), "fn x() {}\n").unwrap();
    std::fs::write(root.join("notes.md"), "# notes\n").unwrap();
    git(&root, &["add", "foo.rs"]);

    let staged = baraddur::git::staged_paths(&root).await.unwrap();
    assert_eq!(staged, vec![PathBuf::from("foo.rs")]);

    let app = App {
        config: Config {
            watch: WatchConfig {
                extensions: vec!["rs".into()],
                debounce_ms: 100,
                ignore: vec![],
            },
            output: OutputConfig::default(),
            on_failure: OnFailureConfig::default(),
            steps: vec![
                Step {
                    name: "rust".into(),
                    cmd: "true".into(),
                    parallel: false,
                    if_changed: vec!["**/*.rs".into()],
                },
                Step {
                    name: "docs".into(),
                    cmd: "true".into(),
                    parallel: false,
                    if_changed: vec!["**/*.md".into()],
                },
            ],
        },
        config_path: root.join(".baraddur.toml"),
        root: root.clone(),
        display_config: DisplayConfig {
            is_tty: false,
            no_clear: true,
            verbosity: Verbosity::Quiet,
        },
    };

    let success = app
        .run_once(RunOnceOptions {
            no_hook: false,
            initial_trigger: Some(staged),
        })
        .await
        .unwrap();
    assert!(success);

    let log = std::fs::read_to_string(root.join(".baraddur").join("last-run.log")).unwrap();
    assert!(
        log.contains("rust"),
        "expected `rust` step to run; log:\n{log}"
    );
    assert!(
        !log.contains("docs"),
        "expected `docs` step to be filtered out; log:\n{log}"
    );
}

fn git(cwd: &std::path::Path, args: &[&str]) {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn git_stdout(cwd: &std::path::Path, args: &[&str]) -> String {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .unwrap_or_else(|e| panic!("git {args:?} failed to spawn: {e}"));
    assert!(
        out.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).unwrap()
}

/// `run_once` must run the configured `[on_failure]` hook when the pipeline
/// fails — mirroring watch-mode behavior. The hook runs synchronously here
/// (unlike watch mode's async-detached hook), so the call returns only after
/// the hook completes. `tee` captures stdin to a file so we can verify the
/// failed-step output reached the hook.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_once_runs_on_failure_hook() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let sentinel = root.join("hook-input.txt");

    let mut app = trivial_app(&td, "false");
    app.config.on_failure = OnFailureConfig {
        enabled: true,
        cmd: format!("tee {}", sentinel.display()),
        prompt: "PROMPT_LINE".into(),
        timeout_secs: 5,
    };

    let success = tokio::time::timeout(
        Duration::from_secs(10),
        app.run_once(RunOnceOptions::default()),
    )
    .await
    .expect("run_once did not return within 10s")
    .expect("run_once returned an error");

    assert!(!success);

    let captured = std::fs::read_to_string(&sentinel)
        .unwrap_or_else(|_| panic!("expected hook to write {}", sentinel.display()));
    assert!(
        captured.contains("PROMPT_LINE"),
        "hook stdin missing prompt prefix; got:\n{captured}"
    );
    assert!(
        captured.contains("noop"),
        "hook stdin missing failed step name; got:\n{captured}"
    );
}

/// `RunOnceOptions::no_hook = true` must skip the hook even when
/// `on_failure.enabled` is true. The sentinel file's absence proves the hook
/// did not run.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_once_no_hook_skips_on_failure_hook() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let sentinel = root.join("hook-input.txt");

    let mut app = trivial_app(&td, "false");
    app.config.on_failure = OnFailureConfig {
        enabled: true,
        cmd: format!("tee {}", sentinel.display()),
        prompt: "PROMPT_LINE".into(),
        timeout_secs: 5,
    };

    let success = tokio::time::timeout(
        Duration::from_secs(10),
        app.run_once(RunOnceOptions {
            no_hook: true,
            initial_trigger: None,
        }),
    )
    .await
    .expect("run_once did not return within 10s")
    .expect("run_once returned an error");

    assert!(!success);
    assert!(
        !sentinel.exists(),
        "hook ran despite no_hook = true: {} exists",
        sentinel.display()
    );
}

/// `git::diff_since` reports both committed changes since `base` AND
/// untracked-but-not-ignored files. `.gitignore`d paths must be excluded so
/// build artifacts don't trigger steps.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn diff_since_includes_committed_changes_and_untracked_but_not_ignored() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();

    git(&root, &["init", "-q"]);
    git(&root, &["config", "user.email", "test@example.com"]);
    git(&root, &["config", "user.name", "test"]);

    // Baseline commit: one tracked file, plus a .gitignore.
    std::fs::write(root.join(".gitignore"), "ignored.rs\n").unwrap();
    std::fs::write(root.join("a.rs"), "fn a() {}\n").unwrap();
    git(&root, &["add", ".gitignore", "a.rs"]);
    git(&root, &["commit", "-q", "-m", "baseline"]);
    // Capture baseline by SHA — `"HEAD"` would move with the next commit.
    let baseline = git_stdout(&root, &["rev-parse", "HEAD"]);
    let baseline = baseline.trim();

    // After the baseline: modify a tracked file (new commit), add an
    // untracked file, and add an untracked-but-ignored file.
    std::fs::write(root.join("a.rs"), "fn a() { let _ = 1; }\n").unwrap();
    git(&root, &["commit", "-q", "-am", "modify a"]);
    std::fs::write(root.join("b.rs"), "fn b() {}\n").unwrap();
    std::fs::write(root.join("ignored.rs"), "fn ignored() {}\n").unwrap();

    let mut paths = baraddur::git::diff_since(&root, baseline).await.unwrap();
    paths.sort();

    assert_eq!(
        paths,
        vec![PathBuf::from("a.rs"), PathBuf::from("b.rs")],
        "expected committed-change `a.rs` + untracked `b.rs`; `ignored.rs` must be excluded"
    );
}

/// A one-shot `run_once` against a failing pipeline returns `Ok(false)` and
/// records the failure in the log.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn run_once_returns_failure_on_failing_step() {
    let td = TempDir::new().unwrap();
    let root = td.path().to_path_buf();
    let app = trivial_app(&td, "false");

    let success = tokio::time::timeout(
        Duration::from_secs(5),
        app.run_once(RunOnceOptions::default()),
    )
    .await
    .expect("run_once did not return within 5s")
    .expect("run_once returned an error");

    assert!(!success, "run_once should return false when a step fails");

    let log = root.join(".baraddur").join("last-run.log");
    let contents = std::fs::read_to_string(&log).unwrap();
    assert!(contents.contains("FAIL"));
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
