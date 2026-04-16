# Phase 4 — Terminal Polish

> Implementation guide for Phase 4. Covers colors, spinners, right-aligned
> layout, verbosity levels, output truncation, startup banner, and idle footer.
>
> **Prerequisite reading:** [`ux-design.md`](./ux-design.md) (visual language,
> state mockups), [`project-plan.md`](./project-plan.md) (Phase 4 checklist).

---

## Table of Contents

1. [Overview](#overview)
2. [New Files and Structural Changes](#new-files-and-structural-changes)
3. [Step 1 — Color Infrastructure (`output/style.rs`)](#step-1--color-infrastructure)
4. [Step 2 — Apply Colors to TTY Display](#step-2--apply-colors-to-tty-display)
5. [Step 3 — Apply Colors to Plain Display](#step-3--apply-colors-to-plain-display)
6. [Step 4 — Right-Aligned Layout](#step-4--right-aligned-layout)
7. [Step 5 — Spinner Animation](#step-5--spinner-animation)
8. [Step 6 — Startup Banner and Idle Footer](#step-6--startup-banner-and-idle-footer)
9. [Step 7 — Verbosity Levels](#step-7--verbosity-levels)
10. [Step 8 — Output Truncation and Log File](#step-8--output-truncation-and-log-file)
11. [Step 9 — `--no-clear` Flag](#step-9---no-clear-flag)
12. [Step 10 — Wire It All Together in `lib.rs`](#step-10--wire-it-all-together)
13. [Testing Strategy](#testing-strategy)
14. [Commit Plan](#commit-plan)

---

## Overview

Phase 4 transforms the structural-but-plain output from Phase 3 into the
polished UX described in `ux-design.md`. No behavioral changes to the pipeline,
watcher, or config — this phase is purely about what the user sees.

**Deliverables:**

| Item | Files Touched |
|---|---|
| Color infrastructure + `NO_COLOR` | new `output/style.rs`, `output/mod.rs` |
| Colored TTY output | `output/display.rs` |
| Colored Plain output | `output/display.rs` |
| Right-aligned durations | `output/display.rs` |
| Spinner animation | `output/display.rs`, `output/mod.rs`, `lib.rs` |
| Startup banner + idle footer | `output/display.rs`, `lib.rs` |
| Verbosity levels (`-q`, `-v`, `-vv`) | `main.rs`, `lib.rs`, `output/display.rs`, `output/mod.rs` |
| Output truncation + log file | `output/display.rs`, `output/mod.rs` |
| `--no-clear` flag | `main.rs`, `lib.rs`, `output/display.rs` |

---

## New Files and Structural Changes

### New file: `src/output/style.rs`

Houses the `Theme` struct, the `Styled` helper enum, and a `visible_len()`
utility for measuring strings that contain ANSI escape codes.

### Changes to `src/output/mod.rs`

- `pub mod style;`
- Add `tick()` method to the `Display` trait (default no-op).
- Add `run_started` parameter changes to pass `DisplayConfig` context.

### Changes to `src/main.rs`

- Add `--verbose` (`-v`, count), `--quiet` (`-q`), `--no-clear`,
  `--summarize`, `--no-summarize` flags.
- Derive `Verbosity` enum from flags.
- Pass `DisplayConfig` to `App`.

### Changes to `src/lib.rs`

- Thread `DisplayConfig` into display constructors.
- Integrate spinner interval into the event loop.
- Pass banner/footer info through display lifecycle.

---

## Step 1 — Color Infrastructure

Create `src/output/style.rs`. This is the foundation everything else builds on.

### `NO_COLOR` detection

The [no-color.org](https://no-color.org/) standard: if the `NO_COLOR`
environment variable is set to **any value** (including empty string), all
color output must be suppressed.

Color is also suppressed when `--no-tty` is active or stdout is not a terminal,
**unless** a hypothetical `--force-color` flag is provided (not in scope for
Phase 4 — just keep the door open).

```rust
// src/output/style.rs

use crossterm::style::{StyledContent, Stylize};
use std::fmt;

/// Decides once at startup whether color output is enabled.
pub fn should_color(is_tty: bool) -> bool {
    is_tty && std::env::var_os("NO_COLOR").is_none()
}
```

### The `Styled` enum — zero-allocation conditional styling

The key problem: `"✓".green().bold()` returns `StyledContent<&str>` which
**always** emits ANSI escapes. We need a way to skip escapes when color is off
without heap-allocating (`Box<dyn Display>`).

```rust
/// A piece of text that is either styled (with ANSI escapes) or plain.
/// Implements `Display` so it can be used directly in `format!()` / `write!()`.
pub enum Styled<'a> {
    Plain(&'a str),
    Colored(StyledContent<&'a str>),
}

impl fmt::Display for Styled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Styled::Plain(s) => f.write_str(s),
            Styled::Colored(sc) => write!(f, "{sc}"),
        }
    }
}
```

### The `Theme` struct — centralized style decisions

```rust
/// Resolved once at startup. Passed into display constructors.
/// All styling decisions go through Theme so NO_COLOR is respected everywhere.
pub struct Theme {
    color: bool,
}

impl Theme {
    pub fn new(color: bool) -> Self {
        Self { color }
    }

    /// Conditionally apply a styling function. When color is off, returns plain text.
    pub fn style<'a>(
        &self,
        text: &'a str,
        apply: fn(&'a str) -> StyledContent<&'a str>,
    ) -> Styled<'a> {
        if self.color {
            Styled::Colored(apply(text))
        } else {
            Styled::Plain(text)
        }
    }

    // ── Convenience methods for the glyphs and text used throughout ──

    pub fn pass_glyph(&self) -> Styled<'static> {
        self.style("✓", |s| s.green().bold())
    }

    pub fn fail_glyph(&self) -> Styled<'static> {
        self.style("✗", |s| s.red().bold())
    }

    pub fn running_glyph(&self, frame: char) -> Styled<'static> {
        // frame is a braille spinner char — we leak a &'static str
        // because the set of 10 frames is fixed. See SPINNER_FRAMES.
        // Actually, we can't leak per-call. Use a different approach:
        // return a String-based wrapper. See note below.
        //
        // For the spinner we'll use a different path — see Step 5.
        self.style("⟳", |s| s.yellow())
    }

    pub fn skip_glyph(&self) -> Styled<'static> {
        self.style("⊘", |s| s.dark_grey())
    }

    pub fn queued_glyph(&self) -> Styled<'static> {
        self.style("·", |s| s.dark_grey())
    }

    pub fn dim<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.dim())
    }

    pub fn cyan<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.cyan())
    }

    pub fn red<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.red())
    }

    pub fn green<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.green())
    }

    pub fn bold<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.bold())
    }
}
```

### `visible_len()` — measure display width excluding ANSI escapes

Needed for right-alignment when the left portion of a line contains color codes.

```rust
/// Returns the visible character count of a string, stripping ANSI escape
/// sequences (CSI sequences of the form ESC [ ... m).
pub fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            len += 1;
        }
    }
    len
}
```

### Full `src/output/style.rs`

```rust
use crossterm::style::{StyledContent, Stylize};
use std::fmt;

// ── Color detection ─────────────────────────────────────────────────────────

pub fn should_color(is_tty: bool) -> bool {
    is_tty && std::env::var_os("NO_COLOR").is_none()
}

// ── Conditional styling ─────────────────────────────────────────────────────

pub enum Styled<'a> {
    Plain(&'a str),
    Colored(StyledContent<&'a str>),
}

impl fmt::Display for Styled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Styled::Plain(s) => f.write_str(s),
            Styled::Colored(sc) => write!(f, "{sc}"),
        }
    }
}

pub struct Theme {
    color: bool,
}

impl Theme {
    pub fn new(color: bool) -> Self {
        Self { color }
    }

    pub fn color_enabled(&self) -> bool {
        self.color
    }

    pub fn style<'a>(
        &self,
        text: &'a str,
        apply: fn(&'a str) -> StyledContent<&'a str>,
    ) -> Styled<'a> {
        if self.color {
            Styled::Colored(apply(text))
        } else {
            Styled::Plain(text)
        }
    }

    pub fn pass_glyph(&self) -> Styled<'static> {
        self.style("✓", |s| s.green().bold())
    }

    pub fn fail_glyph(&self) -> Styled<'static> {
        self.style("✗", |s| s.red().bold())
    }

    pub fn skip_glyph(&self) -> Styled<'static> {
        self.style("⊘", |s| s.dark_grey())
    }

    pub fn queued_glyph(&self) -> Styled<'static> {
        self.style("·", |s| s.dark_grey())
    }

    pub fn dim<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.dim())
    }

    pub fn cyan<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.cyan())
    }

    pub fn red<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.red())
    }

    pub fn green<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.green())
    }

    pub fn bold<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.bold())
    }
}

// ── ANSI-aware string measurement ───────────────────────────────────────────

pub fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            len += 1;
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn visible_len_plain() {
        assert_eq!(visible_len("hello"), 5);
    }

    #[test]
    fn visible_len_with_ansi() {
        // Simulate a green bold "✓": ESC[32;1m✓ESC[0m
        let styled = format!("{}", "✓".green().bold());
        assert_eq!(visible_len(&styled), 1);
    }

    #[test]
    fn visible_len_empty() {
        assert_eq!(visible_len(""), 0);
    }

    #[test]
    fn styled_plain_has_no_escapes() {
        let theme = Theme::new(false);
        let s = format!("{}", theme.pass_glyph());
        assert_eq!(s, "✓");
        assert!(!s.contains('\x1b'));
    }

    #[test]
    fn styled_colored_has_escapes() {
        let theme = Theme::new(true);
        let s = format!("{}", theme.pass_glyph());
        assert!(s.contains('\x1b'), "expected ANSI escapes in: {s:?}");
    }
}
```

---

## Step 2 — Apply Colors to TTY Display

Update `TtyDisplay` to accept a `Theme` and use it throughout `redraw()` and
`run_finished()`.

### Constructor change

```rust
pub struct TtyDisplay {
    theme: Theme,
    step_names: Vec<String>,
    statuses: Vec<StepStatus>,
    name_width: usize,
    rendered_lines: u16,
    spinner_frame: usize,   // added in Step 5
    has_running: bool,       // added in Step 5
    verbosity: Verbosity,    // added in Step 7
    no_clear: bool,          // added in Step 9
}

impl TtyDisplay {
    pub fn new(theme: Theme, verbosity: Verbosity, no_clear: bool) -> Self {
        Self {
            theme,
            step_names: Vec::new(),
            statuses: Vec::new(),
            name_width: 0,
            rendered_lines: 0,
            spinner_frame: 0,
            has_running: false,
            verbosity,
            no_clear,
        }
    }
}
```

### Colored `redraw()`

The full `redraw()` with colors and right-aligned durations. Each step line
follows this layout:

```
▸ format    ✓                                                          0.2s
▸ compile   ✗   warnings as errors                                     0.9s
▸ credo     ⠹                                                              
▸ test      ·                                                              
```

```rust
fn redraw(&mut self) {
    let mut stdout = std::io::stdout();
    let (cols, _) = crossterm::terminal::size().unwrap_or((80, 24));
    let width = cols as usize;

    // Erase previous block.
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
        let (glyph, diagnostic) = match &self.statuses[i] {
            StepStatus::Queued => {
                (format!("{}", self.theme.queued_glyph()), String::new())
            }
            StepStatus::Running => {
                let frame = SPINNER_FRAMES[self.spinner_frame];
                let g = if self.theme.color_enabled() {
                    format!("{}", format_args!("{}", frame).to_string().as_str().yellow())
                } else {
                    frame.to_string()
                };
                (g, String::new())
            }
            StepStatus::Passed(d) => {
                (format!("{}", self.theme.pass_glyph()), format!("{:.1}s", d.as_secs_f64()))
            }
            StepStatus::Failed(d, diag) => {
                let d_str = format!("{:.1}s", d.as_secs_f64());
                let diag_str = if diag.is_empty() {
                    String::new()
                } else {
                    format!("   {}", self.theme.dim(diag))
                };
                (format!("{}", self.theme.fail_glyph()), /* see layout below */)
            }
            StepStatus::Skipped => {
                let g = format!("{}", self.theme.skip_glyph());
                let d = format!("{}", self.theme.dim("skipped"));
                (g, d)
            }
        };

        // Build left portion: "▸ name    glyph   diagnostic"
        let left = format!("▸ {:width$}  {glyph}", name, width = self.name_width);

        // Build right portion: duration, dim, right-aligned
        let right = if !duration_str.is_empty() {
            format!("{}", self.theme.dim(&duration_str))
        } else {
            String::new()
        };

        // Compose full line with right-alignment
        if right.is_empty() {
            println!("{left}");
        } else {
            let left_vis = visible_len(&left);
            let right_vis = visible_len(&right);
            let pad = width.saturating_sub(left_vis + right_vis);
            println!("{left}{:>pad$}{right}", "", pad = pad);
        }

        lines += 1;
    }

    self.rendered_lines = lines;
    let _ = std::io::Write::flush(&mut stdout);
}
```

> **Note:** The above is a structural sketch. The actual implementation should
> cleanly separate the glyph, diagnostic, and duration into variables, then
> compose them. The sketch shows the pattern — adjust variable names during
> implementation.

### Colored `run_finished()` — failure output blocks and footer

```rust
fn run_finished(&mut self, results: &[StepResult]) {
    self.rendered_lines = 0;
    self.has_running = false;

    let failures: Vec<_> = results.iter().filter(|r| !r.success).collect();
    if !failures.is_empty() {
        println!();
        for r in &failures {
            // Cyan section divider: ── credo output ──────────...
            let header = format!("── {} output ", r.name);
            let fill = "─".repeat(60usize.saturating_sub(header.len()));
            println!("{}{}", self.theme.cyan(&header), self.theme.cyan(&fill));

            // Indented output (2 spaces)
            for line in r.stdout.lines() {
                println!("  {line}");
            }
            for line in r.stderr.lines() {
                println!("  {line}");
            }
        }
    }

    // Footer
    let failed = results.iter().filter(|r| !r.success).count();
    let passed = results.iter().filter(|r| r.success).count();
    let skipped = self.step_names.len().saturating_sub(results.len());
    let total: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();

    println!();

    let mut parts: Vec<String> = Vec::new();
    if failed > 0 {
        parts.push(format!("{}", self.theme.red(&format!("{failed} failed"))));
    }
    parts.push(format!("{}", self.theme.green(&format!("{passed} passed"))));
    if skipped > 0 {
        parts.push(format!("{}", self.theme.dim(&format!("{skipped} skipped"))));
    }
    parts.push(format!("{}", self.theme.dim(&format!("{total:.1}s"))));

    println!("{}", parts.join(" · "));
    let _ = std::io::Write::flush(&mut std::io::stdout());
}
```

> **Lifetime note:** The `self.theme.red(...)` methods take `&str`, but
> `format!("{failed} failed")` creates a temporary `String`. You'll need to
> bind these to `let` variables so the borrows live long enough:
> ```rust
> let failed_str = format!("{failed} failed");
> let styled_failed = self.theme.red(&failed_str);
> parts.push(format!("{styled_failed}"));
> ```

### Run divider with timestamp

Per the UX design, each run starts with a timestamp divider:

```
━━━ 14:32:08 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Add this to `run_started()`:

```rust
fn run_started(&mut self, step_names: &[String]) {
    // ... existing field resets ...

    let mut stdout = std::io::stdout();

    // Clear screen (unless --no-clear)
    if !self.no_clear {
        execute!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )
        .ok();
    }

    // Timestamp divider
    let now = chrono::Local::now().format("%H:%M:%S");
    let (cols, _) = crossterm::terminal::size().unwrap_or((80, 24));
    let prefix = format!("━━━ {now} ");
    let fill = "━".repeat((cols as usize).saturating_sub(prefix.len()));
    println!("{}", self.theme.dim(&format!("{prefix}{fill}")));

    self.redraw();
}
```

> **Dependency note:** This uses `chrono` for timestamp formatting. Add to
> `Cargo.toml`:
> ```toml
> chrono = { version = "0.4", default-features = false, features = ["clock"] }
> ```
> Alternatively, use `std::time::SystemTime` and manual formatting to avoid
> the dependency. A minimal approach without chrono:
> ```rust
> use std::time::SystemTime;
> fn hms_now() -> String {
>     let secs = SystemTime::now()
>         .duration_since(SystemTime::UNIX_EPOCH)
>         .unwrap()
>         .as_secs();
>     let local_secs = secs; // UTC — for true local time, chrono is needed
>     let h = (local_secs % 86400) / 3600;
>     let m = (local_secs % 3600) / 60;
>     let s = local_secs % 60;
>     format!("{h:02}:{m:02}:{s:02}")
> }
> ```
> **Recommendation:** Use `chrono` with minimal features. It's a well-known
> crate and local timezone formatting is non-trivial without it.

---

## Step 3 — Apply Colors to Plain Display

`PlainDisplay` is for non-TTY contexts (piped, CI, `--no-tty`). Per the UX
design, **no color by default** in non-TTY mode. However, `PlainDisplay`
should still accept a `Theme` for potential future `--force-color` support.

```rust
pub struct PlainDisplay {
    theme: Theme,
    verbosity: Verbosity,
}

impl PlainDisplay {
    pub fn new(theme: Theme, verbosity: Verbosity) -> Self {
        Self { theme, verbosity }
    }
}
```

Since `should_color()` returns `false` when `is_tty` is `false`, the `Theme`
will produce plain (no-ANSI) output automatically. The `PlainDisplay` code
doesn't change much — it already uses plain text. Just swap the hardcoded
glyphs for `self.theme.pass_glyph()` etc. so the code path is unified.

The `PlainDisplay` also adds timestamps per the UX design:

```
[14:32:08] run started
[14:32:08] format: pass (0.2s)
[14:32:09] compile: pass (1.1s)
[14:32:11] credo: FAIL (3 issues, 1.8s)
```

---

## Step 4 — Right-Aligned Layout

Durations are right-aligned to the terminal edge, rendered dim.

### Terminal width

```rust
fn term_width() -> usize {
    crossterm::terminal::size().map(|(c, _)| c as usize).unwrap_or(80)
}
```

Cache this per-redraw (not per-line) since `size()` is a syscall:

```rust
fn redraw(&mut self) {
    let width = term_width();
    // ... use `width` for all lines in this redraw ...
}
```

### Layout math

Each step line has three visual zones:

```
▸ format    ✓   3 issues                                              1.8s
|-- left ---|--- mid ----|------------- padding -------------|-- right --|
```

```rust
let left = format!("▸ {:nw$}  {glyph}", name, nw = self.name_width);
let mid = diagnostic_str; // e.g. "3 issues" or ""
let right = duration_str; // e.g. "1.8s" or ""

let left_part = if mid.is_empty() {
    left
} else {
    format!("{left}   {mid}")
};

let left_vis = visible_len(&left_part);
let right_vis = visible_len(&right);
let pad = width.saturating_sub(left_vis + right_vis);

println!("{left_part}{:>pad$}{right}", "", pad = pad);
```

### Short diagnostics for failures

The `StepStatus::Failed` variant should carry an optional short diagnostic
string alongside the duration. This requires a small change to `StepStatus`:

```rust
enum StepStatus {
    Queued,
    Running,
    Passed(Duration),
    Failed(Duration, String),  // (duration, short diagnostic)
    Skipped,
}
```

The short diagnostic is derived from the step's output. For now, a simple
heuristic: count non-empty lines in combined stdout+stderr and report
`"N issues"` or `"N warnings"` or `"exit code N"` as a fallback.

```rust
fn short_diagnostic(result: &StepResult) -> String {
    if result.success {
        return String::new();
    }

    let combined = format!("{}{}", result.stdout, result.stderr);
    let non_empty_lines = combined.lines().filter(|l| !l.trim().is_empty()).count();

    match result.exit_code {
        None => "command not found".into(),
        Some(code) if non_empty_lines > 0 => {
            if non_empty_lines == 1 {
                // Use the single line as the diagnostic (truncated)
                let line = combined.lines().find(|l| !l.trim().is_empty()).unwrap();
                let truncated: String = line.chars().take(40).collect();
                if line.len() > 40 {
                    format!("{truncated}...")
                } else {
                    truncated
                }
            } else {
                format!("{non_empty_lines} lines of output")
            }
        }
        Some(code) => format!("exit code {code}"),
    }
}
```

---

## Step 5 — Spinner Animation

The spinner is a braille-dot animation that plays for steps in the `Running`
state. All running spinners share a single frame clock so they advance in
lockstep.

### Constants

```rust
const SPINNER_FRAMES: &[char] = &['⠋', '⠙', '⠹', '⠸', '⠼', '⠴', '⠦', '⠧', '⠇', '⠏'];
const SPINNER_INTERVAL_MS: u64 = 80;
```

### Display trait addition

```rust
// In src/output/mod.rs

pub trait Display: Send {
    fn run_started(&mut self, step_names: &[String]);
    fn step_running(&mut self, name: &str);
    fn step_finished(&mut self, result: &StepResult);
    fn steps_skipped(&mut self, names: &[String]);
    fn run_cancelled(&mut self);
    fn run_finished(&mut self, results: &[StepResult]);

    /// Advance the spinner animation by one frame. Called every 80ms by the
    /// event loop. Only redraws if there are steps in the Running state.
    /// Default is a no-op (PlainDisplay doesn't animate).
    fn tick(&mut self) {}
}
```

### TtyDisplay implementation

```rust
impl TtyDisplay {
    pub fn tick(&mut self) {
        if self.has_running {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
            self.redraw();
        }
    }
}
```

Update `step_running()` to track `has_running`:

```rust
fn step_running(&mut self, name: &str) {
    let idx = self.index_of(name);
    self.statuses[idx] = StepStatus::Running;
    self.has_running = true;
    self.redraw();
}
```

Update `step_finished()` to recompute `has_running`:

```rust
fn step_finished(&mut self, result: &StepResult) {
    let idx = self.index_of(&result.name);
    let diag = short_diagnostic(result);
    self.statuses[idx] = if result.success {
        StepStatus::Passed(result.duration)
    } else {
        StepStatus::Failed(result.duration, diag)
    };
    self.has_running = self.statuses.iter().any(|s| matches!(s, StepStatus::Running));
    self.redraw();
}
```

### Event loop integration (`lib.rs`)

The spinner tick must run concurrently with the pipeline. The cleanest
approach: wrap the pipeline execution in an async block that also services
spinner ticks via a nested `tokio::select!`.

```rust
// In App::run(), replace the bare pipeline::run_pipeline call:

use tokio::time::{interval, Duration, MissedTickBehavior};

// Only create spinner interval for TTY mode
let spinner_interval = if is_tty {
    Some(Duration::from_millis(80))
} else {
    None
};

// Inside the main event loop, the pipeline arm becomes:
result = run_with_spinner(
    &self.config,
    &self.root,
    display.as_mut(),
    spinner_interval,
) => RunOutcome::Completed(result),
```

The helper function:

```rust
async fn run_with_spinner(
    config: &config::Config,
    root: &std::path::Path,
    display: &mut dyn Display,
    spinner_interval: Option<Duration>,
) -> anyhow::Result<Vec<StepResult>> {
    match spinner_interval {
        None => pipeline::run_pipeline(config, root, display).await,
        Some(dur) => {
            let mut ticker = interval(dur);
            ticker.set_missed_tick_behavior(MissedTickBehavior::Skip);

            let pipeline_fut = pipeline::run_pipeline(config, root, display);
            tokio::pin!(pipeline_fut);

            loop {
                tokio::select! {
                    biased;
                    result = &mut pipeline_fut => break result,
                    _ = ticker.tick() => display.tick(),
                }
            }
        }
    }
}
```

**Why this works:** `display` is `&mut dyn Display`. Both the pipeline and the
ticker need mutable access. Because `tokio::select!` is cooperative (only one
branch runs at a time), there's no data race — the pipeline yields at `.await`
points, and the ticker's `display.tick()` runs in between. The `biased;`
keyword ensures the pipeline result is checked first, so a completed pipeline
isn't delayed by a pending tick.

**Why not `Arc<Mutex<>>`:** Unnecessary complexity. The `select!` approach
keeps `display` as a plain `&mut`, matching the existing code.

---

## Step 6 — Startup Banner and Idle Footer

### Startup banner (S1 from UX design)

Shown once at launch, before the first pipeline run. In TTY mode, it's
displayed briefly then cleared when the first run starts.

```
━━━ baraddur 0.1.0 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
watching: /Users/alice/code/my-project
config:   .baraddur.toml  (4 steps)
press ^C to exit
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Add a `banner()` method to the `Display` trait:

```rust
/// Show the startup banner. Called once before the first pipeline run.
fn banner(&mut self, root: &Path, config_path: &Path, step_count: usize) {}
```

TTY implementation:

```rust
fn banner(&mut self, root: &Path, config_path: &Path, step_count: usize) {
    let mut stdout = std::io::stdout();
    execute!(stdout, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0)).ok();

    let width = term_width();
    let version = env!("CARGO_PKG_VERSION");
    let prefix = format!("━━━ baraddur {version} ");
    let fill = "━".repeat(width.saturating_sub(prefix.len()));
    println!("{}", self.theme.dim(&format!("{prefix}{fill}")));

    let config_name = config_path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_default();
    println!(
        "{}  {}",
        self.theme.dim("watching:"),
        root.display()
    );
    println!(
        "{}    {}  ({step_count} steps)",
        self.theme.dim("config:"),
        config_name
    );
    println!("{}", self.theme.dim("press ^C to exit"));

    let bottom = "━".repeat(width);
    println!("{}", self.theme.dim(&bottom));

    let _ = std::io::Write::flush(&mut stdout);
}
```

### Idle footer (S2 / S5 from UX design)

After a successful run completes and the display shows all-pass:

```
all passing · 142 files watched · last run 1.4s
```

After a run with failures:

```
1 failed · 3 passed · 5.4s
```

The footer is already partially implemented in `run_finished()`. Enhance it
to include the idle-state information. The "N files watched" count can be
passed in from `lib.rs` or omitted for Phase 4 (it requires querying the
watcher, which is on a separate thread — defer to a later enhancement).

Simplified footer for Phase 4:

```rust
// At the end of run_finished():
println!();
if failed == 0 {
    let msg = format!("all passing · {total:.1}s");
    println!("{}", self.theme.dim(&msg));
} else {
    // colored footer (already shown above)
}
```

---

## Step 7 — Verbosity Levels

### CLI flags (`main.rs`)

```rust
use clap::ArgAction;

#[derive(Parser, Debug)]
#[command(name = "baraddur", version, about = "...")]
struct Cli {
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    #[arg(short = 'w', long)]
    watch_dir: Option<PathBuf>,

    #[arg(long)]
    no_tty: bool,

    /// Don't clear screen between runs
    #[arg(long)]
    no_clear: bool,

    /// Increase verbosity (-v show passing output, -vv debug)
    #[arg(short = 'v', long, action = ArgAction::Count, conflicts_with = "quiet")]
    verbose: u8,

    /// Only show failures (no step list, no footer on success)
    #[arg(short = 'q', long, conflicts_with = "verbose")]
    quiet: bool,
}
```

### Verbosity enum

Define in `src/output/mod.rs` (or a shared types module):

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    Quiet,    // -q
    Normal,   // default
    Verbose,  // -v
    Debug,    // -vv
}
```

Derive from CLI flags:

```rust
impl Cli {
    fn verbosity(&self) -> Verbosity {
        if self.quiet {
            Verbosity::Quiet
        } else {
            match self.verbose {
                0 => Verbosity::Normal,
                1 => Verbosity::Verbose,
                _ => Verbosity::Debug,
            }
        }
    }
}
```

### Behavior per verbosity level

| Level | Step list | Passing output | Failure output | Footer | Debug events |
|---|---|---|---|---|---|
| **Quiet** | Hidden | Hidden | Shown | Only on failure | Hidden |
| **Normal** | Shown | Hidden | Shown | Always | Hidden |
| **Verbose** | Shown | Shown (indented) | Shown | Always | Hidden |
| **Debug** | Shown | Shown | Shown | Always | Shown (prefixed `[debug]`) |

### Implementation in display

Gate output in `run_finished()` and `redraw()`:

```rust
// In run_finished():

// Show passing step output in Verbose+ mode
if self.verbosity >= Verbosity::Verbose {
    for r in results.iter().filter(|r| r.success) {
        if !r.stdout.is_empty() {
            println!("── {} output ──", r.name);
            for line in r.stdout.lines() {
                println!("  {line}");
            }
        }
    }
}

// In Quiet mode, skip the step list entirely (don't call redraw)
// and only print failure output + footer if there were failures.
```

For Quiet mode, override `run_started()`, `step_running()`, `step_finished()`,
and `steps_skipped()` to be no-ops. Only `run_finished()` produces output,
and only when there are failures:

```rust
fn run_started(&mut self, step_names: &[String]) {
    // Still store names for internal tracking
    self.step_names = step_names.to_vec();
    self.statuses = vec![StepStatus::Queued; step_names.len()];
    self.name_width = step_names.iter().map(|n| n.len()).max().unwrap_or(0);
    self.rendered_lines = 0;

    if self.verbosity == Verbosity::Quiet {
        return; // no visual output
    }

    // ... normal clear + divider + redraw ...
}
```

### Debug events

In `Debug` mode, the event loop in `lib.rs` should print internal events:

```rust
if verbosity == Verbosity::Debug {
    eprintln!("[debug] file change detected, restarting pipeline");
}
```

These go to stderr so they don't interfere with stdout formatting.

---

## Step 8 — Output Truncation and Log File

### Capture limits

Cap each step's captured output at **100 KiB** (102,400 bytes). This is
enforced in `pipeline/step.rs` when reading stdout/stderr:

```rust
const MAX_CAPTURE_BYTES: usize = 100 * 1024; // 100 KiB

pub async fn run(step: &Step, cwd: &Path) -> Result<StepResult> {
    // ... existing command setup ...

    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .output()
        .await;

    let duration = start.elapsed();

    let result = match output {
        Ok(out) => {
            let stdout = truncate_capture(&out.stdout);
            let stderr = truncate_capture(&out.stderr);
            StepResult {
                name: step.name.clone(),
                success: out.status.success(),
                exit_code: out.status.code(),
                stdout,
                stderr,
                duration,
            }
        }
        // ... error case unchanged ...
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
```

### Display truncation

When showing failure output, if a step's output exceeds 50 lines, show the
first 25 and last 25 with an elision marker:

```rust
fn print_truncated_output(output: &str, theme: &Theme) {
    let lines: Vec<&str> = output.lines().collect();
    let max_display_lines = 50;
    let context_lines = 25;

    if lines.len() <= max_display_lines {
        for line in &lines {
            println!("  {line}");
        }
    } else {
        for line in &lines[..context_lines] {
            println!("  {line}");
        }
        let elided = lines.len() - (context_lines * 2);
        let msg = format!("... [{elided} lines elided — see .baraddur/last-run.log] ...");
        println!("  {}", theme.dim(&msg));
        for line in &lines[lines.len() - context_lines..] {
            println!("  {line}");
        }
    }
}
```

### Log file (`.baraddur/last-run.log`)

After each pipeline run, write all step output (full, not truncated) to
`.baraddur/last-run.log` relative to the watch root. Create the `.baraddur/`
directory if it doesn't exist.

This is best done in `lib.rs` after the pipeline completes:

```rust
fn write_run_log(root: &Path, results: &[StepResult]) {
    let log_dir = root.join(".baraddur");
    if std::fs::create_dir_all(&log_dir).is_err() {
        return; // silent failure — log writing is best-effort
    }

    let log_path = log_dir.join("last-run.log");
    let mut content = String::new();

    for r in results {
        content.push_str(&format!("═══ {} ({}) ═══\n",
            r.name,
            if r.success { "pass" } else { "FAIL" }
        ));
        if !r.stdout.is_empty() {
            content.push_str(&r.stdout);
            if !r.stdout.ends_with('\n') {
                content.push('\n');
            }
        }
        if !r.stderr.is_empty() {
            content.push_str("--- stderr ---\n");
            content.push_str(&r.stderr);
            if !r.stderr.ends_with('\n') {
                content.push('\n');
            }
        }
        content.push('\n');
    }

    let _ = std::fs::write(&log_path, &content); // best-effort
}
```

> **`.gitignore` note:** Users should add `.baraddur/` to their project's
> `.gitignore`. We could mention this in error output if the directory is
> created for the first time, but that's optional polish.

---

## Step 9 — `--no-clear` Flag

Already partially handled by the `no_clear` field on `TtyDisplay`. The flag
prevents `terminal::Clear(ClearType::All)` in `run_started()`. The timestamp
divider and step list still print — they just append below the previous run
instead of clearing.

```rust
// In TtyDisplay::run_started():
if !self.no_clear {
    execute!(
        stdout,
        terminal::Clear(ClearType::All),
        cursor::MoveTo(0, 0)
    )
    .ok();
}
```

When `--no-clear` is active, the redraw still works (erase the step block,
reprint it) — only the full-screen clear before each run is suppressed. This
preserves scrollback history while still allowing the step block to update
in-place.

---

## Step 10 — Wire It All Together

### `DisplayConfig` struct

Thread all display-relevant settings through a single struct:

```rust
// In src/output/mod.rs (or src/lib.rs)

pub struct DisplayConfig {
    pub is_tty: bool,
    pub no_clear: bool,
    pub verbosity: Verbosity,
}
```

### Updated `App` struct

```rust
pub struct App {
    pub config: config::Config,
    pub config_path: PathBuf,
    pub root: PathBuf,
    pub display_config: DisplayConfig,
}
```

### Updated `main.rs`

```rust
#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    // ... config loading unchanged ...

    let is_tty = !cli.no_tty && std::io::stdout().is_terminal();

    let app = baraddur::App {
        config: loaded.config,
        config_path: loaded.config_path,
        root,
        display_config: baraddur::output::DisplayConfig {
            is_tty,
            no_clear: cli.no_clear,
            verbosity: cli.verbosity(),
        },
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

### Updated `App::run()` in `lib.rs`

```rust
impl App {
    pub async fn run(self) -> Result<()> {
        let dc = &self.display_config;
        let color = output::style::should_color(dc.is_tty);
        let theme = output::style::Theme::new(color);

        let mut display: Box<dyn Display> = if dc.is_tty {
            Box::new(TtyDisplay::new(theme, dc.verbosity, dc.no_clear))
        } else {
            Box::new(PlainDisplay::new(theme, dc.verbosity))
        };

        // Show startup banner
        display.banner(&self.root, &self.config_path, self.config.steps.len());

        // ... watcher setup unchanged ...

        let spinner_dur = if dc.is_tty {
            Some(Duration::from_millis(80))
        } else {
            None
        };

        loop {
            let outcome = tokio::select! {
                biased;
                _ = tokio::signal::ctrl_c() => RunOutcome::Shutdown,
                maybe = rx.recv() => match maybe {
                    Some(()) => RunOutcome::FileChange,
                    None => RunOutcome::WatcherDied,
                },
                result = run_with_spinner(
                    &self.config,
                    &self.root,
                    display.as_mut(),
                    spinner_dur,
                ) => RunOutcome::Completed(result),
            };

            match outcome {
                RunOutcome::Completed(result) => {
                    let results = result?;
                    write_run_log(&self.root, &results);
                    // idle-wait below
                }
                // ... other arms unchanged ...
            }

            // ... idle select! unchanged ...
        }
    }
}
```

---

## Testing Strategy

### Unit tests (`src/output/style.rs`)

Already sketched above in the `style.rs` listing:

- `visible_len` with plain text, ANSI-escaped text, and empty string
- `Styled::Plain` produces no ANSI escapes
- `Styled::Colored` produces ANSI escapes
- `should_color()` returns false when `NO_COLOR` is set

### Integration tests (`tests/runner.rs`)

The existing `RecordingDisplay` tests remain valid — they don't depend on
visual output. Add a new test to verify the `tick()` trait method has a
default no-op:

```rust
#[test]
fn recording_display_tick_is_noop() {
    let mut d = RecordingDisplay::default();
    d.tick(); // should not panic or add events
    assert!(d.events.is_empty());
}
```

### Manual testing checklist

- [ ] Run against a real project — verify colors appear correctly
- [ ] `NO_COLOR=1 cargo run` — verify no ANSI escapes in output
- [ ] `cargo run -- --no-tty` — verify plain append-only output
- [ ] `cargo run -- -q` — verify only failures shown
- [ ] `cargo run -- -v` — verify passing step output shown
- [ ] `cargo run -- -vv` — verify debug events on stderr
- [ ] `cargo run -- --no-clear` — verify scrollback preserved
- [ ] Trigger a mid-run restart — verify spinner stops cleanly
- [ ] Kill with Ctrl+C during spinner — verify clean exit
- [ ] Pipe output: `cargo run 2>&1 | cat` — verify non-TTY mode
- [ ] Check `.baraddur/last-run.log` is written after each run
- [ ] Resize terminal during a run — verify layout adapts on next redraw

---

## Commit Plan

Break Phase 4 into focused, reviewable commits:

1. **Add `output/style.rs` — color infrastructure and `NO_COLOR` support**
   - New file with `Theme`, `Styled`, `should_color()`, `visible_len()`
   - Tests for all of the above
   - No visual changes yet

2. **Add `Verbosity` enum and CLI flags**
   - Add `-v`, `-q`, `--no-clear` to `Cli`
   - Define `Verbosity` enum in `output/mod.rs`
   - Define `DisplayConfig` struct
   - Update `App` to accept `DisplayConfig`
   - Update display constructors to accept `Theme` and `Verbosity`
   - No behavioral changes to output yet

3. **Colored TTY output — glyphs, step lines, failure blocks, footer**
   - Apply `Theme` throughout `TtyDisplay::redraw()` and `run_finished()`
   - Right-aligned durations
   - Timestamp run divider
   - Short diagnostics on failure lines
   - Startup banner

4. **Colored Plain output**
   - Apply `Theme` to `PlainDisplay`
   - Add timestamps to plain output lines

5. **Spinner animation**
   - Add `tick()` to `Display` trait
   - Implement spinner in `TtyDisplay`
   - Integrate spinner interval into `lib.rs` event loop

6. **Verbosity gating**
   - Quiet mode: suppress non-error output
   - Verbose mode: show passing step output
   - Debug mode: log internal events to stderr

7. **Output truncation and log file**
   - Add `MAX_CAPTURE_BYTES` truncation in `pipeline/step.rs`
   - Display truncation (head+tail) in `output/display.rs`
   - Write `.baraddur/last-run.log` after each run

---

## Dependency Changes

Add to `Cargo.toml`:

```toml
chrono = { version = "0.4", default-features = false, features = ["clock"] }
```

No other new dependencies. Everything else (`crossterm`, `tokio`, `clap`) is
already present with sufficient features.
