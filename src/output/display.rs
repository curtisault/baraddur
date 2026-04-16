use std::io::Write as _;
use std::path::Path;
use std::time::Duration;

use crossterm::{
    cursor, execute,
    terminal::{self, ClearType},
};

use super::{Display, Verbosity};
use super::style::{Theme, visible_len};
use crate::pipeline::StepResult;

// ── Shared helpers ───────────────────────────────────────────────────────────

fn timestamp() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

/// Prints stdout+stderr with head+tail truncation if the output is long.
/// Truncated lines reference the log file for the full output.
fn print_truncated_output(stdout: &str, stderr: &str) {
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

    if lines.len() <= MAX_DISPLAY_LINES {
        for line in &lines {
            println!("  {line}");
        }
    } else {
        for line in &lines[..CONTEXT_LINES] {
            println!("  {line}");
        }
        let elided = lines.len() - (CONTEXT_LINES * 2);
        println!("  ... [{elided} lines elided — see .baraddur/last-run.log] ...");
        for line in &lines[lines.len() - CONTEXT_LINES..] {
            println!("  {line}");
        }
    }
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
            let non_empty: Vec<&str> = combined
                .lines()
                .filter(|l| !l.trim().is_empty())
                .collect();
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

// ── Non-TTY display (append-only) ───────────────────────────────────────────

/// Append-only line output for non-TTY contexts (piped, CI, `--no-tty`).
/// No cursor movement, no screen clearing.
pub struct PlainDisplay {
    theme: Theme,
    verbosity: Verbosity,
}

impl PlainDisplay {
    pub fn new(theme: Theme, verbosity: Verbosity) -> Self {
        Self { theme, verbosity }
    }
}

impl Display for PlainDisplay {
    fn banner(&mut self, root: &Path, config_path: &Path, _step_count: usize) {
        eprintln!(
            "baraddur: watching {}\n          (config: {})",
            root.display(),
            config_path.display(),
        );
    }

    fn run_started(&mut self, _step_names: &[String]) {
        if self.verbosity != Verbosity::Quiet {
            println!("[{}] run started", timestamp());
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
            print_truncated_output(&r.stdout, &r.stderr);
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
        let total: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();

        if self.verbosity != Verbosity::Quiet || failed > 0 {
            println!("[{ts}] run complete: {failed} failed, {passed} passed, {total:.1}s");
        }

        let _ = std::io::stdout().flush();
    }
}

// ── TTY display (full-block redraw) ─────────────────────────────────────────

const SPINNER_FRAMES: &[&str] = &[
    "⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏",
];

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
    /// How many lines the last `redraw()` printed.
    rendered_lines: u16,
    spinner_frame: usize,
    has_running: bool,
    /// Original termios saved on construction so we can restore on drop.
    /// Suppressing echo prevents typed characters from corrupting the redrawn
    /// step-status block while a pipeline is running.
    #[cfg(unix)]
    original_termios: Option<libc::termios>,
}

impl Drop for TtyDisplay {
    fn drop(&mut self) {
        #[cfg(unix)]
        if let Some(t) = self.original_termios {
            unsafe {
                libc::tcsetattr(libc::STDIN_FILENO, libc::TCSANOW, &t);
            }
        }
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
        }
    }

    fn term_width() -> usize {
        crossterm::terminal::size()
            .map(|(c, _)| c as usize)
            .unwrap_or(80)
    }

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
                format!("▸ {:nw$}  {glyph}   {diagnostic}", name, nw = self.name_width)
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

    fn index_of(&self, name: &str) -> usize {
        self.step_names
            .iter()
            .position(|n| n == name)
            .unwrap_or_else(|| panic!("unknown step `{name}`"))
    }
}

impl Display for TtyDisplay {
    fn banner(&mut self, root: &Path, config_path: &Path, step_count: usize) {
        if self.verbosity == Verbosity::Quiet {
            return;
        }

        let mut stdout = std::io::stdout();
        execute!(stdout, terminal::Clear(ClearType::All), cursor::MoveTo(0, 0)).ok();

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
        self.step_names = step_names.to_vec();
        self.statuses = vec![StepStatus::Queued; step_names.len()];
        self.name_width = step_names.iter().map(|n| n.len()).max().unwrap_or(0);
        self.rendered_lines = 0;
        self.has_running = false;

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

        // Timestamp divider
        let ts = chrono::Local::now().format("%H:%M:%S").to_string();
        let width = Self::term_width();
        let prefix = format!("━━━ {ts} ");
        let fill = "━".repeat(width.saturating_sub(visible_len(&prefix)));
        let divider = format!("{prefix}{fill}");
        println!("{}", self.theme.dim(&divider));

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
        self.rendered_lines = 0;
        self.has_running = false;

        if self.verbosity == Verbosity::Quiet && results.iter().all(|r| r.success) {
            return;
        }

        // Print failure output blocks below the step list.
        let failures: Vec<_> = results.iter().filter(|r| !r.success).collect();
        if !failures.is_empty() {
            println!();
            for r in &failures {
                let header = format!("── {} output ", r.name);
                let width = Self::term_width();
                let fill = "─".repeat(width.saturating_sub(visible_len(&header)));
                let header_line = format!("{header}{fill}");
                println!("{}", self.theme.cyan(&header_line));
                print_truncated_output(&r.stdout, &r.stderr);
            }
        }

        // In verbose mode, also show passing step output.
        if self.verbosity >= Verbosity::Verbose {
            for r in results.iter().filter(|r| r.success) {
                if !r.stdout.is_empty() {
                    println!();
                    let header = format!("── {} output ", r.name);
                    let width = Self::term_width();
                    let fill = "─".repeat(width.saturating_sub(visible_len(&header)));
                    let header_line = format!("{header}{fill}");
                    println!("{}", self.theme.cyan(&header_line));
                    for line in r.stdout.lines() {
                        println!("  {line}");
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
            format!("all passing · {total:.1}s")
        } else {
            format!("{total:.1}s")
        };
        parts.push(format!("{}", self.theme.dim(&time_str)));

        println!("{}", parts.join(" · "));
        let _ = std::io::stdout().flush();
    }

    fn tick(&mut self) {
        if self.has_running {
            self.spinner_frame = (self.spinner_frame + 1) % SPINNER_FRAMES.len();
            self.redraw();
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
        assert_ne!(before.c_lflag & libc::ECHO, 0, "pty should start with echo on");

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
