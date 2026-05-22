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
