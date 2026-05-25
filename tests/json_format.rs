//! End-to-end tests for `--format json`. Spawns the real binary so the
//! NDJSON stream captured here is what consumers (editor plugins, dashboards)
//! will see.

use std::path::Path;
use std::process::Command;
use tempfile::TempDir;

use serde_json::Value;

fn baraddur() -> Command {
    Command::new(env!("CARGO_BIN_EXE_baraddur"))
}

fn parse_ndjson(stdout: &[u8]) -> Vec<Value> {
    std::str::from_utf8(stdout)
        .expect("stdout is valid utf-8")
        .lines()
        .filter(|l| !l.is_empty())
        .map(|l| {
            serde_json::from_str::<Value>(l)
                .unwrap_or_else(|e| panic!("invalid JSON line `{l}`: {e}"))
        })
        .collect()
}

fn write_config(root: &Path, body: &str) {
    std::fs::write(root.join(".baraddur.toml"), body).unwrap();
}

/// A passing one-step pipeline must emit run_started → step_running →
/// step_finished → run_finished, all valid JSON, all on their own line.
#[test]
fn check_json_emits_expected_event_sequence() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(
        root,
        r#"
[watch]
extensions = ["rs"]

[[steps]]
name = "step-a"
cmd = "true"
"#,
    );

    let out = baraddur()
        .args(["check", "--format", "json"])
        .current_dir(root)
        .output()
        .unwrap();

    assert!(
        out.status.success(),
        "expected exit 0; stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let events = parse_ndjson(&out.stdout);
    let names: Vec<&str> = events
        .iter()
        .map(|e| e["event"].as_str().expect("event field"))
        .collect();
    assert_eq!(
        names,
        vec![
            "run_started",
            "step_running",
            "step_finished",
            "run_finished"
        ],
        "unexpected event sequence; full events: {events:#?}"
    );

    let run_started = &events[0];
    assert_eq!(run_started["run"], 1);
    assert_eq!(run_started["steps"], serde_json::json!(["step-a"]));

    let step_finished = &events[2];
    assert_eq!(step_finished["name"], "step-a");
    assert_eq!(step_finished["success"], true);
    assert_eq!(step_finished["exit_code"], 0);
    assert!(step_finished["duration_ms"].is_number());
    assert_eq!(step_finished["stdout_truncated"], false);
    assert_eq!(step_finished["stderr_truncated"], false);

    let run_finished = events.last().unwrap();
    assert_eq!(run_finished["run"], 1);
    assert_eq!(run_finished["passed"], 1);
    assert_eq!(run_finished["failed"], 0);
    assert_eq!(run_finished["skipped"], 0);
}

/// A failing step is reflected in step_finished.success=false and the
/// run_finished tally; the process still exits 1 like plain `check`.
#[test]
fn check_json_reports_failures() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(
        root,
        r#"
[watch]
extensions = ["rs"]

[[steps]]
name = "bad"
cmd = "false"
"#,
    );

    let out = baraddur()
        .args(["check", "--format", "json"])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));

    let events = parse_ndjson(&out.stdout);
    let step_finished = events
        .iter()
        .find(|e| e["event"] == "step_finished")
        .expect("step_finished present");
    assert_eq!(step_finished["success"], false);
    assert_eq!(step_finished["exit_code"], 1);

    let run_finished = events
        .iter()
        .find(|e| e["event"] == "run_finished")
        .expect("run_finished present");
    assert_eq!(run_finished["passed"], 0);
    assert_eq!(run_finished["failed"], 1);
}

/// Stage-failure causes a later step to be skipped; the skip surfaces as a
/// `step_skipped` event and is folded into `run_finished.skipped`.
#[test]
fn check_json_emits_step_skipped_on_stage_failure() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(
        root,
        r#"
[watch]
extensions = ["rs"]

[[steps]]
name = "first"
cmd = "false"

[[steps]]
name = "second"
cmd = "true"
"#,
    );

    let out = baraddur()
        .args(["check", "--format", "json"])
        .current_dir(root)
        .output()
        .unwrap();

    assert_eq!(out.status.code(), Some(1));

    let events = parse_ndjson(&out.stdout);
    let skipped: Vec<&str> = events
        .iter()
        .filter(|e| e["event"] == "step_skipped")
        .map(|e| e["name"].as_str().unwrap())
        .collect();
    assert_eq!(skipped, vec!["second"]);

    let run_finished = events
        .iter()
        .find(|e| e["event"] == "run_finished")
        .expect("run_finished present");
    assert_eq!(run_finished["skipped"], 1);
}

/// Captured stdout is ANSI-stripped so JSON consumers don't have to handle
/// escapes themselves.
#[test]
fn check_json_strips_ansi_from_captured_stdout() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(
        root,
        r#"
[watch]
extensions = ["rs"]

[[steps]]
name = "colored"
cmd = "printf '\\x1b[31mred\\x1b[0m\\n'"
"#,
    );

    let out = baraddur()
        .args(["check", "--format", "json"])
        .current_dir(root)
        .output()
        .unwrap();
    assert!(out.status.success());

    let events = parse_ndjson(&out.stdout);
    let step_finished = events
        .iter()
        .find(|e| e["event"] == "step_finished")
        .expect("step_finished present");
    let captured = step_finished["stdout"]
        .as_str()
        .expect("stdout is a string");
    assert!(
        !captured.contains('\u{1b}'),
        "expected ANSI stripped from stdout; got: {captured:?}"
    );
    assert!(
        captured.contains("red"),
        "expected payload preserved; got: {captured:?}"
    );
}

/// `--format json` and `--no-tty` are mutually exclusive at the clap level.
#[test]
fn format_json_conflicts_with_no_tty() {
    let td = TempDir::new().unwrap();
    let root = td.path();
    write_config(
        root,
        r#"
[watch]
extensions = ["rs"]
[[steps]]
name = "step"
cmd = "true"
"#,
    );

    let out = baraddur()
        .args(["check", "--format", "json", "--no-tty"])
        .current_dir(root)
        .output()
        .unwrap();

    assert!(
        !out.status.success(),
        "expected clap-level rejection of incompatible flags"
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("cannot be used with") || stderr.contains("conflicts"),
        "expected clap conflict message; got: {stderr}"
    );
}
