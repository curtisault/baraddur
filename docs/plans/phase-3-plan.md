# Phase 3 — Parallel Execution + Lifecycle + Basic Redraw

> Companion to [`project-plan.md`](./project-plan.md), [`ux-design.md`](./ux-design.md),
> [`phase-1-plan.md`](./phase-1-plan.md), and [`phase-2-plan.md`](./phase-2-plan.md).
> A step-by-step implementation plan for Phase 3.

---

## Goal

Transform Barad-dur from a sequential runner into a real pipeline executor
with lifecycle management. Phase 3 delivers:

1. **Stage grouping.** Group steps into stages by consecutive `parallel` flag.
   Sequential steps run alone; consecutive parallel steps run concurrently.
2. **Parallel execution.** Stages containing multiple steps execute via
   `tokio::task::JoinSet`. All steps in a parallel stage run to completion
   before the pipeline decides whether to continue.
3. **Full-block redraw.** A TTY-aware display that redraws the entire step
   status block on every state change, so parallel step statuses update
   cleanly without per-line cursor math.
4. **TTY detection.** At startup, detect whether stdout is a terminal via
   `std::io::IsTerminal`. Non-TTY mode emits append-only line output (the
   existing `PlainDisplay` behavior, enhanced with timestamps).
5. **Graceful shutdown.** First Ctrl+C cancels any in-flight pipeline, kills
   child processes, and exits with code 0. A second Ctrl+C within 2 seconds
   force-exits with code 130.
6. **Mid-run cancel+restart.** When file events arrive while a pipeline is
   running, abort the in-flight run (dropping the future kills all children
   via `kill_on_drop`) and start a fresh one from stage 1.

Phase 2's completed items (config validation, discovery, ignore patterns)
stay green. Phase 3 replaces `run_sequential` with `run_pipeline` and
rewrites the event loop in `lib.rs`.

---

## Success Criteria (Smoke Test)

### SC-1 — Parallel steps run concurrently

```toml
# .baraddur.toml
[watch]
extensions = ["ex", "exs", "heex"]
ignore = ["_build", "deps", ".git"]

[[steps]]
name = "compile"
cmd = "mix compile --warnings-as-errors"

[[steps]]
name = "credo"
cmd = "mix credo"
parallel = true

[[steps]]
name = "test"
cmd = "mix test --failed"
parallel = true
```

```sh
$ baraddur
▸ compile   ✓   1.1s
▸ credo     ⟳
▸ test      ⟳
```

Both `credo` and `test` show `⟳` simultaneously. Each settles independently.
Total wall-clock time for the parallel stage is `max(credo, test)`, not
`credo + test`.

### SC-2 — Stage failure skips subsequent stages

```sh
▸ compile   ✗   warnings as errors   0.9s
▸ credo     ⊘   skipped
▸ test      ⊘   skipped

── compile output ──────────────────────────────────────────────────
  ...

1 failed · 0 passed · 2 skipped · 0.9s
```

### SC-3 — Parallel stage failure: all steps in the stage complete

If `credo` fails but `test` passes, both still run to completion before the
pipeline reports results:

```sh
▸ compile   ✓   1.1s
▸ credo     ✗   3 issues   1.8s
▸ test      ✓   2.3s

── credo output ────────────────────────────────────────────────────
  ...

1 failed · 2 passed · 5.4s
```

No subsequent stages would run (there are none in this config, but if there
were, they'd be skipped).

### SC-4 — Mid-run cancel+restart on file change

```sh
▸ compile   ✓   1.1s
▸ credo     ⟳
▸ test      ⟳
# (save a file while credo and test are running)
# screen clears, new run starts:
▸ compile   ⟳
▸ credo     ·
▸ test      ·
```

The in-flight `credo` and `test` processes are killed. A fresh pipeline
starts from stage 1.

### SC-5 — Graceful Ctrl+C

```sh
▸ compile   ✓   1.1s
▸ credo     ⟳
▸ test      ⟳
^C
baraddur: exiting.
$ echo $?
0
```

Child processes are killed, watcher stops, exit code 0.

### SC-6 — Double-tap Ctrl+C force-exits

```sh
^C
baraddur: exiting...
^C
baraddur: force exit.
$ echo $?
130
```

### SC-7 — TTY detection

```sh
# TTY mode (interactive terminal):
$ baraddur
▸ compile   ✓   1.1s
▸ credo     ✓   1.8s
▸ test      ✓   2.3s
# (screen redraws in place on each state change)

# Non-TTY mode (piped):
$ baraddur 2>&1 | cat
--- run started ---
▸ compile running
▸ compile ✓  (1.1s)
▸ credo running
▸ test running
▸ credo ✓  (1.8s)
▸ test ✓  (2.3s)
--- run complete: 0 failed, 3 passed, 2.3s ---
```

### SC-8 — Non-TTY forced via `--no-tty`

```sh
$ baraddur --no-tty
--- run started ---
▸ compile running
...
```

---

## Non-Goals for Phase 3 (deferred)

- **Colors, styled text.** Phase 4 adds green `✓`, red `✗`, yellow `⟳`,
  cyan headers, dim metadata via crossterm's styling API.
- **Spinner animation.** Phase 4 adds braille-dot animation on a shared
  frame clock.
- **Step timings right-aligned.** Phase 4 handles layout refinement.
- **Output truncation (100 KiB cap).** Phase 4.
- **Verbosity levels (`-q`, `-v`, `-vv`).** Phase 4.
- **Startup banner (S1).** Phase 4.
- **Idle footer.** Phase 4.
- **LLM summaries.** Phase 5.
- **Per-step timeout.** Open question from project plan — not Phase 3 scope.

---

## Prerequisites

- Phase 2 complete (it is — see the checklist in `project-plan.md`).
- **New dependency: `crossterm`.** Used for cursor movement and screen
  clearing in the TTY display. Already listed as a key crate in the project
  plan but intentionally omitted from Cargo.toml until now.
- Familiarity with `tokio::task::JoinSet` — the primary concurrency
  primitive for parallel stages.

### Pre-flight: verify crate versions

Check [crates.io](https://crates.io) for the current stable version of
`crossterm`. The plan uses `0.28`, but verify before coding. The API surface
we use (cursor movement, screen clearing) has been stable across minor
versions.

---

## Implementation Order

Bottom-up. Each step ends with a green `cargo check` (or `cargo test` where
noted).

```
1. Cargo.toml + schema tweaks          → Step 0
2. Stage types + grouping function      → Step 1
3. Display trait expansion              → Step 2
4. PlainDisplay update (non-TTY)        → Step 3
5. TtyDisplay (full-block redraw)       → Step 4
6. Pipeline runner rewrite              → Step 5
7. Event loop rewrite (lib.rs)          → Step 6
8. main.rs updates                      → Step 7
9. Tests                                → Step 8
10. Smoke test                          → Step 9
```

---

## Step 0 — Dependencies + Schema Tweaks

### Cargo.toml

Add `crossterm` to dependencies:

```toml
[dependencies]
# ... existing deps ...
crossterm = "0.28"
```

Run `cargo build` to confirm it resolves.

### `src/config/schema.rs` — Add `Clone` to `Step`

Parallel stages spawn tasks via `JoinSet`. Each spawned task needs to own
its `Step` data, so `Step` must be `Clone`. This is cheap — two `String`s
and a `bool`.

```rust
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    pub cmd: String,

    #[serde(default)]
    pub parallel: bool,
}
```

### `src/main.rs` — Add `--no-tty` flag

Reserve the flag now so it's available when we wire TTY detection in Step 6.

```rust
#[derive(Parser, Debug)]
#[command(
    name = "baraddur",
    version,
    about = "Project-agnostic file watcher that surfaces issues before CI"
)]
struct Cli {
    /// Config file path (disables walk-up discovery)
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Directory to watch [default: directory containing the discovered config]
    #[arg(short = 'w', long)]
    watch_dir: Option<PathBuf>,

    /// Force non-TTY (append-only) output even on a terminal
    #[arg(long)]
    no_tty: bool,
}
```

`cargo check` should pass.

---

## Step 1 — Stage Types + Grouping

**File:** `src/pipeline/mod.rs`

Define the `Stage` type and a function to group steps into stages based on
consecutive `parallel` flags.

### Execution model recap

From `project-plan.md`:

```
stage 1: [format]           # parallel=false → runs alone, must pass
stage 2: [compile]          # parallel=false → runs alone, must pass
stage 3: [credo, test]      # parallel=true  → run concurrently via JoinSet
```

Rules:
- A step with `parallel = false` is always its own stage (one step).
- Consecutive steps with `parallel = true` are batched into a single stage.
- A stage with one step runs directly (no JoinSet overhead).
- A stage with multiple steps runs all concurrently via JoinSet.

### Code

```rust
// src/pipeline/mod.rs
pub mod runner;
pub mod step;

pub use runner::run_pipeline;
pub use step::{run as run_step, StepResult};

use crate::config::Step;

/// A group of steps that execute together as one stage of the pipeline.
///
/// - A stage with one step runs that step directly.
/// - A stage with multiple steps runs them all concurrently via `JoinSet`.
#[derive(Debug)]
pub struct Stage<'a> {
    pub steps: Vec<&'a Step>,
}

impl Stage<'_> {
    /// A stage is parallel when it contains more than one step.
    /// (A single `parallel = true` step is still alone in its stage
    /// and doesn't need concurrent machinery.)
    pub fn is_parallel(&self) -> bool {
        self.steps.len() > 1
    }
}

/// Groups steps into stages by consecutive `parallel` flag.
///
/// Sequential steps (`parallel = false`) each become their own stage.
/// Consecutive parallel steps (`parallel = true`) are batched into one stage.
pub fn group_into_stages(steps: &[Step]) -> Vec<Stage<'_>> {
    let mut stages = Vec::new();
    let mut i = 0;

    while i < steps.len() {
        if steps[i].parallel {
            // Collect consecutive parallel steps into one stage.
            let start = i;
            while i < steps.len() && steps[i].parallel {
                i += 1;
            }
            stages.push(Stage {
                steps: steps[start..i].iter().collect(),
            });
        } else {
            // Sequential step = its own stage.
            stages.push(Stage {
                steps: vec![&steps[i]],
            });
            i += 1;
        }
    }

    stages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Step;

    fn step(name: &str, parallel: bool) -> Step {
        Step {
            name: name.into(),
            cmd: "true".into(),
            parallel,
        }
    }

    #[test]
    fn all_sequential() {
        let steps = vec![step("a", false), step("b", false), step("c", false)];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].steps.len(), 1);
        assert_eq!(stages[0].steps[0].name, "a");
        assert!(!stages[0].is_parallel());
    }

    #[test]
    fn all_parallel() {
        let steps = vec![step("a", true), step("b", true), step("c", true)];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].steps.len(), 3);
        assert!(stages[0].is_parallel());
    }

    #[test]
    fn mixed_sequential_then_parallel() {
        let steps = vec![
            step("format", false),
            step("compile", false),
            step("credo", true),
            step("test", true),
        ];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].steps[0].name, "format");
        assert_eq!(stages[1].steps[0].name, "compile");
        assert_eq!(stages[2].steps.len(), 2);
        assert_eq!(stages[2].steps[0].name, "credo");
        assert_eq!(stages[2].steps[1].name, "test");
    }

    #[test]
    fn parallel_then_sequential() {
        let steps = vec![
            step("a", true),
            step("b", true),
            step("c", false),
        ];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].steps.len(), 2);
        assert_eq!(stages[1].steps.len(), 1);
    }

    #[test]
    fn single_step() {
        let steps = vec![step("only", false)];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 1);
        assert!(!stages[0].is_parallel());
    }

    #[test]
    fn empty_steps() {
        let stages = group_into_stages(&[]);
        assert!(stages.is_empty());
    }

    #[test]
    fn alternating_parallel_sequential() {
        let steps = vec![
            step("a", true),
            step("b", true),
            step("c", false),
            step("d", true),
            step("e", true),
        ];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].steps.len(), 2);  // a, b
        assert_eq!(stages[1].steps.len(), 1);  // c
        assert_eq!(stages[2].steps.len(), 2);  // d, e
    }
}
```

**Design notes:**
- `Stage` borrows from the `Step` slice via lifetime `'a`. Stages are
  transient — computed at the start of each run and consumed immediately.
  No need for owned data.
- `is_parallel()` is based on `steps.len() > 1`, not on the `parallel`
  flag. A lone `parallel = true` step doesn't need JoinSet overhead.
- The old `run_sequential` re-export is replaced by `run_pipeline` (written
  in Step 5). Temporarily comment out the re-export or point it at a stub
  until Step 5 is done.

`cargo check` should pass after updating the re-export.

---

## Step 2 — Display Trait Expansion

**File:** `src/output/mod.rs`

The Phase 1/2 Display trait needs new methods to support parallel execution,
skipped steps, and run cancellation. This is the full replacement:

```rust
// src/output/mod.rs
pub mod display;
pub use display::{PlainDisplay, TtyDisplay};

use crate::pipeline::StepResult;

/// Sink for pipeline lifecycle events.
///
/// Implementations:
/// - `PlainDisplay` — append-only line output for non-TTY / piped contexts.
/// - `TtyDisplay` — full-block redraw for interactive terminals.
pub trait Display: Send {
    /// A new pipeline run is starting. `step_names` lists all steps in
    /// declared order, used for layout (name-column padding).
    ///
    /// In TTY mode this clears the screen. In non-TTY mode it prints a
    /// header line.
    fn run_started(&mut self, step_names: &[String]);

    /// A step has begun executing. Called once per step, in execution order.
    /// For parallel stages, called for each step before any of them finish.
    fn step_running(&mut self, name: &str);

    /// A step has completed (pass or fail). For parallel stages, called as
    /// each step finishes (order is non-deterministic).
    fn step_finished(&mut self, result: &StepResult);

    /// Steps were skipped because an earlier stage failed. Called once per
    /// skipped stage with the names of all steps in that stage.
    fn steps_skipped(&mut self, names: &[String]);

    /// The run was cancelled mid-flight (file change triggered restart).
    /// In TTY mode this is a no-op (screen will clear on next `run_started`).
    /// In non-TTY mode this emits a line so the log shows the interruption.
    fn run_cancelled(&mut self);

    /// The run completed normally. `results` contains all steps that actually
    /// executed (not skipped ones). Prints failure output blocks and a
    /// summary footer.
    fn run_finished(&mut self, results: &[StepResult]);
}
```

**Changes from Phase 2:**
- `run_started()` now takes `&[String]` for step names (layout computation).
- `step_started()` renamed to `step_running()` (clearer semantics — the step
  is actively executing, not just about to start).
- Added `steps_skipped()` for UX state S7.
- Added `run_cancelled()` for UX state S8.

This breaks `PlainDisplay` and the test `RecordingDisplay` — they're updated
in Steps 3 and 8.

`cargo check` will fail here until Step 3 updates PlainDisplay. That's OK —
proceed immediately to Step 3.

---

## Step 3 — PlainDisplay Update (Non-TTY)

**File:** `src/output/display.rs`

Update `PlainDisplay` to implement the expanded trait. This is the non-TTY
display — append-only lines, no cursor movement, no screen clearing.

Phase 3 keeps `PlainDisplay` in the same file and adds `TtyDisplay`
alongside it in Step 4. (Phase 4 may split into separate files if needed.)

```rust
// src/output/display.rs
use std::io::Write as _;

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

        if !result.success {
            println!("── {} output ──", result.name);
            if !result.stdout.is_empty() {
                print!("{}", result.stdout);
                if !result.stdout.ends_with('\n') {
                    println!();
                }
            }
            if !result.stderr.is_empty() {
                print!("{}", result.stderr);
                if !result.stderr.ends_with('\n') {
                    println!();
                }
            }
        }
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
        let failed = results.iter().filter(|r| !r.success).count();
        let passed = results.iter().filter(|r| r.success).count();
        let total: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();
        println!("--- run complete: {failed} failed, {passed} passed, {total:.1}s ---");
        let _ = std::io::stdout().flush();
    }
}
```

**Key change from Phase 2:** `step_finished` in PlainDisplay no longer
prints raw output inline for failures. Instead, that responsibility moves
to `run_finished` which prints all failure output at the end. This avoids
interleaving failure output with the step status lines when parallel steps
finish in non-deterministic order.

Wait — on reflection, for non-TTY mode the UX doc shows failure output
inline (`[timestamp] --- credo output ---`). Since non-TTY is append-only,
interleaving is the natural behavior. But for parallel stages, we want to
collect all results first and then show failures. Two options:

1. Show failure output inline as each step finishes (even interleaved).
2. Defer failure output to `run_finished`.

Option 2 is cleaner and matches the TTY behavior. Use it:

```rust
    fn step_finished(&mut self, result: &StepResult) {
        let status = if result.success { "✓" } else { "✗" };
        println!(
            "▸ {}  {}  ({:.1}s)",
            result.name,
            status,
            result.duration.as_secs_f64()
        );
        // Failure output is printed in run_finished, not here.
    }

    fn run_finished(&mut self, results: &[StepResult]) {
        // Print failure output blocks
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

        // Footer
        let failed = results.iter().filter(|r| !r.success).count();
        let passed = results.iter().filter(|r| r.success).count();
        let total: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();
        println!("--- run complete: {failed} failed, {passed} passed, {total:.1}s ---");
        let _ = std::io::stdout().flush();
    }
```

`cargo check` should pass with the updated PlainDisplay (though TtyDisplay
doesn't exist yet — add a stub or skip re-exporting it until Step 4).

---

## Step 4 — TtyDisplay (Full-Block Redraw)

**File:** `src/output/display.rs` (appended after `PlainDisplay`)

The TTY display maintains internal state for every step and redraws the
entire step-status block on each state change. This is the "full-block
redraw" approach — simple and correct, no per-line cursor tracking.

### How the redraw works

1. `run_started` clears the screen and prints the initial block (all steps
   queued).
2. Each `step_running` / `step_finished` / `steps_skipped` call updates
   internal state, then calls `redraw()`.
3. `redraw()` moves the cursor up `N` lines (where `N` is the number of
   lines printed last time), clears from cursor to end of screen, and
   reprints the block.
4. `run_finished` stops redrawing the step block (it's already final),
   appends failure output and footer below, and resets state so the next
   `run_started` does a full screen clear.

### Internal state

```rust
/// Status of a single step, tracked by the display for redraw.
#[derive(Debug, Clone)]
enum StepStatus {
    Queued,
    Running,
    Passed(Duration),
    Failed(Duration),
    Skipped,
}
```

### Code

```rust
// src/output/display.rs (continued after PlainDisplay)

use crossterm::{cursor, execute, terminal::{self, ClearType}};
use std::time::Duration;

// ── TTY display (full-block redraw) ─────────────────────────────────────────

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

#[derive(Debug, Clone)]
enum StepStatus {
    Queued,
    Running,
    Passed(Duration),
    Failed(Duration),
    Skipped,
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
                StepStatus::Passed(d) => {
                    ("✓", format!("  {:.1}s", d.as_secs_f64()))
                }
                StepStatus::Failed(d) => {
                    ("✗", format!("  {:.1}s", d.as_secs_f64()))
                }
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
        // The step block is already showing final state from the last
        // redraw. Stop tracking lines so the next redraw (from a new run)
        // doesn't erase the failure output we're about to print.
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
        let skipped = self.step_names.len() - results.len();
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
```

### Update `src/output/mod.rs` exports

```rust
pub mod display;
pub use display::{PlainDisplay, TtyDisplay};
```

`cargo check` should pass.

---

## Step 5 — Pipeline Runner Rewrite

**File:** `src/pipeline/runner.rs`

Replace `run_sequential` with `run_pipeline`. The new runner groups steps
into stages, executes sequential stages directly and parallel stages via
`JoinSet`, and calls the expanded `Display` trait methods.

```rust
// src/pipeline/runner.rs
use anyhow::{Context, Result};
use std::path::Path;
use tokio::task::JoinSet;

use super::step::{self, StepResult};
use super::group_into_stages;
use crate::config::Config;
use crate::output::Display;

/// Runs the full pipeline: groups steps into stages, executes each stage
/// (sequentially or in parallel), stops after the first failing stage.
///
/// Returns all `StepResult`s for steps that actually ran (not skipped).
pub async fn run_pipeline(
    config: &Config,
    cwd: &Path,
    display: &mut dyn Display,
) -> Result<Vec<StepResult>> {
    let step_names: Vec<String> = config.steps.iter().map(|s| s.name.clone()).collect();
    display.run_started(&step_names);

    let stages = group_into_stages(&config.steps);
    let mut all_results: Vec<StepResult> = Vec::with_capacity(config.steps.len());
    let mut stage_failed = false;

    for stage in &stages {
        if stage_failed {
            // Skip this entire stage — an earlier stage failed.
            let skipped_names: Vec<String> =
                stage.steps.iter().map(|s| s.name.clone()).collect();
            display.steps_skipped(&skipped_names);
            continue;
        }

        let stage_results = if stage.is_parallel() {
            run_parallel_stage(stage, cwd, display).await?
        } else {
            run_sequential_step(stage.steps[0], cwd, display).await?
        };

        // Check if any step in this stage failed.
        if stage_results.iter().any(|r| !r.success) {
            stage_failed = true;
        }

        all_results.extend(stage_results);
    }

    display.run_finished(&all_results);
    Ok(all_results)
}

/// Runs a single step (a stage with one step).
async fn run_sequential_step(
    step_cfg: &crate::config::Step,
    cwd: &Path,
    display: &mut dyn Display,
) -> Result<Vec<StepResult>> {
    display.step_running(&step_cfg.name);
    let result = step::run(step_cfg, cwd).await?;
    display.step_finished(&result);
    Ok(vec![result])
}

/// Runs all steps in a parallel stage concurrently via JoinSet.
/// All steps run to completion before returning — even if some fail.
async fn run_parallel_stage(
    stage: &super::Stage<'_>,
    cwd: &Path,
    display: &mut dyn Display,
) -> Result<Vec<StepResult>> {
    // Mark all steps in the stage as running.
    for &step_cfg in &stage.steps {
        display.step_running(&step_cfg.name);
    }

    // Spawn each step as a separate task.
    let mut set = JoinSet::new();
    for &step_cfg in &stage.steps {
        let step_owned = step_cfg.clone();
        let cwd_owned = cwd.to_path_buf();
        set.spawn(async move { step::run(&step_owned, &cwd_owned).await });
    }

    // Collect results as tasks complete (order is non-deterministic).
    let mut stage_results = Vec::with_capacity(stage.steps.len());
    while let Some(join_result) = set.join_next().await {
        let step_result = join_result
            .context("step task panicked")?
            .context("step execution failed")?;
        display.step_finished(&step_result);
        stage_results.push(step_result);
    }

    Ok(stage_results)
}
```

### Update `src/pipeline/mod.rs` re-exports

Replace the old `run_sequential` export:

```rust
pub use runner::run_pipeline;
```

The full `mod.rs` becomes:

```rust
pub mod runner;
pub mod step;

pub use runner::run_pipeline;
pub use step::{run as run_step, StepResult};

// ... Stage, group_into_stages, and tests from Step 1 ...
```

**Design notes:**

- **`JoinSet` is the right primitive.** It manages a set of spawned tasks,
  allows polling them one-by-one via `join_next()`, and aborts all remaining
  tasks when dropped. This is exactly what we need for both normal execution
  and cancellation.

- **`kill_on_drop(true)` in `step::run`** (set in Phase 1) is critical here.
  When the JoinSet is dropped (e.g., the pipeline future is cancelled by
  `tokio::select!`), each task is aborted. The aborted task drops its
  `Command::output()` future, which drops the `Child`, which sends SIGKILL
  to the child process. The chain: `select! drop → JoinSet drop → task abort
  → future drop → Child drop → SIGKILL`.

- **`step::run` returns `Result<StepResult>`**, where a non-zero exit is a
  failing `StepResult` (not an `Err`). `Err` is reserved for infrastructure
  failures (bad `cmd` parse, task panic). The runner propagates `Err`
  immediately — the pipeline can't continue if a step's command couldn't
  even be parsed.

- **Display is called from the main task**, not from spawned tasks. The
  JoinSet spawns tasks that only run the command. Results flow back through
  `join_next()`, and the main task calls `display.step_finished()`. No
  `Send`/`Sync` issues, no `Arc<Mutex<dyn Display>>` needed.

- **Step execution order within a parallel stage is non-deterministic.**
  Steps are spawned and polled via JoinSet, and `join_next()` returns
  whichever task finishes first. The display handles this correctly because
  it tracks each step by name, not by position.

`cargo check` should pass.

---

## Step 6 — Event Loop Rewrite (`lib.rs`)

**File:** `src/lib.rs`

The Phase 2 event loop is sequential: run pipeline, then wait for events.
Phase 3's event loop must handle three concurrent concerns:

1. Running the pipeline.
2. Watching for file changes (to cancel+restart).
3. Watching for Ctrl+C (to shut down).

### Architecture

```
         ┌─────────────────────────────────────────────────────┐
         │                   Event Loop                        │
         │                                                     │
    ┌────┴─────┐    ┌─────────────────┐    ┌──────────────┐   │
    │ Ctrl+C   │    │ File watcher rx │    │ Pipeline fut │   │
    └────┬─────┘    └────────┬────────┘    └──────┬───────┘   │
         │                   │                     │           │
         │    tokio::select! (biased)              │           │
         │                                         │           │
         ▼                   ▼                     ▼           │
      Shutdown          Cancel+Restart          Completed      │
         │                   │                     │           │
         │                   └──→ loop back ───────┘           │
         │                                                     │
         └─────────────────────────────────────────────────────┘
```

The key insight: `tokio::select!` drops the losing branches' futures. When
the file watcher fires during a pipeline run, `select!` drops the pipeline
future, which drops the JoinSet, which aborts tasks, which kills child
processes. No explicit cancellation token needed.

### Borrow safety with `tokio::select!`

The pipeline future borrows `&mut display`. When `select!` completes
(regardless of which branch won), all futures are dropped and the borrow
is released. Code after the `select!` block can access `display` again.

To make this work cleanly, the `select!` returns an enum describing what
happened, and the post-select `match` handles each case with `display`
available:

```rust
enum RunOutcome {
    Completed(Result<Vec<StepResult>>),
    FileChange,
    Shutdown,
    WatcherDied,
}
```

### Code

```rust
// src/lib.rs
pub mod config;
pub mod output;
pub mod pipeline;
pub mod watcher;

use anyhow::Result;
use std::io::IsTerminal as _;
use std::path::PathBuf;
use std::time::Duration;

use crate::output::{Display, PlainDisplay, TtyDisplay};

pub struct App {
    pub config: config::Config,
    pub config_path: PathBuf,
    pub root: PathBuf,
    pub no_tty: bool,
}

impl App {
    pub async fn run(self) -> Result<()> {
        let is_tty = !self.no_tty && std::io::stdout().is_terminal();

        let mut display: Box<dyn Display> = if is_tty {
            Box::new(TtyDisplay::new())
        } else {
            Box::new(PlainDisplay)
        };

        // Only print the banner in non-TTY mode (TTY mode clears the screen
        // on each run, so a startup banner would vanish immediately).
        if !is_tty {
            eprintln!(
                "baraddur: watching {}\n          (config: {})",
                self.root.display(),
                self.config_path.display(),
            );
        }

        let wcfg = watcher::WatchConfig {
            root: self.root.clone(),
            debounce: Duration::from_millis(self.config.watch.debounce_ms),
            extensions: self.config.watch.extensions.clone(),
            ignore: self.config.watch.ignore.clone(),
        };
        let mut rx = watcher::start(wcfg)?;

        // Main event loop. `should_run` is true when a pipeline run is
        // needed (initial run + after file changes).
        let mut should_run = true;

        loop {
            if should_run {
                should_run = false;

                // Run the pipeline, but also listen for file changes and
                // Ctrl+C. tokio::select! drops the losing futures, which
                // kills child processes via kill_on_drop.
                let outcome = tokio::select! {
                    biased;

                    _ = tokio::signal::ctrl_c() => RunOutcome::Shutdown,

                    maybe = rx.recv() => {
                        match maybe {
                            Some(()) => RunOutcome::FileChange,
                            None => RunOutcome::WatcherDied,
                        }
                    }

                    result = pipeline::run_pipeline(
                        &self.config,
                        &self.root,
                        display.as_mut(),
                    ) => RunOutcome::Completed(result),
                };

                // Pipeline future is dropped here — all borrows released.
                // display is available again.
                match outcome {
                    RunOutcome::Completed(result) => {
                        result?;
                        // Pipeline finished. Fall through to idle-wait below.
                    }
                    RunOutcome::FileChange => {
                        // Drain any additional pending triggers.
                        while rx.try_recv().is_ok() {}
                        display.run_cancelled();
                        should_run = true;
                        continue;
                    }
                    RunOutcome::Shutdown => {
                        return self.shutdown().await;
                    }
                    RunOutcome::WatcherDied => {
                        eprintln!(
                            "baraddur: file watcher stopped unexpectedly. exiting."
                        );
                        return Ok(());
                    }
                }
            }

            // ── Idle: wait for the next file change or Ctrl+C ───────────
            tokio::select! {
                biased;

                _ = tokio::signal::ctrl_c() => {
                    return self.shutdown().await;
                }

                maybe = rx.recv() => {
                    match maybe {
                        Some(()) => {
                            while rx.try_recv().is_ok() {}
                            should_run = true;
                        }
                        None => {
                            eprintln!(
                                "baraddur: file watcher stopped unexpectedly. exiting."
                            );
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    /// Handles graceful shutdown. Spawns a background task that listens for
    /// a second Ctrl+C and force-exits with code 130 if it arrives.
    async fn shutdown(&self) -> Result<()> {
        eprintln!("\nbaraddur: exiting...");

        // Double-tap handler: a second Ctrl+C within any timeframe
        // force-exits immediately.
        tokio::spawn(async {
            tokio::signal::ctrl_c().await.ok();
            eprintln!("baraddur: force exit.");
            std::process::exit(130);
        });

        // Normal cleanup: the watcher rx and display are dropped when App
        // is dropped. Child processes were already killed by dropping the
        // pipeline future. Nothing blocking to do here.
        Ok(())
    }
}

enum RunOutcome {
    Completed(Result<Vec<pipeline::StepResult>>),
    FileChange,
    Shutdown,
    WatcherDied,
}
```

**Design notes:**

- **`biased` in `tokio::select!`.** Ensures deterministic priority: Ctrl+C
  is always checked first, then file events, then pipeline completion. Without
  `biased`, random branch selection could miss a shutdown signal when
  multiple events arrive simultaneously.

- **`RunOutcome` enum.** Allows the `select!` result to be handled in a
  `match` block after the futures are dropped, so `display` is accessible
  again. This sidesteps Rust's borrow-checker constraint on having both the
  pipeline future (which borrows `display`) and the cancel handler (which
  needs `display`) active simultaneously.

- **Shutdown double-tap.** On first Ctrl+C, `shutdown()` is called. It
  spawns a background task that listens for the second Ctrl+C and calls
  `std::process::exit(130)`. The main function returns `Ok(())` (exit
  code 0) for the normal path. If the user presses Ctrl+C again before
  the process exits, the background task fires and force-exits.

- **No CancellationToken needed.** `tokio::select!`'s drop semantics handle
  cancellation naturally. The kill chain is:
  `select! drop → run_pipeline future drop → JoinSet drop → task abort →
  Command::output() future drop → Child drop → SIGKILL`.

- **`should_run` flag.** A simple state machine: `true` means "run the
  pipeline," `false` means "wait for events." Set to `true` on startup
  (initial run) and after each file change.

- **`is_tty` detection.** Uses `std::io::IsTerminal` (stable since Rust
  1.70). Checks `stdout.is_terminal()` AND `!self.no_tty`. The `--no-tty`
  flag forces non-TTY mode even on a terminal.

- **Banner suppression in TTY mode.** The TTY display clears the screen on
  each run, so a startup banner would vanish instantly. The banner is only
  printed in non-TTY mode. Phase 4 adds a proper startup state (S1) that
  persists before the first clear.

---

## Step 7 — `main.rs` Updates

**File:** `src/main.rs`

Wire the `--no-tty` flag through to `App`:

```rust
// src/main.rs
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use baraddur::config::{self, ConfigSource};

#[derive(Parser, Debug)]
#[command(
    name = "baraddur",
    version,
    about = "Project-agnostic file watcher that surfaces issues before CI"
)]
struct Cli {
    /// Config file path (disables walk-up discovery)
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Directory to watch [default: directory containing the discovered config]
    #[arg(short = 'w', long)]
    watch_dir: Option<PathBuf>,

    /// Force non-TTY (append-only) output even on a terminal
    #[arg(long)]
    no_tty: bool,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let loaded = match config::load(cli.config.as_deref()) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("baraddur: {e}");
            return ExitCode::from(2);
        }
    };

    let root = match cli.watch_dir {
        Some(p) => p,
        None => match loaded.source {
            ConfigSource::WalkUp => loaded.config_dir.clone(),
            ConfigSource::CliOverride | ConfigSource::Global => match std::env::current_dir() {
                Ok(cwd) => cwd,
                Err(e) => {
                    eprintln!("baraddur: getting current directory: {e}");
                    return ExitCode::from(1);
                }
            },
        },
    };

    let app = baraddur::App {
        config: loaded.config,
        config_path: loaded.config_path,
        root,
        no_tty: cli.no_tty,
    };

    match app.run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("baraddur: {e:#}");
            ExitCode::from(1)
        }
    }
}
```

**Changes from Phase 2:**
- Added `no_tty: bool` field to `Cli` and passed through to `App`.
- Exit code semantics remain: `0` success/shutdown, `1` runtime, `2` config.
  Force-exit via double-tap Ctrl+C uses `std::process::exit(130)` directly
  (handled in `lib.rs`), so it never flows through `main`'s return.

`cargo check` should pass.

---

## Step 8 — Tests

### 8a. Stage grouping unit tests

Already written in Step 1 inside `pipeline/mod.rs`. Seven tests covering
all grouping edge cases.

### 8b. Update `tests/runner.rs` — RecordingDisplay + new tests

The existing integration tests need updating for the new Display trait and
`run_pipeline` API. Add new tests for parallel execution and stage skipping.

```rust
// tests/runner.rs
use baraddur::config::{Config, OutputConfig, Step, SummarizeConfig, WatchConfig};
use baraddur::output::Display;
use baraddur::pipeline;
use baraddur::pipeline::StepResult;
use std::sync::Mutex;

/// Test display that records all lifecycle events for assertion.
#[derive(Default)]
struct RecordingDisplay {
    events: Mutex<Vec<String>>,
}

impl RecordingDisplay {
    fn events(&self) -> Vec<String> {
        self.events.lock().unwrap().clone()
    }
}

impl Display for RecordingDisplay {
    fn run_started(&mut self, step_names: &[String]) {
        self.events
            .lock()
            .unwrap()
            .push(format!("run_started:{}", step_names.join(",")));
    }

    fn step_running(&mut self, name: &str) {
        self.events
            .lock()
            .unwrap()
            .push(format!("running:{name}"));
    }

    fn step_finished(&mut self, r: &StepResult) {
        self.events
            .lock()
            .unwrap()
            .push(format!("finished:{}:{}", r.name, r.success));
    }

    fn steps_skipped(&mut self, names: &[String]) {
        for name in names {
            self.events
                .lock()
                .unwrap()
                .push(format!("skipped:{name}"));
        }
    }

    fn run_cancelled(&mut self) {
        self.events.lock().unwrap().push("run_cancelled".into());
    }

    fn run_finished(&mut self, _results: &[StepResult]) {
        self.events.lock().unwrap().push("run_finished".into());
    }
}

fn make_config(steps: Vec<Step>) -> Config {
    Config {
        watch: WatchConfig {
            extensions: vec!["rs".into()],
            debounce_ms: 1000,
            ignore: vec![],
        },
        output: OutputConfig::default(),
        summarize: SummarizeConfig::default(),
        steps,
    }
}

// ── Sequential behavior (same as Phase 2, now via run_pipeline) ─────────────

#[tokio::test]
async fn sequential_stops_at_first_failure() {
    let cfg = make_config(vec![
        Step { name: "first".into(), cmd: "true".into(), parallel: false },
        Step { name: "second".into(), cmd: "false".into(), parallel: false },
        Step { name: "third".into(), cmd: "true".into(), parallel: false },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    // first passes, second fails, third is skipped.
    assert_eq!(results.len(), 2);
    assert!(results[0].success);
    assert!(!results[1].success);

    let events = display.events();
    assert!(events.contains(&"skipped:third".to_string()));
    assert!(!events.iter().any(|e| e.starts_with("running:third")));
}

#[tokio::test]
async fn sequential_all_pass() {
    let cfg = make_config(vec![
        Step { name: "a".into(), cmd: "true".into(), parallel: false },
        Step { name: "b".into(), cmd: "true".into(), parallel: false },
        Step { name: "c".into(), cmd: "true".into(), parallel: false },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.success));
}

// ── Parallel execution ──────────────────────────────────────────────────────

#[tokio::test]
async fn parallel_steps_all_run() {
    let cfg = make_config(vec![
        Step { name: "a".into(), cmd: "true".into(), parallel: true },
        Step { name: "b".into(), cmd: "true".into(), parallel: true },
        Step { name: "c".into(), cmd: "true".into(), parallel: true },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.success));

    // All three should have been marked as running before any finished.
    let events = display.events();
    let running_indices: Vec<_> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.starts_with("running:"))
        .map(|(i, _)| i)
        .collect();
    let first_finished = events
        .iter()
        .position(|e| e.starts_with("finished:"))
        .unwrap();
    assert!(
        running_indices.iter().all(|&i| i < first_finished),
        "all steps should be marked running before any finish"
    );
}

#[tokio::test]
async fn parallel_stage_runs_all_even_if_one_fails() {
    let cfg = make_config(vec![
        Step { name: "pass".into(), cmd: "true".into(), parallel: true },
        Step { name: "fail".into(), cmd: "false".into(), parallel: true },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    // Both steps ran — even though one failed.
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|r| r.success));
    assert!(results.iter().any(|r| !r.success));
}

#[tokio::test]
async fn parallel_wall_clock_is_max_not_sum() {
    // Two steps that each sleep 0.3s. If parallel, wall clock should be
    // ~0.3s, not ~0.6s. Allow generous margin for CI.
    let cfg = make_config(vec![
        Step {
            name: "slow_a".into(),
            cmd: "sleep 0.3".into(),
            parallel: true,
        },
        Step {
            name: "slow_b".into(),
            cmd: "sleep 0.3".into(),
            parallel: true,
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let start = std::time::Instant::now();
    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 2);
    // Should be ~0.3s, definitely under 0.55s if running in parallel.
    assert!(
        elapsed.as_secs_f64() < 0.55,
        "parallel steps took {:.2}s — expected under 0.55s",
        elapsed.as_secs_f64()
    );
}

// ── Mixed stages ────────────────────────────────────────────────────────────

#[tokio::test]
async fn mixed_stages_sequential_then_parallel() {
    let cfg = make_config(vec![
        Step { name: "seq".into(), cmd: "true".into(), parallel: false },
        Step { name: "par_a".into(), cmd: "true".into(), parallel: true },
        Step { name: "par_b".into(), cmd: "true".into(), parallel: true },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.success));

    let events = display.events();
    // seq must finish before par_a/par_b start running.
    let seq_finished = events.iter().position(|e| e == "finished:seq:true").unwrap();
    let par_a_running = events.iter().position(|e| e == "running:par_a").unwrap();
    assert!(seq_finished < par_a_running);
}

#[tokio::test]
async fn stage_failure_skips_subsequent_stages() {
    let cfg = make_config(vec![
        Step { name: "fail".into(), cmd: "false".into(), parallel: false },
        Step { name: "skip_a".into(), cmd: "true".into(), parallel: true },
        Step { name: "skip_b".into(), cmd: "true".into(), parallel: true },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    // Only the failing step ran.
    assert_eq!(results.len(), 1);
    assert!(!results[0].success);

    let events = display.events();
    assert!(events.contains(&"skipped:skip_a".to_string()));
    assert!(events.contains(&"skipped:skip_b".to_string()));
}

// ── Output capture ──────────────────────────────────────────────────────────

#[tokio::test]
async fn captures_stdout_and_stderr_on_failure() {
    let cfg = make_config(vec![Step {
        name: "noisyfail".into(),
        cmd: "sh -c 'echo out; echo err >&2; exit 1'".into(),
        parallel: false,
    }]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert!(results[0].stdout.contains("out"));
    assert!(results[0].stderr.contains("err"));
}
```

### 8c. Existing `tests/config_load.rs` — no changes needed

Config loading is unchanged in Phase 3. All existing tests should pass
as-is.

### 8d. Run the full suite

```sh
cargo test
```

All tests should pass: schema, validate, discovery, config_load, runner
(new), and pipeline grouping.

---

## Step 9 — Smoke-Test Checklist

After implementation, run through each manually:

- [ ] **Parallel steps run concurrently.** Configure `credo` + `test` as
      `parallel = true`. Both show `⟳` simultaneously. Wall-clock time is
      roughly `max(credo, test)`.
- [ ] **Stage failure skips later stages.** Break `compile`. Verify that
      `credo` and `test` show `⊘ skipped`.
- [ ] **Parallel stage — all steps complete.** One parallel step fails, the
      other passes. Both run to completion before results are reported.
- [ ] **Mid-run cancel+restart.** Save a file while `compile` is running.
      The run restarts from stage 1. Old child processes are killed.
- [ ] **Mid-run cancel during parallel stage.** Save a file while two
      parallel steps are running. Both child processes are killed. New run
      starts.
- [ ] **Ctrl+C during run.** Press Ctrl+C while a step is running. Process
      exits with code 0.
- [ ] **Ctrl+C during idle.** Let a run complete, then press Ctrl+C. Exit
      code 0.
- [ ] **Double-tap Ctrl+C.** Press Ctrl+C twice quickly. Process exits with
      code 130.
- [ ] **TTY mode.** Run interactively. Screen clears on each run. Step
      statuses redraw in place.
- [ ] **Non-TTY mode (piped).** Run `baraddur 2>&1 | cat`. Output is
      append-only, no escape sequences.
- [ ] **Non-TTY via `--no-tty`.** Run `baraddur --no-tty`. Append-only
      output even on a terminal.
- [ ] **All Phase 2 behavior preserved.** Config walk-up, global fallback,
      validation errors, unknown field errors — all still work. Exit code 2
      for config errors.

---

## References

### Crate documentation

| Crate | URL |
|---|---|
| `tokio::task::JoinSet` | https://docs.rs/tokio/latest/tokio/task/struct.JoinSet.html |
| `tokio::select!` | https://docs.rs/tokio/latest/tokio/macro.select.html |
| `tokio::signal::ctrl_c` | https://docs.rs/tokio/latest/tokio/signal/fn.ctrl_c.html |
| `std::io::IsTerminal` | https://doc.rust-lang.org/std/io/trait.IsTerminal.html |
| `crossterm::terminal` | https://docs.rs/crossterm/latest/crossterm/terminal/index.html |
| `crossterm::cursor` | https://docs.rs/crossterm/latest/crossterm/cursor/index.html |
| `crossterm::execute!` | https://docs.rs/crossterm/latest/crossterm/macro.execute.html |

### Conceptual reading

- **Tokio `select!` cancellation safety** — https://tokio.rs/tokio/tutorial/select#cancellation
  Critical for understanding what happens when a branch is dropped. Key
  takeaway: `Command::output()` is cancel-safe because it creates a new
  future each time it's polled; dropping it just kills the child.
- **JoinSet lifecycle** — when a JoinSet is dropped, all tasks are aborted.
  This is the mechanism that kills parallel step processes on cancel.
- **`biased` in `select!`** — without `biased`, branches are polled in
  random order. With `biased`, they're polled top-to-bottom. Use `biased`
  when priority matters (signals before work).

---

## Common Pitfalls

1. **`Step` must be `Clone`.** JoinSet tasks need owned data. The spawned
   async block can't borrow from the `Stage` because the borrow doesn't
   live long enough (the task is `'static`). Clone the `Step` and
   `PathBuf` before spawning.

2. **`tokio::select!` and `&mut display`.** The pipeline future borrows
   `&mut display`. You cannot access `display` in other `select!` branches
   while the pipeline future exists. Use the `RunOutcome` enum pattern:
   `select!` returns a simple enum (no borrows), then `match` on it after
   the `select!` block where all futures are dropped.

3. **`biased` matters for correctness.** Without `biased`, a simultaneous
   Ctrl+C and file event might randomly pick the file event, causing a
   restart instead of a shutdown. Always put `Ctrl+C` first and use
   `biased`.

4. **JoinSet task results are `Result<T, JoinError>`.** A `JoinError`
   means the task panicked or was cancelled. In normal operation, cancelled
   tasks don't appear in `join_next()` results (the JoinSet is dropped
   before polling). But handle it defensively with `.context("task
   panicked")?`.

5. **Double-counted `join_next()`.** If you break out of the
   `while let Some(result) = set.join_next().await` loop early (e.g., on
   the first failure), the remaining tasks still run until the JoinSet is
   dropped. Phase 3's design intentionally lets all parallel steps complete
   — don't short-circuit.

6. **Non-deterministic parallel step ordering.** `join_next()` returns
   whichever task finishes first. Don't assume steps finish in declaration
   order. The display handles this correctly via `index_of()` name lookup.
   Tests should not assert on ordering within a parallel stage.

7. **`Command::output()` is not cancel-safe in the Tokio sense.** If
   polled and then dropped, the child process is killed (via
   `kill_on_drop`). This is correct behavior for us — we want cancellation
   to kill children. But don't try to resume a dropped `Command::output()`
   future.

8. **`crossterm::execute!` can fail.** If stdout is redirected to a file
   or pipe, terminal control sequences are meaningless. `execute!` may
   return `Err`, but since we use `.ok()` (ignore errors), this is fine.
   The TTY detection in Step 6 ensures we only use `TtyDisplay` on real
   terminals — but be aware that unusual setups (SSH, screen, tmux) may
   report `is_terminal() = true` with limited terminal support.

9. **`std::process::exit(130)` for double-tap Ctrl+C.** This bypasses
   Rust's normal cleanup (drop impls, destructors). That's intentional —
   the user pressed Ctrl+C twice because normal shutdown is stuck. Don't
   try to clean up on the force path.

10. **Signal handler re-registration.** After the first `ctrl_c().await`
    resolves, you need a new `ctrl_c()` future to catch the second signal.
    The `tokio::spawn` in `shutdown()` creates this second listener.
    Don't try to reuse the first one.

11. **`TtyDisplay::index_of` panics on unknown names.** This is a
    programming error (the runner passed a name not in the config). In
    production, this would be a bug in the runner. The panic is intentional
    — it's louder than a silent ignore and easier to debug.

12. **`rendered_lines` must be reset in `run_cancelled()` and
    `run_finished()`.** If it's not reset, the next `run_started` will try
    to move the cursor up past the failure output / footer, corrupting
    the display. Setting `rendered_lines = 0` means the next `redraw()`
    or `run_started()` starts fresh.

---

## Definition of Done

- [x] `cargo build --release` produces a working binary.
- [x] `cargo test` passes — stage grouping, runner (sequential + parallel +
      mixed + skipping), config, watcher.
- [x] `cargo clippy -- -D warnings` is clean.
- [x] `cargo fmt --check` is clean.
- [ ] All 12 boxes in the Step 9 smoke-test checklist are verified.
- [x] `project-plan.md` Phase 3 checklist items are all checked:
  - [x] `pipeline/mod.rs` — group steps into stages by consecutive `parallel` flag
  - [x] `pipeline/runner.rs` — parallel stages via `tokio::task::JoinSet`
  - [x] Collect all parallel step results before deciding to continue
  - [x] `output/display.rs` — full-block redraw on each state change
  - [x] TTY detection at startup (`std::io::IsTerminal`)
  - [x] Non-TTY path emits append-only line output
  - [x] Graceful shutdown (Ctrl+C, double-tap force-exit)
  - [x] Mid-run cancel+restart on new file events
- [ ] Phase 3 commit(s) pushed to `phase-3` branch, ready to merge into
      `main`.

Once all boxes are checked, Phase 4 (terminal polish — colors, spinners,
output truncation, verbosity) is ready to start.
