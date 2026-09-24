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
    /// True when `stdout` was clipped at `MAX_CAPTURE_BYTES`.
    pub stdout_truncated: bool,
    /// True when `stderr` was clipped at `MAX_CAPTURE_BYTES`.
    pub stderr_truncated: bool,
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
        Ok(out) => {
            let (stdout, stdout_truncated) = truncate_capture(&out.stdout);
            let (stderr, stderr_truncated) = truncate_capture(&out.stderr);
            StepResult {
                name: step.name.clone(),
                success: out.status.success(),
                exit_code: out.status.code(),
                stdout,
                stderr,
                duration,
                stdout_truncated,
                stderr_truncated,
            }
        }
        Err(e) => StepResult {
            name: step.name.clone(),
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("failed to launch `{program}`: {e}"),
            duration,
            stdout_truncated: false,
            stderr_truncated: false,
        },
    };

    Ok(result)
}

fn truncate_capture(bytes: &[u8]) -> (String, bool) {
    if bytes.len() <= MAX_CAPTURE_BYTES {
        (String::from_utf8_lossy(bytes).into_owned(), false)
    } else {
        let mut s = String::from_utf8_lossy(&bytes[..MAX_CAPTURE_BYTES]).into_owned();
        s.push_str("\n... [output truncated at 100 KiB] ...\n");
        (s, true)
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Sizes clustered around the truncation boundary plus small ones, so
    /// the interesting edge is hit far more often than uniform sampling
    /// over 100 KiB would manage.
    fn capture_size() -> impl Strategy<Value = usize> {
        prop_oneof![
            0usize..512,
            (MAX_CAPTURE_BYTES - 8)..(MAX_CAPTURE_BYTES + 8),
        ]
    }

    proptest! {
        /// The truncated flag is set exactly when input exceeds the cap; the
        /// untruncated text is the full lossy decode, the truncated text is
        /// the lossy decode of the first `MAX_CAPTURE_BYTES` plus a marker.
        #[test]
        fn truncation_is_exactly_at_the_cap(size in capture_size(), seed in any::<u8>()) {
            let bytes: Vec<u8> = (0..size).map(|i| seed.wrapping_add(i as u8)).collect();
            let (text, truncated) = truncate_capture(&bytes);

            prop_assert_eq!(truncated, bytes.len() > MAX_CAPTURE_BYTES);
            if truncated {
                let head = String::from_utf8_lossy(&bytes[..MAX_CAPTURE_BYTES]);
                prop_assert!(text.starts_with(head.as_ref()));
                prop_assert!(text.ends_with("truncated at 100 KiB] ...\n"));
            } else {
                prop_assert_eq!(text, String::from_utf8_lossy(&bytes).into_owned());
            }
        }
    }
}
