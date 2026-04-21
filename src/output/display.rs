use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent};
use crossterm::{
    cursor, execute,
    terminal::{self, ClearType},
};

use super::style::{Theme, visible_len};
use super::{BrowseAction, Display, Verbosity};
use crate::pipeline::StepResult;

// ── Shared helpers ───────────────────────────────────────────────────────────

fn timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// Formats stdout+stderr with head+tail truncation if the output is long.
/// Returns a string with `  ` prefix on each line, ready to print.
fn format_truncated_output(stdout: &str, stderr: &str) -> String {
    let combined = if stderr.is_empty() {
        stdout.to_string()
    } else if stdout.is_empty() {
        stderr.to_string()
    } else if stdout.ends_with('\n') {
        format!("{stdout}{stderr}")
    } else {
        format!("{stdout}\n{stderr}")
    };

    let lines: Vec<&str> = combined.lines().collect();
    const MAX_DISPLAY_LINES: usize = 50;
    const CONTEXT_LINES: usize = 25;

    let mut out = String::new();
    if lines.len() <= MAX_DISPLAY_LINES {
        for line in &lines {
            out.push_str(&format!("  {line}\n"));
        }
    } else {
        for line in &lines[..CONTEXT_LINES] {
            out.push_str(&format!("  {line}\n"));
        }
        let elided = lines.len() - (CONTEXT_LINES * 2);
        out.push_str(&format!(
            "  ... [{elided} lines elided — see .baraddur/last-run.log] ...\n"
        ));
        for line in &lines[lines.len() - CONTEXT_LINES..] {
            out.push_str(&format!("  {line}\n"));
        }
    }
    out
}

/// Builds a short inline diagnostic from a failing step's output.
fn short_diagnostic(result: &StepResult) -> String {
    if result.success {
        return String::new();
    }
    match result.exit_code {
        None => "command not found".into(),
        Some(_) => {
            let combined = format!("{}{}", result.stdout, result.stderr);
            let non_empty: Vec<&str> = combined.lines().filter(|l| !l.trim().is_empty()).collect();
            match non_empty.len() {
                0 => String::new(),
                1 => {
                    let line = non_empty[0];
                    let truncated: String = line.chars().take(40).collect();
                    if line.chars().count() > 40 {
                        format!("{truncated}…")
                    } else {
                        truncated
                    }
                }
                n => format!("{n} lines"),
            }
        }
    }
}

/// Formats the trigger suffix for a run divider/header.
/// Single file → `"  ·  lib/foo.ex"`, multiple → `"  ·  3 files"`, none → `""`.
fn format_trigger_suffix(paths: Option<&[PathBuf]>) -> String {
    match paths {
        Some([p]) => format!("  ·  {}", p.display()),
        Some(ps) => format!("  ·  {} files", ps.len()),
        None => String::new(),
    }
}

// ── Non-TTY display (append-only) ───────────────────────────────────────────

/// Append-only line output for non-TTY contexts (piped, CI, `--no-tty`).
/// No cursor movement, no screen clearing.
pub struct PlainDisplay {
    theme: Theme,
    verbosity: Verbosity,
    trigger_paths: Option<Vec<PathBuf>>,
    run_start: Option<Instant>,
    run_count: usize,
}

impl PlainDisplay {
    pub fn new(theme: Theme, verbosity: Verbosity) -> Self {
        Self {
            theme,
            verbosity,
            trigger_paths: None,
            run_start: None,
            run_count: 0,
        }
    }
}

impl Display for PlainDisplay {
    fn set_trigger(&mut self, paths: &[PathBuf]) {
        self.trigger_paths = Some(paths.to_vec());
    }

    fn banner(&mut self, root: &Path, config_path: &Path, _step_count: usize) {
        eprintln!(
            "baraddur: watching {}\n          (config: {})",
            root.display(),
            config_path.display(),
        );
    }

    fn run_started(&mut self, _step_names: &[String]) {
        self.run_start = Some(Instant::now());
        self.run_count += 1;
        if self.verbosity != Verbosity::Quiet {
            let trigger = self.trigger_paths.take();
            let suffix = format_trigger_suffix(trigger.as_deref());
            println!("[{}] run #{} started{suffix}", timestamp(), self.run_count);
        }
    }

    fn step_running(&mut self, name: &str) {
        if self.verbosity != Verbosity::Quiet {
            println!("[{}] ▸ {} running", timestamp(), name);
        }
    }

    fn step_finished(&mut self, result: &StepResult) {
        if self.verbosity == Verbosity::Quiet && result.success {
            return;
        }
        let status = if result.success {
            format!("{}", self.theme.pass_glyph())
        } else {
            format!("{}", self.theme.fail_glyph())
        };
        println!(
            "[{}] ▸ {}  {}  ({:.1}s)",
            timestamp(),
            result.name,
            status,
            result.duration.as_secs_f64()
        );
    }

    fn steps_skipped(&mut self, names: &[String]) {
        if self.verbosity != Verbosity::Quiet {
            let ts = timestamp();
            for name in names {
                println!("[{ts}] ▸ {name}  {}  skipped", self.theme.skip_glyph());
            }
        }
    }

    fn run_cancelled(&mut self) {
        if self.verbosity != Verbosity::Quiet {
            println!("[{}] run cancelled", timestamp());
        }
    }

    fn run_finished(&mut self, results: &[StepResult]) {
        let ts = timestamp();

        // Print failure output blocks.
        for r in results.iter().filter(|r| !r.success) {
            println!("[{ts}] --- {} output ---", r.name);
            print!("{}", format_truncated_output(&r.stdout, &r.stderr));
        }

        // In verbose mode, also show passing step output.
        if self.verbosity >= Verbosity::Verbose {
            for r in results.iter().filter(|r| r.success) {
                if !r.stdout.is_empty() {
                    println!("[{ts}] --- {} output ---", r.name);
                    for line in r.stdout.lines() {
                        println!("  {line}");
                    }
                }
            }
        }

        let failed = results.iter().filter(|r| !r.success).count();
        let passed = results.iter().filter(|r| r.success).count();
        let elapsed = self
            .run_start
            .take()
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or_else(|| results.iter().map(|r| r.duration.as_secs_f64()).sum());

        if self.verbosity != Verbosity::Quiet || failed > 0 {
            println!("[{ts}] run complete: {failed} failed, {passed} passed, {elapsed:.1}s");
        }

        let _ = std::io::stdout().flush();
    }
}

// ── TTY display (full-block redraw) ─────────────────────────────────────────

const SPINNER_FRAMES: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

/// Status of a single step, tracked by the display for redraw.
#[derive(Debug, Clone)]
enum StepStatus {
    Queued,
    Running,
    Passed(Duration),
    Failed(Duration, String), // (duration, short diagnostic)
    Skipped,
}

/// Interactive terminal display. On each state change, erases the previous
/// step-status block and reprints it.
pub struct TtyDisplay {
    theme: Theme,
    verbosity: Verbosity,
    no_clear: bool,
    step_names: Vec<String>,
    statuses: Vec<StepStatus>,
    name_width: usize,
    /// How many lines the last `redraw()` or `browse_redraw()` printed.
    rendered_lines: u16,
    spinner_frame: usize,
    has_running: bool,
    /// Original termios saved on construction so we can restore on drop.
    /// Suppressing echo prevents typed characters from corrupting the redrawn
    /// step-status block while a pipeline is running.
    #[cfg(unix)]
    original_termios: Option<libc::termios>,
    // ── Browse mode state ────────────────────────────────────────────────────
    /// Pre-formatted output per step, captured in `run_finished`.
    step_outputs: Vec<String>,
    /// Whether each step's output is shown inline in browse mode.
    expanded: Vec<bool>,
    /// Tracks the `O` toggle: true when all steps are expanded.
    all_expanded: bool,
    /// Index of the currently highlighted row.
    cursor: usize,
    /// True while in the post-run interactive navigation state.
    browse_active: bool,
    /// Last key code pressed — used for `gg` double-tap detection.
    last_key: Option<KeyCode>,
    /// Whether raw mode is currently enabled (used by Drop for cleanup).
    raw_mode_active: bool,
    /// File(s) that triggered this run. Set by `set_trigger`, consumed by `run_started`.
    trigger_paths: Option<Vec<PathBuf>>,
    /// Monotonically increasing counter incremented on each `run_started`.
    run_count: usize,
    /// Plain (unstyled) divider text from `run_started`. Printed as the first line
    /// of every `redraw()` and `browse_redraw()`, colored live from `statuses`.
    run_divider: String,
    /// Wall-clock start time of the current run, for accurate elapsed time in the footer.
    run_start: Option<Instant>,
    /// Pre-formatted summary line from `run_finished`, shown persistently in browse mode.
    run_summary: String,
    /// Terminal row offset for browse-mode viewport scrolling.
    /// Ensures the cursor step is always visible even when output overflows the screen.
    browse_scroll: usize,
}

impl Drop for TtyDisplay {
    fn drop(&mut self) {
        if self.raw_mode_active {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(std::io::stdout(), cursor::Show);
        }
        #[cfg(unix)]
        if let Some(t) = self.original_termios {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
        // Non-unix: crossterm disable_raw_mode above is sufficient.
    }
}

impl TtyDisplay {
    pub fn new(theme: Theme, verbosity: Verbosity, no_clear: bool) -> Self {
        // Disable terminal echo so that keystrokes typed while the pipeline is
        // running do not appear in the output and corrupt the step-status block.
        // We clear only ECHO/ECHOE and leave everything else (ISIG, OPOST, …)
        // untouched so that Ctrl+C still generates SIGINT and println! still
        // works normally.
        #[cfg(unix)]
        let original_termios = unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 {
                let backup = t;
                t.c_lflag &= !(libc::ECHO | libc::ECHOE);
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
                Some(backup)
            } else {
                None
            }
        };

        Self {
            theme,
            verbosity,
            no_clear,
            step_names: Vec::new(),
            statuses: Vec::new(),
            name_width: 0,
            rendered_lines: 0,
            spinner_frame: 0,
            has_running: false,
            #[cfg(unix)]
            original_termios,
            step_outputs: Vec::new(),
            expanded: Vec::new(),
            all_expanded: false,
            cursor: 0,
            browse_active: false,
            last_key: None,
            raw_mode_active: false,
            trigger_paths: None,
            run_count: 0,
            run_divider: String::new(),
            run_start: None,
            run_summary: String::new(),
            browse_scroll: 0,
        }
    }

    fn term_width() -> usize {
        crossterm::terminal::size()
            .map(|(c, _)| c as usize)
            .unwrap_or(80)
    }

    /// Returns the number of terminal rows a single printed line will occupy,
    /// accounting for line wrapping at `width` columns.
    fn visual_rows_for(text: &str, width: usize) -> u16 {
        let vlen = visible_len(text);
        if width == 0 || vlen == 0 {
            1
        } else {
            vlen.div_ceil(width) as u16
        }
    }

    fn term_height() -> u16 {
        crossterm::terminal::size().map(|(_, r)| r).unwrap_or(24)
    }

    fn raw_mode_on(&mut self) {
        if terminal::enable_raw_mode().is_ok() {
            self.raw_mode_active = true;
            // cfmakeraw() clears two flags we need:
            // - OPOST: breaks println! because \n no longer implies \r
            // - ISIG:  breaks Ctrl+C because it no longer generates SIGINT
            // Re-enable both immediately after so the display and signal
            // handling continue to work correctly.
            #[cfg(unix)]
            unsafe {
                let mut t: libc::termios = std::mem::zeroed();
                if libc::tcgetattr(libc::STDIN_FILENO, &mut t) == 0 {
                    t.c_oflag |= libc::OPOST;
                    t.c_lflag |= libc::ISIG;
                    libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
                }
            }
        }
    }

    fn raw_mode_off(&mut self) {
        if self.raw_mode_active {
            let _ = terminal::disable_raw_mode();
            self.raw_mode_active = false;
        }
    }

    /// Redraws the step block in place during a pipeline run (no highlight, no
    /// expanded output). Uses `rendered_lines` to erase the previous render.
    fn redraw(&mut self) {
        if self.verbosity == Verbosity::Quiet {
            return;
        }

        let mut stdout = std::io::stdout();
        let width = Self::term_width();

        if self.rendered_lines > 0 {
            execute!(
                stdout,
                cursor::MoveUp(self.rendered_lines),
                terminal::Clear(ClearType::FromCursorDown)
            )
            .ok();
        }

        let mut lines = 0u16;

        if !self.run_divider.is_empty() {
            println!("{}", self.divider_styled());
            lines += 1;
        }

        for (i, name) in self.step_names.iter().enumerate() {
            let (glyph, diagnostic, duration_str) = match &self.statuses[i] {
                StepStatus::Queued => (
                    format!("{}", self.theme.queued_glyph()),
                    String::new(),
                    String::new(),
                ),
                StepStatus::Running => {
                    let frame = SPINNER_FRAMES[self.spinner_frame];
                    let g = format!("{}", self.theme.yellow(frame));
                    (g, String::new(), String::new())
                }
                StepStatus::Passed(d) => (
                    format!("{}", self.theme.pass_glyph()),
                    String::new(),
                    format!("{:.1}s", d.as_secs_f64()),
                ),
                StepStatus::Failed(d, diag) => {
                    let d_str = format!("{:.1}s", d.as_secs_f64());
                    let diag_str = if diag.is_empty() {
                        String::new()
                    } else {
                        format!("{}", self.theme.dim(diag))
                    };
                    (format!("{}", self.theme.fail_glyph()), diag_str, d_str)
                }
                StepStatus::Skipped => (
                    format!("{}", self.theme.skip_glyph()),
                    format!("{}", self.theme.dim("skipped")),
                    String::new(),
                ),
            };

            // Build left portion: "▸ name    glyph   diagnostic"
            let left = if diagnostic.is_empty() {
                format!("▸ {:nw$}  {glyph}", name, nw = self.name_width)
            } else {
                format!(
                    "▸ {:nw$}  {glyph}   {diagnostic}",
                    name,
                    nw = self.name_width
                )
            };

            if duration_str.is_empty() {
                println!("{left}");
            } else {
                let right = format!("{}", self.theme.dim(&duration_str));
                let left_vis = visible_len(&left);
                let right_vis = visible_len(&right);
                let pad = width.saturating_sub(left_vis + right_vis);
                println!("{left}{:pad$}{right}", "");
            }

            lines += 1;
        }

        self.rendered_lines = lines;
        let _ = stdout.flush();
    }

    /// Redraws the step list for browse mode: includes cursor highlight and
    /// inline expanded output for toggled steps. Clipped to terminal height
    /// via a scroll viewport that always keeps the cursor step visible.
    fn browse_redraw(&mut self) {
        let mut stdout = std::io::stdout();
        let width = Self::term_width();
        let term_height = Self::term_height() as usize;

        // ── Build full content into (text, terminal_rows) pairs ──────────
        // We build everything first, then apply viewport clipping, so the
        // scroll logic can see total height before deciding what to render.
        let mut all_lines: Vec<(String, usize)> = Vec::new();
        let mut cursor_top_row = 0usize; // terminal row where the cursor step starts
        let mut cursor_row_height = 1usize;
        let mut cumulative = 0usize;

        if !self.run_divider.is_empty() {
            all_lines.push((self.divider_styled(), 1));
            cumulative += 1;
        }

        for (i, name) in self.step_names.iter().enumerate() {
            let (glyph, diagnostic, duration_str) = match &self.statuses[i] {
                StepStatus::Queued => (
                    format!("{}", self.theme.queued_glyph()),
                    String::new(),
                    String::new(),
                ),
                StepStatus::Running => {
                    let frame = SPINNER_FRAMES[self.spinner_frame];
                    (
                        format!("{}", self.theme.yellow(frame)),
                        String::new(),
                        String::new(),
                    )
                }
                StepStatus::Passed(d) => (
                    format!("{}", self.theme.pass_glyph()),
                    String::new(),
                    format!("{:.1}s", d.as_secs_f64()),
                ),
                StepStatus::Failed(d, diag) => {
                    let d_str = format!("{:.1}s", d.as_secs_f64());
                    let diag_str = if diag.is_empty() {
                        String::new()
                    } else {
                        format!("{}", self.theme.dim(diag))
                    };
                    (format!("{}", self.theme.fail_glyph()), diag_str, d_str)
                }
                StepStatus::Skipped => (
                    format!("{}", self.theme.skip_glyph()),
                    format!("{}", self.theme.dim("skipped")),
                    String::new(),
                ),
            };

            let arrow = if i == self.cursor && !self.theme.color_enabled() {
                "▶"
            } else {
                "▸"
            };
            let raw_prefix = format!("{arrow} {:nw$}", name, nw = self.name_width);
            let styled_prefix = if i == self.cursor && self.browse_active {
                format!("{}", self.theme.selected(&raw_prefix))
            } else {
                raw_prefix
            };

            let left = if diagnostic.is_empty() {
                format!("{styled_prefix}  {glyph}")
            } else {
                format!("{styled_prefix}  {glyph}   {diagnostic}")
            };

            let (step_text, step_rows) = if duration_str.is_empty() {
                let r = Self::visual_rows_for(&left, width) as usize;
                (left, r)
            } else {
                let right = format!("{}", self.theme.dim(&duration_str));
                let left_vis = visible_len(&left);
                let right_vis = visible_len(&right);
                let pad = width.saturating_sub(left_vis + right_vis);
                (format!("{left}{:pad$}{right}", ""), 1)
            };

            if i == self.cursor {
                cursor_top_row = cumulative;
                cursor_row_height = step_rows;
            }
            cumulative += step_rows;
            all_lines.push((step_text, step_rows));

            if self.expanded.get(i).copied().unwrap_or(false)
                && let Some(output) = self.step_outputs.get(i).filter(|o| !o.is_empty())
            {
                for line in output.lines() {
                    let r = Self::visual_rows_for(line, width) as usize;
                    cumulative += r;
                    all_lines.push((line.to_string(), r));
                }
            }
        }

        if self.browse_active {
            all_lines.push((String::new(), 1));
            if !self.run_summary.is_empty() {
                all_lines.push((self.run_summary.clone(), 1));
                all_lines.push((String::new(), 1));
                cumulative += 2;
            }
            let help = "  j/k ↑/↓  navigate · Enter/o  toggle output · O  expand all · q  quit";
            all_lines.push((format!("{}", self.theme.dim(help)), 1));
            cumulative += 2;
        }

        // ── Adjust scroll so cursor step stays in viewport ───────────────
        // Reserve 1 extra row so the last line never hugs the very bottom.
        let viewport = term_height.saturating_sub(1);
        let total_rows = cumulative;

        if cursor_top_row < self.browse_scroll {
            self.browse_scroll = cursor_top_row;
        } else if cursor_top_row + cursor_row_height > self.browse_scroll + viewport {
            self.browse_scroll = cursor_top_row + cursor_row_height - viewport;
        }
        self.browse_scroll = self.browse_scroll.min(total_rows.saturating_sub(viewport));

        // ── Erase previous render, then print the viewport ───────────────
        if self.rendered_lines > 0 {
            let move_up = self
                .rendered_lines
                .min((term_height as u16).saturating_sub(1));
            execute!(
                stdout,
                cursor::MoveUp(move_up),
                terminal::Clear(ClearType::FromCursorDown)
            )
            .ok();
        }

        let mut skip = self.browse_scroll;
        let mut rendered = 0usize;

        for (text, rows) in &all_lines {
            if skip > 0 {
                if skip >= *rows {
                    skip -= rows;
                    continue;
                }
                // Partial skip: skip the whole line rather than printing a
                // truncated middle of a wrapped line.
                skip = 0;
                continue;
            }
            if rendered >= viewport {
                break;
            }
            println!("{text}");
            rendered += rows;
        }

        self.rendered_lines = rendered as u16;
        let _ = stdout.flush();
    }

    fn index_of(&self, name: &str) -> usize {
        self.step_names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("unknown step `{name}`"))
    }

    /// Returns the run divider styled with the appropriate color based on step statuses.
    /// Dim while steps are still running/queued; green when all settled and passed;
    /// red when all settled and any failed.
    fn divider_styled(&self) -> String {
        if self.run_divider.is_empty() {
            return String::new();
        }
        let all_settled = self
            .statuses
            .iter()
            .all(|s| !matches!(s, StepStatus::Running | StepStatus::Queued));
        let any_failed = self
            .statuses
            .iter()
            .any(|s| matches!(s, StepStatus::Failed(..)));

        if all_settled && any_failed {
            format!("{}", self.theme.red(&self.run_divider))
        } else if all_settled {
            format!("{}", self.theme.green(&self.run_divider))
        } else {
            format!("{}", self.theme.dim(&self.run_divider))
        }
    }
}

impl Display for TtyDisplay {
    fn set_trigger(&mut self, paths: &[PathBuf]) {
        self.trigger_paths = Some(paths.to_vec());
    }

    fn banner(&mut self, root: &Path, config_path: &Path, step_count: usize) {
        if self.verbosity == Verbosity::Quiet {
            return;
        }

        let mut stdout = std::io::stdout();
        execute!(
            stdout,
            terminal::Clear(ClearType::All),
            cursor::MoveTo(0, 0)
        )
        .ok();

        let width = Self::term_width();
        let version = env!("CARGO_PKG_VERSION");
        let prefix = format!("━━━ baraddur {version} ");
        let fill = "━".repeat(width.saturating_sub(visible_len(&prefix)));
        let header = format!("{prefix}{fill}");
        println!("{}", self.theme.dim(&header));

        println!("{}  {}", self.theme.dim("watching:"), root.display());

        let config_name = config_path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        println!(
            "{}    {}  ({step_count} steps)",
            self.theme.dim("config:  "),
            config_name
        );
        println!("{}", self.theme.dim("press ^C to exit"));

        let bottom = "━".repeat(width);
        println!("{}", self.theme.dim(&bottom));

        let _ = stdout.flush();
    }

    fn run_started(&mut self, step_names: &[String]) {
        self.run_start = Some(Instant::now());
        self.run_count += 1;
        self.step_names = step_names.to_vec();
        self.statuses = vec![StepStatus::Queued; step_names.len()];
        self.name_width = step_names.iter().map(|n| n.len()).max().unwrap_or(0);
        self.rendered_lines = 0;
        self.has_running = false;
        // Reset browse state for the new run.
        self.step_outputs = vec![String::new(); step_names.len()];
        self.expanded = vec![false; step_names.len()];
        self.all_expanded = false;
        self.cursor = 0;
        self.browse_active = false;
        self.last_key = None;
        self.browse_scroll = 0;

        if self.verbosity == Verbosity::Quiet {
            return;
        }

        let mut stdout = std::io::stdout();

        if !self.no_clear {
            execute!(
                stdout,
                terminal::Clear(ClearType::All),
                cursor::MoveTo(0, 0)
            )
            .ok();
        }

        // Build and store the divider text. redraw() will print it (as its first line)
        // and recolor it live based on statuses, so no println! or cursor position needed.
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        let trigger = self.trigger_paths.take();
        let trigger_str = format_trigger_suffix(trigger.as_deref());
        let width = Self::term_width();
        let prefix = format!("━━━ #{} {ts}{trigger_str} ", self.run_count);
        let fill = "━".repeat(width.saturating_sub(visible_len(&prefix)));
        self.run_divider = format!("{prefix}{fill}");

        self.redraw();
    }

    fn step_running(&mut self, name: &str) {
        let idx = self.index_of(name);
        self.statuses[idx] = StepStatus::Running;
        self.has_running = true;
        self.redraw();
    }

    fn step_finished(&mut self, result: &StepResult) {
        let idx = self.index_of(&result.name);
        let diag = short_diagnostic(result);
        self.statuses[idx] = if result.success {
            StepStatus::Passed(result.duration)
        } else {
            StepStatus::Failed(result.duration, diag)
        };
        self.has_running = self
            .statuses
            .iter()
            .any(|s| matches!(s, StepStatus::Running));
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
        // No-op in TTY mode — the next run_started clears the screen.
        self.rendered_lines = 0;
        self.has_running = false;
    }

    fn run_finished(&mut self, results: &[StepResult]) {
        // Keep rendered_lines intact (holds the step-list row count from the last
        // redraw) so browse_redraw can MoveUp over the step list + footer together
        // and replace them cleanly in one pass.
        self.has_running = false;

        // Capture outputs and set initial browse state.
        for r in results {
            if let Some(idx) = self.step_names.iter().position(|n| n == &r.name) {
                self.step_outputs[idx] = format_truncated_output(&r.stdout, &r.stderr);
                self.expanded[idx] = !r.success;
            }
        }
        self.cursor = results
            .iter()
            .find(|r| !r.success)
            .and_then(|r| self.step_names.iter().position(|n| n == &r.name))
            .unwrap_or(0);
        self.all_expanded = results.iter().any(|r| !r.success);

        if self.verbosity == Verbosity::Quiet && results.iter().all(|r| r.success) {
            self.rendered_lines = 0;
            return;
        }

        // Footer only — output is shown inline in browse mode, not duplicated here.
        let failed = results.iter().filter(|r| !r.success).count();
        let passed = results.iter().filter(|r| r.success).count();
        let skipped = self.step_names.len().saturating_sub(results.len());
        let elapsed = self
            .run_start
            .take()
            .map(|t| t.elapsed().as_secs_f64())
            .unwrap_or_else(|| results.iter().map(|r| r.duration.as_secs_f64()).sum());

        println!();
        self.rendered_lines += 1;

        let mut parts: Vec<String> = Vec::new();
        if failed > 0 {
            let s = format!("{failed} failed");
            parts.push(format!("{}", self.theme.red(&s)));
        }
        let s = format!("{passed} passed");
        parts.push(format!("{}", self.theme.green(&s)));
        if skipped > 0 {
            let s = format!("{skipped} skipped");
            parts.push(format!("{}", self.theme.dim(&s)));
        }
        let time_str = if failed == 0 {
            format!("all passing · {elapsed:.1}s")
        } else {
            format!("{elapsed:.1}s")
        };
        parts.push(format!("{}", self.theme.dim(&time_str)));

        let summary = parts.join(" · ");
        self.run_summary = summary.clone();
        println!("{summary}");
        self.rendered_lines += 1;

        let _ = std::io::stdout().flush();
    }

    fn tick(&mut self) {
        if self.has_running {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
            self.redraw();
        }
    }

    fn enter_browse_mode(&mut self) {
        self.browse_active = true;
        self.raw_mode_on();
        let _ = execute!(std::io::stdout(), cursor::Hide);
        self.browse_redraw();
    }

    fn exit_browse_mode(&mut self) {
        // Set browse_active false before the final redraw so the cursor
        // highlight is not shown in the static post-browse state.
        self.browse_active = false;
        self.browse_redraw();
        self.raw_mode_off();
        let _ = execute!(std::io::stdout(), cursor::Show);
    }

    fn browse_redraw_if_active(&mut self) {
        if self.browse_active {
            self.browse_redraw();
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> BrowseAction {
        let n = self.step_names.len();
        if n == 0 {
            return if matches!(key.code, KeyCode::Char('q')) {
                BrowseAction::Quit
            } else {
                BrowseAction::Noop
            };
        }

        match key.code {
            KeyCode::Char('j') | KeyCode::Down => {
                self.cursor = (self.cursor + 1).min(n - 1);
                self.last_key = None;
                BrowseAction::Redraw
            }
            KeyCode::Char('k') | KeyCode::Up => {
                self.cursor = self.cursor.saturating_sub(1);
                self.last_key = None;
                BrowseAction::Redraw
            }
            KeyCode::Char('g') => {
                if self.last_key == Some(KeyCode::Char('g')) {
                    self.cursor = 0;
                    self.last_key = None;
                    BrowseAction::Redraw
                } else {
                    self.last_key = Some(KeyCode::Char('g'));
                    BrowseAction::Noop
                }
            }
            KeyCode::Char('G') => {
                self.cursor = n - 1;
                self.last_key = None;
                BrowseAction::Redraw
            }
            KeyCode::Enter | KeyCode::Char('o') => {
                self.expanded[self.cursor] = !self.expanded[self.cursor];
                self.last_key = None;
                BrowseAction::Redraw
            }
            KeyCode::Char('O') => {
                self.all_expanded = !self.all_expanded;
                for e in &mut self.expanded {
                    *e = self.all_expanded;
                }
                self.last_key = None;
                BrowseAction::Redraw
            }
            KeyCode::Char('q') => BrowseAction::Quit,
            _ => {
                // Any unrecognized key clears the pending `g` chord.
                self.last_key = None;
                BrowseAction::Noop
            }
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// Serialize stdin-redirecting tests so they don't race each other.
    static STDIN_LOCK: Mutex<()> = Mutex::new(());

    fn termios_of(fd: libc::c_int) -> libc::termios {
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            assert_eq!(
                libc::tcgetattr(fd, &mut t),
                0,
                "tcgetattr failed: {}",
                std::io::Error::last_os_error()
            );
            t
        }
    }

    /// Opens a pseudo-terminal pair. Returns `(master_fd, slave_fd)`.
    fn open_pty() -> (libc::c_int, libc::c_int) {
        let mut master: libc::c_int = -1;
        let mut slave: libc::c_int = -1;
        let ret = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        };
        assert_eq!(
            ret,
            0,
            "openpty failed: {}",
            std::io::Error::last_os_error()
        );
        (master, slave)
    }

    #[test]
    fn tty_display_disables_echo_and_restores_on_drop() {
        let _guard = STDIN_LOCK.lock().unwrap();

        let (master, slave) = open_pty();

        // Pty slave should start with echo enabled.
        let before = termios_of(slave);
        assert_ne!(
            before.c_lflag & libc::ECHO,
            0,
            "pty should start with echo on"
        );

        // Redirect stdin to the pty slave so TtyDisplay sees a real TTY fd.
        let saved_stdin = unsafe { libc::dup(libc::STDIN_FILENO) };
        assert_ne!(saved_stdin, -1);
        assert_eq!(
            unsafe { libc::dup2(slave, libc::STDIN_FILENO) },
            libc::STDIN_FILENO
        );

        {
            let _display = TtyDisplay::new(Theme::new(false), Verbosity::Normal, false);

            // Echo must be off while TtyDisplay is alive.
            let during = termios_of(slave);
            assert_eq!(
                during.c_lflag & libc::ECHO,
                0,
                "ECHO should be cleared while TtyDisplay is alive"
            );
        } // ← TtyDisplay dropped here; Drop restores the original termios.

        // Echo must be back on after drop.
        let after = termios_of(slave);
        assert_ne!(
            after.c_lflag & libc::ECHO,
            0,
            "ECHO should be restored after TtyDisplay is dropped"
        );

        // Clean up.
        unsafe {
            libc::dup2(saved_stdin, libc::STDIN_FILENO);
            libc::close(saved_stdin);
            libc::close(master);
            libc::close(slave);
        }
    }
}
