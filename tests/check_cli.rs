//! End-to-end tests for `baraddur check`. Spawns the real binary so the
//! exit codes the shell observes are the codes that matter.

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
name = "check-step"
cmd = "{step_cmd}"
"#
        ),
    )
    .unwrap();
}

fn baraddur() -> Command {
    Command::new(env!("CARGO_BIN_EXE_baraddur"))
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

#[test]
fn check_exits_zero_on_passing_pipeline() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(root, "true");

    let status = baraddur().arg("check").current_dir(root).status().unwrap();

    assert!(status.success(), "expected exit 0; got {status:?}");
    assert!(
        root.join(".baraddur").join("last-run.log").exists(),
        "run log should be written on every check"
    );
}

#[test]
fn check_exits_one_on_failing_step() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(root, "false");

    let status = baraddur().arg("check").current_dir(root).status().unwrap();

    assert_eq!(status.code(), Some(1), "expected exit 1 on step failure");
    let log = std::fs::read_to_string(root.join(".baraddur").join("last-run.log")).unwrap();
    assert!(
        log.contains("FAIL"),
        "expected FAIL marker in run log; got:\n{log}"
    );
}

/// Pointing `--config` at a nonexistent path must surface as exit code 2
/// (the dedicated "config error" code), distinct from a pipeline failure.
#[test]
fn check_exits_two_on_config_error() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    let missing = root.join("nope.toml");

    let status = baraddur()
        .arg("--config")
        .arg(&missing)
        .arg("check")
        .current_dir(root)
        .status()
        .unwrap();

    assert_eq!(
        status.code(),
        Some(2),
        "expected exit 2 for config error; got {status:?}"
    );
}

/// `check --staged` runs only the steps whose `if_changed` globs match the
/// staged paths. A `**/*.md` step with only `foo.rs` staged must be skipped —
/// and since the `.md` step is `false`, the run only passes when it's filtered
/// out.
#[test]
fn check_staged_filters_steps_by_path() {
    let td = TempDir::new().unwrap();
    let root = td.path();

    std::fs::write(
        root.join(".baraddur.toml"),
        r#"
[watch]
extensions = ["rs", "md"]

[[steps]]
name = "rust"
cmd = "true"
if_changed = ["**/*.rs"]

[[steps]]
name = "docs"
cmd = "false"
if_changed = ["**/*.md"]
"#,
    )
    .unwrap();

    git(root, &["init", "-q"]);
    git(root, &["config", "user.email", "test@example.com"]);
    git(root, &["config", "user.name", "test"]);
    std::fs::write(root.join("foo.rs"), "fn x() {}\n").unwrap();
    std::fs::write(root.join("notes.md"), "# notes\n").unwrap();
    git(root, &["add", "foo.rs"]);

    let status = baraddur()
        .args(["check", "--staged"])
        .current_dir(root)
        .status()
        .unwrap();

    assert!(
        status.success(),
        "expected exit 0 — the failing `docs` step should have been filtered out; got {status:?}"
    );
    let log = std::fs::read_to_string(root.join(".baraddur").join("last-run.log")).unwrap();
    assert!(log.contains("rust"), "rust step should run; log:\n{log}");
    assert!(
        !log.contains("docs"),
        "docs step should be filtered out; log:\n{log}"
    );
}
