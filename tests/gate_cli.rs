//! End-to-end tests for `baraddur gate`. Spawns the actual binary so the
//! exec-replacement path is exercised, not bypassed.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

fn write_config(root: &Path, step_cmd: &str) {
    std::fs::write(
        root.join(".baraddur.toml"),
        format!(
            r#"
[watch]
extensions = ["rs"]

[[steps]]
name = "gate-step"
cmd = "{step_cmd}"
"#
        ),
    )
    .unwrap();
}

fn baraddur() -> Command {
    Command::new(env!("CARGO_BIN_EXE_baraddur"))
}

/// On pipeline pass, the wrapped command runs. We verify by having the
/// wrapped command create a sentinel file; if it ran, the file exists.
#[test]
fn gate_passes_runs_wrapped_command() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(root, "true");

    let sentinel = root.join("sentinel.txt");
    let status = baraddur()
        .args(["gate", "touch"])
        .arg(&sentinel)
        .current_dir(root)
        .status()
        .unwrap();

    assert!(status.success(), "expected gate to exit 0; got {status:?}");
    assert!(
        sentinel.exists(),
        "wrapped `touch` did not run: sentinel {} missing",
        sentinel.display()
    );
}

/// On pipeline fail, the wrapped command is NOT executed and gate exits 1.
#[test]
fn gate_failure_does_not_run_wrapped_command() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(root, "false");

    let sentinel = root.join("sentinel.txt");
    let status = baraddur()
        .args(["gate", "touch"])
        .arg(&sentinel)
        .current_dir(root)
        .status()
        .unwrap();

    assert_eq!(status.code(), Some(1), "expected exit 1 on pipeline fail");
    assert!(
        !sentinel.exists(),
        "wrapped command ran despite pipeline failure"
    );
}

/// On pipeline pass, the wrapped command's exit code propagates through gate
/// (exec replaces the process on Unix, so the shell sees the wrapped exit).
#[test]
fn gate_propagates_wrapped_exit_code() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(root, "true");

    let status = baraddur()
        .args(["gate", "sh", "-c", "exit 42"])
        .current_dir(root)
        .status()
        .unwrap();

    assert_eq!(
        status.code(),
        Some(42),
        "expected wrapped exit code to propagate"
    );
}

/// `gate --staged` with nothing staged short-circuits: the pipeline is skipped
/// and the wrapped command runs immediately. Proven by configuring a step that
/// would *fail* if it ran — if exec happens, we know the pipeline was skipped.
#[test]
fn gate_staged_empty_skips_pipeline_and_execs() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    // A failing step would normally block exec. The test only passes if the
    // empty-staged short-circuit fires before `false` runs.
    write_config(root, "false");
    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "test@example.com"]);
    git(root, &["config", "user.name", "test"]);

    let sentinel = root.join("sentinel.txt");
    let status = baraddur()
        .args(["gate", "--staged", "touch"])
        .arg(&sentinel)
        .current_dir(root)
        .status()
        .unwrap();

    assert!(
        status.success(),
        "expected gate to short-circuit and exec; got {status:?}"
    );
    assert!(
        sentinel.exists(),
        "wrapped command did not run despite empty staged list"
    );
    // Pipeline skipped → run log should not exist.
    let log = root.join(".baraddur").join("last-run.log");
    assert!(
        !log.exists(),
        "expected pipeline to be skipped, but found {}",
        log.display()
    );
}

fn git(cwd: &Path, args: &[&str]) {
    let out = Command::new("git")
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
