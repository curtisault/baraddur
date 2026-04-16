use std::io::Write as _;
use std::time::Duration;

use crossterm::{
    cursor, execute,
    terminal::{self, ClearType},
};

use super::Display;
use crate::pipeline::StepResult;

// ── Non-TTY display (append-only) ───────────────────────────────────────────

/// Append-only line output for non-TTY contexts (piped, CI, `--no-tty`).
/// No cursor movement, no screen clearing, no colors.
pub struct PlainDisplay;

impl Display for PlainDisplay {
    fn run_started(&mut self, _step_names: &[String]) {
        println!("--- run started ---");
    }

    fn step_running(&mut self, name: &str) {
        println!("▸ {name} running");
    }

    fn step_finished(&mut self, result: &StepResult) {
        let status = if result.success { "✓" } else { "✗" };
        println!(
            "▸ {}  {}  ({:.1}s)",
            result.name,
            status,
            result.duration.as_secs_f64()
        );
        // Failure output is deferred to run_finished so parallel step output
        // doesn't interleave with step status lines.
    }

    fn steps_skipped(&mut self, names: &[String]) {
        for name in names {
            println!("▸ {name}  ⊘  skipped");
        }
    }

    fn run_cancelled(&mut self) {
        println!("--- run cancelled ---");
    }

    fn run_finished(&mut self, results: &[StepResult]) {
        // Print failure output blocks.
        for r in results.iter().filter(|r| !r.success) {
            println!("── {} output ──", r.name);
            if !r.stdout.is_empty() {
                print!("{}", r.stdout);
                if !r.stdout.ends_with('\n') {
                    println!();
                }
            }
            if !r.stderr.is_empty() {
                print!("{}", r.stderr);
                if !r.stderr.ends_with('\n') {
                    println!();
                }
            }
        }

        let failed = results.iter().filter(|r| !r.success).count();
        let passed = results.iter().filter(|r| r.success).count();
        let total: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();
        println!("--- run complete: {failed} failed, {passed} passed, {total:.1}s ---");
        let _ = std::io::stdout().flush();
    }
}

// ── TTY display (full-block redraw) ─────────────────────────────────────────

/// Status of a single step, tracked by the display for redraw.
#[derive(Debug, Clone)]
enum StepStatus {
    Queued,
    Running,
    Passed(Duration),
    Failed(Duration),
    Skipped,
}

/// Interactive terminal display. On each state change, erases the previous
/// step-status block and reprints it. No per-line cursor math — just
/// "erase N lines, reprint everything."
///
/// Phase 4 will add colors, spinners, right-aligned timings, and styled
/// section headers. Phase 3 keeps it structural — correct layout, no polish.
pub struct TtyDisplay {
    step_names: Vec<String>,
    statuses: Vec<StepStatus>,
    name_width: usize,
    /// How many lines the last `redraw()` printed. Used to move the cursor
    /// back up before the next redraw.
    rendered_lines: u16,
}

impl Default for TtyDisplay {
    fn default() -> Self {
        Self::new()
    }
}

impl TtyDisplay {
    pub fn new() -> Self {
        Self {
            step_names: Vec::new(),
            statuses: Vec::new(),
            name_width: 0,
            rendered_lines: 0,
        }
    }

    /// Erases the previous render and reprints the step-status block.
    fn redraw(&mut self) {
        let mut stdout = std::io::stdout();

        // Move cursor up to overwrite the previous block.
        if self.rendered_lines > 0 {
            execute!(
                stdout,
                cursor::MoveUp(self.rendered_lines),
                terminal::Clear(ClearType::FromCursorDown)
            )
            .ok();
        }

        let mut lines = 0u16;

        for (i, name) in self.step_names.iter().enumerate() {
            let (glyph, suffix) = match &self.statuses[i] {
                StepStatus::Queued => ("·", String::new()),
                StepStatus::Running => ("⟳", String::new()),
                StepStatus::Passed(d) => ("✓", format!("  {:.1}s", d.as_secs_f64())),
                StepStatus::Failed(d) => ("✗", format!("  {:.1}s", d.as_secs_f64())),
                StepStatus::Skipped => ("⊘", "  skipped".into()),
            };

            println!(
                "▸ {:width$}  {}{}",
                name,
                glyph,
                suffix,
                width = self.name_width
            );
            lines += 1;
        }

        self.rendered_lines = lines;
        let _ = std::io::Write::flush(&mut stdout);
    }

    /// Finds the index of a step by name. Panics if not found (programming
    /// error — runner must only reference steps from the config).
    fn index_of(&self, name: &str) -> usize {
        self.step_names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("unknown step `{name}`"))
    }
}

impl Display for TtyDisplay {
    fn run_started(&mut self, step_names: &[String]) {
        self.step_names = step_names.to_vec();
        self.statuses = vec![StepStatus::Queued; step_names.len()];
        self.name_width = step_names.iter().map(|n| n.len()).max().unwrap_or(0);
        self.rendered_lines = 0;

        // Full screen clear before each run.
        let mut stdout = std::io::stdout();
        execute!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )
        .ok();

        self.redraw();
    }

    fn step_running(&mut self, name: &str) {
        let idx = self.index_of(name);
        self.statuses[idx] = StepStatus::Running;
        self.redraw();
    }

    fn step_finished(&mut self, result: &StepResult) {
        let idx = self.index_of(&result.name);
        self.statuses[idx] = if result.success {
            StepStatus::Passed(result.duration)
        } else {
            StepStatus::Failed(result.duration)
        };
        self.redraw();
    }

    fn steps_skipped(&mut self, names: &[String]) {
        for name in names {
            let idx = self.index_of(name);
            self.statuses[idx] = StepStatus::Skipped;
        }
        self.redraw();
    }

    fn run_cancelled(&mut self) {
        // No-op in TTY mode. The next run_started will clear the screen.
        // Phase 4 can show a brief "cancelled — restarting..." transition.
        self.rendered_lines = 0;
    }

    fn run_finished(&mut self, results: &[StepResult]) {
        // The step block is already showing final state from the last redraw.
        // Reset rendered_lines so the next run_started doesn't try to erase
        // the failure output we're about to print below the block.
        self.rendered_lines = 0;

        // Print failure output blocks below the step list.
        let failures: Vec<_> = results.iter().filter(|r| !r.success).collect();
        if !failures.is_empty() {
            println!();
            for r in &failures {
                println!("── {} output ──", r.name);
                if !r.stdout.is_empty() {
                    print!("{}", r.stdout);
                    if !r.stdout.ends_with('\n') {
                        println!();
                    }
                }
                if !r.stderr.is_empty() {
                    print!("{}", r.stderr);
                    if !r.stderr.ends_with('\n') {
                        println!();
                    }
                }
            }
        }

        // Footer
        let failed = results.iter().filter(|r| !r.success).count();
        let passed = results.iter().filter(|r| r.success).count();
        let skipped = self.step_names.len().saturating_sub(results.len());
        let total: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();
        println!();
        if skipped > 0 {
            println!("{failed} failed · {passed} passed · {skipped} skipped · {total:.1}s");
        } else {
            println!("{failed} failed · {passed} passed · {total:.1}s");
        }
        let _ = std::io::Write::flush(&mut std::io::stdout());
    }
}
