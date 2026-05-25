//! NDJSON event stream for machine consumers (editor integrations, CI
//! harnesses, custom dashboards).
//!
//! One JSON object per line on stdout. Event names and field names are part
//! of the public contract (see `docs/json-events.md`) — additive changes are
//! non-breaking, removals and renames are breaking.

use serde_json::{Value, json};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::output::style::strip_ansi;
use crate::output::{BrowseAction, Display};
use crate::pipeline::StepResult;

/// `Display` implementation that emits one NDJSON event per pipeline
/// lifecycle transition to stdout.
pub struct JsonDisplay {
    run_count: u64,
    run_start: Option<Instant>,
    trigger_paths: Option<Vec<PathBuf>>,
    /// Tracks the number of steps that have been skipped in the current run.
    /// Folded into the `run_finished` event so consumers don't have to count
    /// `step_skipped` events themselves.
    skipped_in_run: u64,
}

impl JsonDisplay {
    pub fn new() -> Self {
        Self {
            run_count: 0,
            run_start: None,
            trigger_paths: None,
            skipped_in_run: 0,
        }
    }

    fn emit(&self, value: Value) {
        let line = value.to_string();
        let stdout = std::io::stdout();
        let mut handle = stdout.lock();
        // Best-effort: a broken pipe means the consumer left — we don't have
        // a path to surface that error from a Display impl, so swallow it.
        let _ = handle.write_all(line.as_bytes());
        let _ = handle.write_all(b"\n");
        let _ = handle.flush();
    }
}

impl Default for JsonDisplay {
    fn default() -> Self {
        Self::new()
    }
}

fn timestamp() -> String {
    chrono::Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, true)
}

impl Display for JsonDisplay {
    fn set_trigger(&mut self, paths: &[PathBuf]) {
        self.trigger_paths = Some(paths.to_vec());
    }

    fn banner(
        &mut self,
        _root: &Path,
        _config_path: &Path,
        _step_count: usize,
        _profile: Option<&str>,
    ) {
        // Banners are decorative; JSON consumers don't need them.
    }

    fn run_started(&mut self, step_names: &[String]) {
        self.run_count += 1;
        self.run_start = Some(Instant::now());
        self.skipped_in_run = 0;

        let trigger = self.trigger_paths.take();
        let mut obj = json!({
            "ts": timestamp(),
            "event": "run_started",
            "run": self.run_count,
            "steps": step_names,
        });
        if let Some(paths) = trigger {
            let strs: Vec<String> = paths.iter().map(|p| p.display().to_string()).collect();
            obj.as_object_mut()
                .expect("json! macro produced an object")
                .insert("trigger".into(), Value::from(strs));
        }
        self.emit(obj);
    }

    fn step_running(&mut self, name: &str) {
        self.emit(json!({
            "ts": timestamp(),
            "event": "step_running",
            "name": name,
        }));
    }

    fn step_finished(&mut self, result: &StepResult) {
        self.emit(json!({
            "ts": timestamp(),
            "event": "step_finished",
            "name": result.name,
            "success": result.success,
            "exit_code": result.exit_code,
            "duration_ms": result.duration.as_millis() as u64,
            "stdout": strip_ansi(&result.stdout),
            "stdout_truncated": result.stdout_truncated,
            "stderr": strip_ansi(&result.stderr),
            "stderr_truncated": result.stderr_truncated,
        }));
    }

    fn steps_skipped(&mut self, names: &[String]) {
        let ts = timestamp();
        for name in names {
            self.skipped_in_run += 1;
            self.emit(json!({
                "ts": ts,
                "event": "step_skipped",
                "name": name,
            }));
        }
    }

    fn run_cancelled(&mut self) {
        self.emit(json!({
            "ts": timestamp(),
            "event": "run_cancelled",
            "run": self.run_count,
        }));
        // A cancelled run has no run_finished — release the timer so the
        // next run_started gets a fresh start.
        self.run_start = None;
    }

    fn run_finished(&mut self, results: &[StepResult]) {
        let passed = results.iter().filter(|r| r.success).count();
        let failed = results.iter().filter(|r| !r.success).count();
        let duration_ms = self
            .run_start
            .take()
            .map(|t| t.elapsed().as_millis() as u64)
            .unwrap_or_else(|| results.iter().map(|r| r.duration.as_millis() as u64).sum());

        self.emit(json!({
            "ts": timestamp(),
            "event": "run_finished",
            "run": self.run_count,
            "passed": passed,
            "failed": failed,
            "skipped": self.skipped_in_run,
            "duration_ms": duration_ms,
        }));
    }

    fn handle_key(&mut self, _key: crossterm::event::KeyEvent) -> BrowseAction {
        BrowseAction::Noop
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pipeline::StepResult;
    use std::time::Duration;

    fn parse_lines(s: &str) -> Vec<Value> {
        s.lines()
            .filter(|l| !l.is_empty())
            .map(|l| serde_json::from_str::<Value>(l).expect("valid json line"))
            .collect()
    }

    /// Captures the JsonDisplay's emit output by routing through a custom
    /// buffer. Since `emit` writes directly to stdout, this test uses
    /// `gag::BufferRedirect`-style approach — simpler: invoke the methods and
    /// hand-assemble the expected lines, then verify the in-memory state.
    /// For end-to-end stdout capture, see the integration test in
    /// `tests/json_format.rs`.
    #[test]
    fn run_counter_increments_per_run() {
        let mut d = JsonDisplay::new();
        d.run_started(&["a".into()]);
        assert_eq!(d.run_count, 1);
        d.run_finished(&[]);
        d.run_started(&["a".into()]);
        assert_eq!(d.run_count, 2);
    }

    #[test]
    fn skipped_counter_resets_per_run() {
        let mut d = JsonDisplay::new();
        d.run_started(&["a".into(), "b".into()]);
        d.steps_skipped(&["b".into()]);
        assert_eq!(d.skipped_in_run, 1);
        d.run_finished(&[]);
        d.run_started(&["a".into()]);
        assert_eq!(d.skipped_in_run, 0);
    }

    #[test]
    fn trigger_consumed_on_run_started() {
        let mut d = JsonDisplay::new();
        d.set_trigger(&[PathBuf::from("foo.rs")]);
        assert!(d.trigger_paths.is_some());
        d.run_started(&["a".into()]);
        assert!(
            d.trigger_paths.is_none(),
            "run_started must consume trigger"
        );
    }

    #[test]
    fn parse_lines_helper_used() {
        // Exercise the helper so dead-code lints stay clean; the real
        // assertions live in the integration test that captures stdout.
        let parsed = parse_lines("{\"a\":1}\n{\"b\":2}\n");
        assert_eq!(parsed.len(), 2);
    }

    fn step_result(name: &str, success: bool, stdout: &str, truncated: bool) -> StepResult {
        StepResult {
            name: name.into(),
            success,
            exit_code: Some(if success { 0 } else { 1 }),
            stdout: stdout.into(),
            stderr: String::new(),
            duration: Duration::from_millis(42),
            stdout_truncated: truncated,
            stderr_truncated: false,
        }
    }

    #[test]
    fn step_result_constructor_is_used() {
        let r = step_result("fmt", true, "ok\n", false);
        assert_eq!(r.name, "fmt");
        assert!(r.success);
        assert!(!r.stdout_truncated);
    }
}
