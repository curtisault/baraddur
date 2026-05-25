//! Post-failure hook: pipes combined failed-step output to a user-configured
//! command and captures its stdout. Cancelled (and the child process killed
//! via `kill_on_drop`) when the spawned task is aborted.

use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::Command;

use crate::config::OnFailureConfig;
use crate::pipeline::StepResult;

/// Formats failed step results into a single string suitable for piping to the
/// hook command. Only failed steps are included.
pub fn combine_failed_output(results: &[StepResult]) -> String {
    use std::fmt::Write as _;

    let mut out = String::new();
    for r in results.iter().filter(|r| !r.success) {
        let _ = writeln!(
            out,
            "=== {} (exit: {}) ===",
            r.name,
            r.exit_code
                .map(|c| c.to_string())
                .unwrap_or_else(|| "unknown".into())
        );
        if !r.stdout.is_empty() {
            out.push_str(&r.stdout);
            if !r.stdout.ends_with('\n') {
                out.push('\n');
            }
        }
        if !r.stderr.is_empty() {
            if !r.stdout.is_empty() {
                out.push_str("--- stderr ---\n");
            }
            out.push_str(&r.stderr);
            if !r.stderr.ends_with('\n') {
                out.push('\n');
            }
        }
        out.push('\n');
    }
    out
}

/// Runs the on_failure hook with `combined` (and optional prompt prefix) piped
/// on stdin. Returns captured stdout as `Ok(Some(text))` on success, `Ok(None)`
/// if the hook exits non-zero or times out (in which case a diagnostic is
/// written to stderr), and `Err` for malformed config.
///
/// The child process inherits no stdout/stderr; output is captured via pipes.
/// `kill_on_drop` is set, so aborting the surrounding task kills the child.
pub async fn run_hook(cfg: &OnFailureConfig, cwd: &Path, combined: &str) -> Result<Option<String>> {
    let parts = shell_words::split(&cfg.cmd)
        .with_context(|| format!("parsing on_failure.cmd `{}`", cfg.cmd))?;

    let (program, args) = parts
        .split_first()
        .ok_or_else(|| anyhow!("on_failure.cmd is empty"))?;

    let mut child = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .with_context(|| format!("spawning on_failure.cmd `{program}`"))?;

    // Write prompt + combined output to stdin, then close it so the child sees EOF.
    if let Some(mut stdin) = child.stdin.take() {
        if !cfg.prompt.is_empty() {
            let _ = stdin.write_all(cfg.prompt.as_bytes()).await;
            let _ = stdin.write_all(b"\n\n").await;
        }
        let _ = stdin.write_all(combined.as_bytes()).await;
        let _ = stdin.shutdown().await;
    }

    let timeout = Duration::from_secs(cfg.timeout_secs);
    let output = match tokio::time::timeout(timeout, child.wait_with_output()).await {
        Ok(Ok(out)) => out,
        Ok(Err(e)) => {
            eprintln!("baraddur: on_failure hook failed: {e}");
            return Ok(None);
        }
        Err(_) => {
            // Timed out — the Child is dropped here, kill_on_drop sends SIGKILL.
            eprintln!(
                "baraddur: on_failure hook timed out after {}s",
                cfg.timeout_secs
            );
            return Ok(None);
        }
    };

    if !output.status.success() {
        eprintln!(
            "baraddur: on_failure hook exited {}",
            output
                .status
                .code()
                .map(|c| c.to_string())
                .unwrap_or_else(|| "signal".into())
        );
        return Ok(None);
    }

    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    if text.is_empty() {
        Ok(None)
    } else {
        Ok(Some(text))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn cfg(cmd: &str, timeout_secs: u64) -> OnFailureConfig {
        OnFailureConfig {
            enabled: true,
            cmd: cmd.into(),
            prompt: String::new(),
            timeout_secs,
        }
    }

    fn pass_result(name: &str) -> StepResult {
        StepResult {
            name: name.into(),
            success: true,
            exit_code: Some(0),
            stdout: String::new(),
            stderr: String::new(),
            duration: Duration::from_millis(10),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    fn fail_result(name: &str, stdout: &str, stderr: &str) -> StepResult {
        StepResult {
            name: name.into(),
            success: false,
            exit_code: Some(1),
            stdout: stdout.into(),
            stderr: stderr.into(),
            duration: Duration::from_millis(10),
            stdout_truncated: false,
            stderr_truncated: false,
        }
    }

    #[test]
    fn combine_includes_only_failures() {
        let combined = combine_failed_output(&[
            pass_result("ok"),
            fail_result("bad", "error here\n", ""),
            pass_result("also-ok"),
        ]);
        assert!(combined.contains("bad"));
        assert!(combined.contains("error here"));
        assert!(!combined.contains("ok"));
    }

    #[test]
    fn combine_separates_stdout_and_stderr() {
        let combined = combine_failed_output(&[fail_result("bad", "out line\n", "err line\n")]);
        assert!(combined.contains("out line"));
        assert!(combined.contains("--- stderr ---"));
        assert!(combined.contains("err line"));
    }

    #[tokio::test]
    async fn run_hook_pipes_stdin_to_stdout() {
        let out = run_hook(&cfg("cat", 5), Path::new("."), "hello\nworld\n")
            .await
            .unwrap();
        assert_eq!(out.as_deref(), Some("hello\nworld"));
    }

    #[tokio::test]
    async fn run_hook_includes_prompt_prefix() {
        let mut c = cfg("cat", 5);
        c.prompt = "PROMPT".into();
        let out = run_hook(&c, Path::new("."), "DATA").await.unwrap();
        let text = out.unwrap();
        assert!(text.starts_with("PROMPT"));
        assert!(text.contains("DATA"));
    }

    #[tokio::test]
    async fn run_hook_returns_none_on_timeout() {
        // sleep 10s with a 1s timeout
        let out = run_hook(&cfg("sleep 10", 1), Path::new("."), "")
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn run_hook_returns_none_on_nonzero_exit() {
        let out = run_hook(&cfg("false", 5), Path::new("."), "anything")
            .await
            .unwrap();
        assert!(out.is_none());
    }

    #[tokio::test]
    async fn run_hook_returns_none_for_empty_stdout() {
        let out = run_hook(&cfg("true", 5), Path::new("."), "anything")
            .await
            .unwrap();
        assert!(out.is_none());
    }
}
