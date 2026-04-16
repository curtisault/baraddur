use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::process::Command;

use crate::config::Step;

/// Maximum bytes captured per step. Output beyond this is truncated.
const MAX_CAPTURE_BYTES: usize = 100 * 1024; // 100 KiB

#[derive(Debug, Clone)]
pub struct StepResult {
    pub name: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

/// Runs a single step and returns its result.
///
/// A step that exits non-zero is a failing `StepResult`, not an `Err`.
/// Only returns `Err` for things like malformed `cmd` strings.
pub async fn run(step: &Step, cwd: &Path) -> Result<StepResult> {
    let parts = shell_words::split(&step.cmd)
        .with_context(|| format!("parsing cmd `{}` for step `{}`", step.cmd, step.name))?;

    let (program, args) = parts
        .split_first()
        .ok_or_else(|| anyhow!("empty cmd for step `{}`", step.name))?;

    let start = Instant::now();

    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .output()
        .await;

    let duration = start.elapsed();

    let result = match output {
        Ok(out) => StepResult {
            name: step.name.clone(),
            success: out.status.success(),
            exit_code: out.status.code(),
            stdout: truncate_capture(&out.stdout),
            stderr: truncate_capture(&out.stderr),
            duration,
        },
        Err(e) => StepResult {
            name: step.name.clone(),
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("failed to launch `{program}`: {e}"),
            duration,
        },
    };

    Ok(result)
}

fn truncate_capture(bytes: &[u8]) -> String {
    if bytes.len() <= MAX_CAPTURE_BYTES {
        String::from_utf8_lossy(bytes).into_owned()
    } else {
        let mut s = String::from_utf8_lossy(&bytes[..MAX_CAPTURE_BYTES]).into_owned();
        s.push_str("\n... [output truncated at 100 KiB] ...\n");
        s
    }
}
