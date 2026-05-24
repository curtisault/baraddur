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

#[cfg(unix)]
mod terminal_io {
    //! Safe wrappers around the termios syscalls we need.
    //!
    //! Factored out of `TtyDisplay` so tests can target these helpers directly
    //! against a pty slave fd, without redirecting the process-wide stdin.
    use rustix::termios::{
        LocalModes, OptionalActions, OutputModes, Termios, tcgetattr, tcsetattr,
    };
    use std::os::fd::AsFd;

    /// Reads termios from `fd`, clears `ECHO`/`ECHOE`, writes back. Returns
    /// the pre-modification termios so the caller can restore later. Returns
    /// `None` if `fd` isn't a tty (tcgetattr/tcsetattr failed).
    pub fn suppress_echo<F: AsFd>(fd: F) -> Option<Termios> {
        let fd = fd.as_fd();
        let mut t = tcgetattr(fd).ok()?;
        let backup = t.clone();
        t.local_modes.remove(LocalModes::ECHO | LocalModes::ECHOE);
        tcsetattr(fd, OptionalActions::Now, &t).ok()?;
        Some(backup)
    }

    /// Re-enables `OPOST` and `ISIG` on `fd`. Called after crossterm's
    /// `enable_raw_mode` so that `println!` still emits `\r` and Ctrl+C
    /// still raises `SIGINT`. Silent on error.
    pub fn restore_signals_and_output<F: AsFd>(fd: F) {
        let fd = fd.as_fd();
        if let Ok(mut t) = tcgetattr(fd) {
            t.output_modes.insert(OutputModes::OPOST);
            t.local_modes.insert(LocalModes::ISIG);
            let _ = tcsetattr(fd, OptionalActions::Now, &t);
        }
    }

    /// Restores `fd`'s termios to a previously saved snapshot. Silent on error.
    pub fn restore<F: AsFd>(fd: F, t: &Termios) {
        let _ = tcsetattr(fd, OptionalActions::Now, t);
    }
}

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

/// Default `EditorSpawn` implementation. Resolves the editor via
/// `$VISUAL` → `$EDITOR` → `vi`, parses it through `shell_words` so users
/// can set things like `EDITOR="code --wait"`, and passes `+LINE FILE`.
/// `col` is ignored — most editors only accept the line number with the `+`
/// flag.
fn default_editor_spawn(path: &Path, line: u32, _col: Option<u32>) -> std::io::Result<()> {
    let editor = std::env::var("VISUAL")
        .or_else(|_| std::env::var("EDITOR"))
        .unwrap_or_else(|_| "vi".into());

    let parts = shell_words::split(&editor).unwrap_or_else(|_| vec![editor.clone()]);
    let (program, args) = parts.split_first().ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidInput, "empty $EDITOR / $VISUAL")
    })?;

    // Non-zero exit (user quit with :cq, etc.) isn't an error from baraddur's
    // perspective; only failure to launch is.
    let _ = std::process::Command::new(program)
        .args(args)
        .arg(format!("+{line}"))
        .arg(path)
        .status()?;
    Ok(())
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
    /// True if the run that just started was triggered by file changes;
    /// used by `run_finished` to emit "no steps match changed paths" when
    /// the trigger left zero applicable steps.
    last_run_triggered: bool,
}

impl PlainDisplay {
    pub fn new(theme: Theme, verbosity: Verbosity) -> Self {
        Self {
            theme,
            verbosity,
            trigger_paths: None,
            run_start: None,
            run_count: 0,
            last_run_triggered: false,
        }
    }
}

impl Display for PlainDisplay {
    fn set_trigger(&mut self, paths: &[PathBuf]) {
        self.trigger_paths = Some(paths.to_vec());
    }

    fn banner(
        &mut self,
        root: &Path,
        config_path: &Path,
        _step_count: usize,
        profile: Option<&str>,
    ) {
        let profile_suffix = profile
            .map(|p| format!("\n          (profile: {p})"))
            .unwrap_or_default();
        eprintln!(
            "baraddur: watching {}\n          (config: {}){profile_suffix}",
            root.display(),
            config_path.display(),
        );
    }

    fn run_started(&mut self, _step_names: &[String]) {
        self.run_start = Some(Instant::now());
        self.run_count += 1;
        let trigger = self.trigger_paths.take();
        self.last_run_triggered = trigger.is_some();
        if self.verbosity != Verbosity::Quiet {
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

        // File-change run produced zero applicable steps — surface this so
        // the user knows their save didn't trigger any work.
        if results.is_empty() && self.last_run_triggered && self.verbosity != Verbosity::Quiet {
            println!("[{ts}] no steps match changed paths");
        }

        let _ = std::io::stdout().flush();
    }

    fn hook_output(&mut self, text: &str) {
        if self.verbosity == Verbosity::Quiet {
            return;
        }
        let ts = timestamp();
        println!("[{ts}] --- on_failure ---");
        for line in text.lines() {
            println!("  {line}");
        }
        let _ = std::io::stdout().flush();
    }

    fn hook_started(&mut self) {
        if self.verbosity != Verbosity::Quiet {
            println!("[{}] on_failure hook running…", timestamp());
            let _ = std::io::stdout().flush();
        }
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
    original_termios: Option<rustix::termios::Termios>,
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
    /// Captured output from the `[on_failure]` hook for the current run, if any.
    /// Rendered as a dim block between the run summary and the help bar.
    hook_output_text: String,
    /// One-shot status message shown below the help bar (e.g., "no failures
    /// to rerun" when the user presses `f` with no failed steps). Cleared at
    /// the start of any recognized recognized key press and on `run_started`.
    transient_message: Option<String>,
    /// Snapshot of `trigger_paths` consumed by `run_started`, preserved so
    /// `run_finished` can detect "file change → 0 steps matched" and surface
    /// a `no steps match changed paths` message.
    last_trigger_paths: Option<Vec<PathBuf>>,
    /// True while an `[on_failure]` hook task is in flight. Renders a dim
    /// "running on_failure hook…" line between the summary and help bar.
    hook_running: bool,
    /// Root directory of the pipeline (captured from `banner`). Used to
    /// resolve diagnostic paths emitted relative to the runner cwd.
    root: PathBuf,
    /// Diagnostics parsed from each step's output (`run_finished`). Empty
    /// when a step had no parseable diagnostics or hasn't completed yet.
    parsed_diagnostics: Vec<Vec<crate::output::diagnostic::Diagnostic>>,
    /// Index of the "current" diagnostic *within the cursor step's list*.
    /// Per-step so that moving the cursor away and back preserves the spot.
    current_diagnostic: Vec<usize>,
    /// Editor spawn closure — injectable for tests so we can assert the
    /// command without actually exec'ing a subprocess. Default uses
    /// `$VISUAL` → `$EDITOR` → `vi` with `+LINE FILE`.
    editor_spawn: EditorSpawn,
    /// True while the `?` help modal is showing. The modal replaces the
    /// one-line help bar with a vertical key reference. Any key dismisses.
    help_modal_active: bool,
}

/// `(file, line, col?)` → spawn-and-wait outcome. Returns `Ok(())` when the
/// editor was launched (even if the user quit it non-zero); `Err` only when
/// we couldn't start it at all.
type EditorSpawn = Box<dyn Fn(&Path, u32, Option<u32>) -> std::io::Result<()> + Send + Sync>;

impl Drop for TtyDisplay {
    fn drop(&mut self) {
        if self.raw_mode_active {
            let _ = terminal::disable_raw_mode();
            let _ = execute!(std::io::stdout(), cursor::Show);
        }
        #[cfg(unix)]
        if let Some(t) = &self.original_termios {
            terminal_io::restore(std::io::stdin(), t);
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
        let original_termios = terminal_io::suppress_echo(std::io::stdin());

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
            hook_output_text: String::new(),
            transient_message: None,
            last_trigger_paths: None,
            hook_running: false,
            root: PathBuf::new(),
            parsed_diagnostics: Vec::new(),
            current_diagnostic: Vec::new(),
            editor_spawn: Box::new(default_editor_spawn),
            help_modal_active: false,
        }
    }

    /// Replaces the default editor spawner. Tests use this to record the
    /// `(path, line, col)` triple without actually launching an editor.
    pub fn set_editor_spawn(&mut self, f: EditorSpawn) {
        self.editor_spawn = f;
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
            terminal_io::restore_signals_and_output(std::io::stdin());
        }
    }

    fn raw_mode_off(&mut self) {
        if self.raw_mode_active {
            let _ = terminal::disable_raw_mode();
            self.raw_mode_active = false;
        }
    }

    /// Releases the terminal (raw mode off, cursor visible) for the duration
    /// of `f`, then restores browse-mode state and redraws. Used by `e` to
    /// hand the terminal off to `$EDITOR` cleanly.
    fn with_terminal_released<F, R>(&mut self, f: F) -> R
    where
        F: FnOnce(&mut Self) -> R,
    {
        let was_browse = self.browse_active;
        if was_browse {
            self.raw_mode_off();
            let _ = execute!(std::io::stdout(), cursor::Show);
        }
        let result = f(self);
        if was_browse {
            self.raw_mode_on();
            let _ = execute!(std::io::stdout(), cursor::Hide);
            self.browse_redraw();
        }
        result
    }

    /// Builds the rendered, styled lines of the `?` help modal. Two-column
    /// vertical key reference; section headers in cyan, body dim. Returned
    /// as `Vec<String>` so the caller can size each line individually.
    fn help_modal_lines(&self) -> Vec<String> {
        // Each row is `(left, right)`; either may be empty for spacers and
        // section headers that only occupy one column.
        let rows: &[(&str, &str)] = &[
            ("Navigation", "Diagnostics"),
            ("  j / ↓   down", "  n   next"),
            ("  k / ↑   up", "  p   prev"),
            ("  g g     top", "  e   open in editor"),
            ("  G       bottom", ""),
            ("", ""),
            ("Output", "Rerun"),
            ("  Enter / o  toggle", "  r   pipeline"),
            ("  O          expand", "  f   failed steps"),
            ("", "  c   step under cursor"),
        ];

        let col_width = 26usize;
        let mut out = Vec::with_capacity(rows.len() + 4);

        // Header.
        out.push(format!("  {}", self.theme.cyan("Help")));
        out.push(String::new());

        for (left, right) in rows {
            let is_header = !left.is_empty() && !left.starts_with(' ');
            let is_right_header = !right.is_empty() && !right.starts_with(' ');

            let left_styled = if left.is_empty() {
                String::new()
            } else if is_header {
                format!("{}", self.theme.cyan(left))
            } else {
                format!("{}", self.theme.dim(left))
            };
            let right_styled = if right.is_empty() {
                String::new()
            } else if is_right_header {
                format!("{}", self.theme.cyan(right))
            } else {
                format!("{}", self.theme.dim(right))
            };

            // Pad left to `col_width` visible columns so right column lines up.
            let left_pad = col_width.saturating_sub(visible_len(&left_styled));
            out.push(format!("  {left_styled}{:left_pad$}{right_styled}", ""));
        }

        out.push(String::new());
        out.push(format!("  {}", self.theme.dim("press any key to dismiss")));
        out
    }

    /// Moves the diagnostic cursor for the current step by `delta`, wrapping
    /// around the list. Returns `false` when the step has no diagnostics.
    fn advance_diagnostic(&mut self, delta: i32) -> bool {
        let len = match self.parsed_diagnostics.get(self.cursor) {
            Some(d) if !d.is_empty() => d.len() as i32,
            _ => return false,
        };
        let cur = self
            .current_diagnostic
            .get(self.cursor)
            .copied()
            .unwrap_or(0) as i32;
        let next = (cur + delta).rem_euclid(len) as usize;
        self.current_diagnostic[self.cursor] = next;
        true
    }

    /// Opens the current diagnostic's `(file, line, col)` in the configured
    /// editor. No-op when the step has no diagnostics; reports spawn errors
    /// via the transient-message slot.
    fn open_current_diagnostic(&mut self) {
        let Some(diags) = self.parsed_diagnostics.get(self.cursor) else {
            return;
        };
        if diags.is_empty() {
            self.transient_message = Some("no diagnostics on this step".into());
            return;
        }
        let idx = self
            .current_diagnostic
            .get(self.cursor)
            .copied()
            .unwrap_or(0);
        let diag = diags[idx].clone();
        let abs = if diag.path.is_absolute() {
            diag.path.clone()
        } else {
            self.root.join(&diag.path)
        };
        let result = self.with_terminal_released(|s| (s.editor_spawn)(&abs, diag.line, diag.col));
        if let Err(e) = result {
            self.transient_message = Some(format!("editor: {e}"));
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
            let divider = self.divider_styled();
            println!("{divider}");
            lines += Self::visual_rows_for(&divider, width);
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

            let line = if duration_str.is_empty() {
                left
            } else {
                let right = format!("{}", self.theme.dim(&duration_str));
                let left_vis = visible_len(&left);
                let right_vis = visible_len(&right);
                let pad = width.saturating_sub(left_vis + right_vis);
                format!("{left}{:pad$}{right}", "")
            };

            println!("{line}");
            lines += Self::visual_rows_for(&line, width);
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
                let line = format!("{left}{:pad$}{right}", "");
                let r = Self::visual_rows_for(&line, width) as usize;
                (line, r)
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
                let diags: &[crate::output::diagnostic::Diagnostic] = self
                    .parsed_diagnostics
                    .get(i)
                    .map(Vec::as_slice)
                    .unwrap_or(&[]);
                let current = self.current_diagnostic.get(i).copied().unwrap_or(0);
                let is_cursor_step = i == self.cursor;
                for line in output.lines() {
                    // Output lines have a two-space prefix from
                    // `format_truncated_output`. Strip before pattern matching.
                    let raw = line.strip_prefix("  ").unwrap_or(line);
                    let diag_idx = crate::output::diagnostic::extract_line(raw)
                        .and_then(|d| diags.iter().position(|x| x == &d));
                    let (display_line, r) = if let Some(idx) = diag_idx {
                        let is_current = is_cursor_step && idx == current;
                        let styled = if is_current {
                            // Color: cyan marker + cyan underlined body.
                            // NO_COLOR: distinct `▶` glyph stands in for the
                            // underline, matching the cursor-row fallback at
                            // the step-list level.
                            if self.theme.color_enabled() {
                                format!(
                                    "{} {}",
                                    self.theme.cyan("▸"),
                                    self.theme.cyan_underline(raw)
                                )
                            } else {
                                format!("▶ {raw}")
                            }
                        } else {
                            format!("{} {raw}", self.theme.dim("▸"))
                        };
                        let r = Self::visual_rows_for(&styled, width) as usize;
                        (styled, r)
                    } else {
                        let r = Self::visual_rows_for(line, width) as usize;
                        (line.to_string(), r)
                    };
                    cumulative += r;
                    all_lines.push((display_line, r));
                }
            }
        }

        if self.browse_active {
            // Spacer before the footer.
            all_lines.push((String::new(), 1));
            cumulative += 1;
            if !self.run_summary.is_empty() {
                let r = Self::visual_rows_for(&self.run_summary, width) as usize;
                all_lines.push((self.run_summary.clone(), r));
                cumulative += r;
                all_lines.push((String::new(), 1));
                cumulative += 1;
            }
            // Hook slot: "running…" while in flight; output once settled.
            // Only one of the two is shown at a time.
            if self.hook_running {
                let line = format!("  {}", self.theme.dim("running on_failure hook…"));
                let r = Self::visual_rows_for(&line, width) as usize;
                all_lines.push((line, r));
                cumulative += r;
                all_lines.push((String::new(), 1));
                cumulative += 1;
            } else if !self.hook_output_text.is_empty() {
                for line in self.hook_output_text.lines() {
                    let styled = format!("  {}", self.theme.dim(line));
                    let r = Self::visual_rows_for(&styled, width) as usize;
                    all_lines.push((styled, r));
                    cumulative += r;
                }
                all_lines.push((String::new(), 1));
                cumulative += 1;
            }
            if self.help_modal_active {
                for line in self.help_modal_lines() {
                    let r = Self::visual_rows_for(&line, width) as usize;
                    all_lines.push((line, r));
                    cumulative += r;
                }
            } else {
                // Option B: grouped, symbol-led, fits ~80 cols.
                let help = "  ↕ j/k   ⏎ toggle   ▸ n/p/e   ↺ r/f/c   ? help · q quit";
                let help_styled = format!("{}", self.theme.dim(help));
                let r = Self::visual_rows_for(&help_styled, width) as usize;
                all_lines.push((help_styled, r));
                cumulative += r;
            }

            if let Some(msg) = &self.transient_message {
                let line = format!("  {}", self.theme.yellow(msg));
                let r = Self::visual_rows_for(&line, width) as usize;
                all_lines.push((line, r));
                cumulative += r;
            }
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

    fn banner(
        &mut self,
        root: &Path,
        config_path: &Path,
        step_count: usize,
        profile: Option<&str>,
    ) {
        // Capture for editor-jump path resolution before the early-quiet exit
        // so diagnostic paths still resolve even when banner output is muted.
        self.root = root.to_path_buf();

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
        let profile_suffix = profile
            .map(|p| format!("  (profile: {p})"))
            .unwrap_or_default();
        println!(
            "{}    {}  ({step_count} steps){profile_suffix}",
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
        self.hook_output_text.clear();
        self.transient_message = None;
        self.hook_running = false;
        self.parsed_diagnostics = vec![Vec::new(); step_names.len()];
        self.current_diagnostic = vec![0; step_names.len()];
        self.help_modal_active = false;

        // Stash trigger BEFORE moving the value into the divider, so
        // `run_finished` can detect "file change → zero steps matched".
        let trigger = self.trigger_paths.take();
        self.last_trigger_paths = trigger.clone();

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
                // Parse diagnostics from the full (untruncated) output so
                // entries hidden by elision are still navigable via n/p.
                let combined = if r.stderr.is_empty() {
                    r.stdout.clone()
                } else if r.stdout.is_empty() {
                    r.stderr.clone()
                } else {
                    format!("{}\n{}", r.stdout, r.stderr)
                };
                self.parsed_diagnostics[idx] = crate::output::diagnostic::parse(&combined);
                self.current_diagnostic[idx] = 0;
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
        let width = Self::term_width();
        self.rendered_lines += Self::visual_rows_for(&summary, width);

        // File-change run with zero applicable steps after path filtering —
        // surface this so a save doesn't look like a silent no-op. Shown in
        // the transient-message slot below the help bar.
        if results.is_empty() && self.last_trigger_paths.is_some() {
            self.transient_message = Some(match self.last_trigger_paths.as_deref() {
                Some([p]) => format!("no steps match changed path: {}", p.display()),
                Some(ps) if ps.len() > 1 => {
                    format!("no steps match {} changed paths", ps.len())
                }
                _ => "no steps match changed paths".into(),
            });
        }

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

    fn hook_output(&mut self, text: &str) {
        self.hook_output_text = text.to_string();
        // Receiving output implies the hook settled; clear the running flag
        // even if `hook_finished` hasn't been called yet.
        self.hook_running = false;
        if self.browse_active {
            self.browse_redraw();
        }
    }

    fn hook_started(&mut self) {
        self.hook_running = true;
        if self.browse_active {
            self.browse_redraw();
        }
    }

    fn hook_finished(&mut self) {
        self.hook_running = false;
        if self.browse_active {
            self.browse_redraw();
        }
    }

    fn handle_key(&mut self, key: KeyEvent) -> BrowseAction {
        // Modal capture: while the `?` help is up, any key dismisses it and
        // does nothing else — the key isn't forwarded to normal handling so
        // users don't accidentally rerun or quit while reading the keys.
        if self.help_modal_active {
            self.help_modal_active = false;
            self.last_key = None;
            return BrowseAction::Redraw;
        }

        let n = self.step_names.len();
        if n == 0 {
            return if matches!(key.code, KeyCode::Char('q')) {
                BrowseAction::Quit
            } else {
                BrowseAction::Noop
            };
        }

        // Any recognized key dismisses a prior transient message. The 'f'-no-
        // failures arm re-sets it below. If we cleared a visible message but
        // the resulting action is Noop, promote to Redraw so the message
        // actually disappears from the screen.
        let had_message = self.transient_message.take().is_some();

        let action = match key.code {
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
            KeyCode::Char('r') => {
                self.last_key = None;
                BrowseAction::Rerun
            }
            KeyCode::Char('f') => {
                self.last_key = None;
                // Only meaningful if there were failures to retry. Otherwise
                // surface a one-shot message so the user knows the key was
                // received but had nothing to act on.
                if self
                    .statuses
                    .iter()
                    .any(|s| matches!(s, StepStatus::Failed(..)))
                {
                    BrowseAction::RerunFailed
                } else {
                    self.transient_message = Some("no failures to re-run".into());
                    BrowseAction::Redraw
                }
            }
            KeyCode::Char('c') => {
                self.last_key = None;
                // Rerun the step under the cursor in isolation. Always
                // emitted — a skipped step runs alone (matches "rerun
                // exactly what you pointed at"), and a passing step reruns
                // freshly which is useful after edits.
                BrowseAction::RerunCursor(self.step_names[self.cursor].clone())
            }
            KeyCode::Char('n') => {
                self.last_key = None;
                if self.advance_diagnostic(1) {
                    BrowseAction::Redraw
                } else {
                    self.transient_message = Some("no diagnostics on this step".into());
                    BrowseAction::Redraw
                }
            }
            KeyCode::Char('p') => {
                self.last_key = None;
                if self.advance_diagnostic(-1) {
                    BrowseAction::Redraw
                } else {
                    self.transient_message = Some("no diagnostics on this step".into());
                    BrowseAction::Redraw
                }
            }
            KeyCode::Char('e') => {
                self.last_key = None;
                self.open_current_diagnostic();
                BrowseAction::Redraw
            }
            KeyCode::Char('?') => {
                self.last_key = None;
                self.help_modal_active = true;
                BrowseAction::Redraw
            }
            KeyCode::Char('q') => BrowseAction::Quit,
            _ => {
                // Any unrecognized key clears the pending `g` chord.
                self.last_key = None;
                BrowseAction::Noop
            }
        };

        // Hide a just-cleared message even when no other state changed.
        if matches!(action, BrowseAction::Noop) && had_message {
            BrowseAction::Redraw
        } else {
            action
        }
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[cfg(unix)]
mod tests {
    use super::*;
    use rustix::termios::{LocalModes, OutputModes, tcgetattr};
    use rustix_openpty::openpty;
    use std::os::fd::AsFd;

    /// suppress_echo must clear ECHO/ECHOE on the target fd and return a
    /// backup whose Termios can be used to restore the original state.
    #[test]
    fn suppress_echo_clears_echo_and_restore_brings_it_back() {
        let pty = openpty(None, None).expect("openpty failed");
        let user = pty.user.as_fd();

        // Pty user side starts with echo enabled.
        let before = tcgetattr(user).unwrap();
        assert!(
            before.local_modes.contains(LocalModes::ECHO),
            "pty should start with echo on"
        );

        let backup = terminal_io::suppress_echo(user).expect("suppress_echo failed");

        let during = tcgetattr(user).unwrap();
        assert!(
            !during.local_modes.contains(LocalModes::ECHO),
            "ECHO should be cleared after suppress_echo"
        );
        assert!(
            !during.local_modes.contains(LocalModes::ECHOE),
            "ECHOE should also be cleared"
        );

        terminal_io::restore(user, &backup);

        let after = tcgetattr(user).unwrap();
        assert!(
            after.local_modes.contains(LocalModes::ECHO),
            "ECHO should be restored after restore()"
        );
    }

    /// Pressing `c` in browse mode returns `RerunCursor` carrying the name
    /// of the step under the cursor. Verifies the third rerun key alongside
    /// `r` (full) and `f` (failed).
    #[test]
    fn handle_key_c_returns_rerun_cursor_for_step_under_cursor() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut d = TtyDisplay::new(Theme::new(false), Verbosity::Quiet, true);
        // Populate step list via the normal lifecycle so internal vectors are
        // sized correctly. Cursor lands at index 0 by default.
        d.run_started(&["alpha".to_string(), "beta".to_string(), "gamma".to_string()]);
        d.cursor = 1;

        let action = d.handle_key(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::NONE));
        match action {
            BrowseAction::RerunCursor(name) => assert_eq!(name, "beta"),
            other => panic!("expected RerunCursor(\"beta\"), got {other:?}"),
        }
    }

    /// `n` / `p` wrap around the diagnostic list for the cursor step, and
    /// `e` hands the *current* `(path, line, col)` to the injected editor
    /// spawn — proving the navigation index correctly drives editor jump.
    #[test]
    fn diagnostic_navigation_and_editor_spawn() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use std::sync::{Arc, Mutex};

        type EditorCall = (std::path::PathBuf, u32, Option<u32>);

        let mut d = TtyDisplay::new(Theme::new(false), Verbosity::Quiet, true);
        d.banner(
            std::path::Path::new("/tmp/repo"),
            std::path::Path::new("/tmp/repo/.baraddur.toml"),
            1,
            None,
        );
        d.run_started(&["step".to_string()]);

        d.parsed_diagnostics[0] = vec![
            crate::output::diagnostic::Diagnostic {
                path: std::path::PathBuf::from("a.rs"),
                line: 1,
                col: Some(1),
            },
            crate::output::diagnostic::Diagnostic {
                path: std::path::PathBuf::from("b.rs"),
                line: 2,
                col: Some(2),
            },
            crate::output::diagnostic::Diagnostic {
                path: std::path::PathBuf::from("c.rs"),
                line: 3,
                col: None,
            },
        ];

        // Recorder captures (path, line, col) on each invocation.
        let calls: Arc<Mutex<Vec<EditorCall>>> = Arc::new(Mutex::new(Vec::new()));
        let recorder = calls.clone();
        d.set_editor_spawn(Box::new(move |path, line, col| {
            recorder
                .lock()
                .unwrap()
                .push((path.to_path_buf(), line, col));
            Ok(())
        }));

        // Initial diag is index 0 → a.rs.
        d.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        // n n → 1 → 2; one more n wraps back to 0.
        d.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Char('n'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));
        // p wraps 2 → 1.
        d.handle_key(KeyEvent::new(KeyCode::Char('p'), KeyModifiers::NONE));
        d.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

        let recorded = calls.lock().unwrap().clone();
        assert_eq!(recorded.len(), 4);
        assert_eq!(
            recorded[0],
            (std::path::PathBuf::from("/tmp/repo/a.rs"), 1, Some(1))
        );
        assert_eq!(
            recorded[1],
            (std::path::PathBuf::from("/tmp/repo/b.rs"), 2, Some(2))
        );
        assert_eq!(
            recorded[2],
            (std::path::PathBuf::from("/tmp/repo/c.rs"), 3, None)
        );
        assert_eq!(
            recorded[3],
            (std::path::PathBuf::from("/tmp/repo/b.rs"), 2, Some(2))
        );
    }

    /// `?` opens the help modal; any subsequent key dismisses it without
    /// forwarding the keypress to the normal handlers (so the user can read
    /// the keys without accidentally rerunning or quitting).
    #[test]
    fn help_modal_opens_on_question_mark_and_dismisses_on_any_key() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

        let mut d = TtyDisplay::new(Theme::new(false), Verbosity::Quiet, true);
        d.run_started(&["step".to_string()]);
        assert!(!d.help_modal_active);

        d.handle_key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        assert!(d.help_modal_active, "? should open the modal");

        // Pressing `q` while the modal is up should dismiss it instead of
        // quitting — that's the modal-capture guarantee.
        let action = d.handle_key(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::NONE));
        assert!(matches!(action, BrowseAction::Redraw));
        assert!(!d.help_modal_active, "any key should dismiss the modal");
    }

    /// `e` on a step with no diagnostics surfaces a transient message and
    /// never invokes the spawn closure.
    #[test]
    fn editor_key_with_no_diagnostics_is_no_op() {
        use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
        use std::sync::{Arc, Mutex};

        let mut d = TtyDisplay::new(Theme::new(false), Verbosity::Quiet, true);
        d.run_started(&["step".to_string()]);

        let invoked = Arc::new(Mutex::new(false));
        let flag = invoked.clone();
        d.set_editor_spawn(Box::new(move |_, _, _| {
            *flag.lock().unwrap() = true;
            Ok(())
        }));

        d.handle_key(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::NONE));

        assert!(!*invoked.lock().unwrap(), "editor should not be invoked");
        assert_eq!(
            d.transient_message.as_deref(),
            Some("no diagnostics on this step")
        );
    }

    /// restore_signals_and_output must turn OPOST and ISIG back on, even if
    /// they had been cleared (as crossterm's enable_raw_mode would).
    #[test]
    fn restore_signals_and_output_reenables_opost_and_isig() {
        use rustix::termios::{OptionalActions, tcsetattr};

        let pty = openpty(None, None).expect("openpty failed");
        let user = pty.user.as_fd();

        // Clear OPOST and ISIG to simulate raw-mode setup.
        let mut t = tcgetattr(user).unwrap();
        t.output_modes.remove(OutputModes::OPOST);
        t.local_modes.remove(LocalModes::ISIG);
        tcsetattr(user, OptionalActions::Now, &t).unwrap();

        terminal_io::restore_signals_and_output(user);

        let after = tcgetattr(user).unwrap();
        assert!(
            after.output_modes.contains(OutputModes::OPOST),
            "OPOST should be re-enabled"
        );
        assert!(
            after.local_modes.contains(LocalModes::ISIG),
            "ISIG should be re-enabled"
        );
    }
}
