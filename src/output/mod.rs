pub mod diagnostic;
pub mod display;
pub mod style;
pub use display::{PlainDisplay, TtyDisplay};
pub use style::Theme;

use crate::pipeline::StepResult;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Verbosity {
    Quiet,
    Normal,
    Verbose,
    Debug,
}

pub struct DisplayConfig {
    pub is_tty: bool,
    pub no_clear: bool,
    pub verbosity: Verbosity,
}

/// Returned by `Display::handle_key` to tell the caller what to do next.
#[derive(Debug)]
pub enum BrowseAction {
    /// Key was consumed but no visual change is needed.
    Noop,
    /// State changed; caller should call `browse_redraw_if_active`.
    Redraw,
    /// User pressed `q`; caller should exit browse mode and shut down the app.
    Quit,
    /// User pressed `r`; caller should exit browse mode and rerun the full
    /// pipeline (equivalent to the initial run — no trigger, no step filter).
    Rerun,
    /// User pressed `f`; caller should exit browse mode and rerun only the
    /// steps that failed in the previous run. Display returns this only if
    /// at least one step in the current view was marked Failed.
    RerunFailed,
    /// User pressed `c`; caller should exit browse mode and rerun only the
    /// single step under the cursor. The name is carried so the main loop
    /// can drop it straight into `rerun_filter`.
    RerunCursor(String),
}

/// Sink for pipeline lifecycle events.
///
/// Implementations:
/// - `PlainDisplay` — append-only line output for non-TTY / piped contexts.
/// - `TtyDisplay` — full-block redraw for interactive terminals.
pub trait Display: Send {
    /// A new pipeline run is starting. `step_names` lists all steps in
    /// declared order, used for layout (name-column padding).
    fn run_started(&mut self, step_names: &[String]);

    /// A step has begun executing.
    fn step_running(&mut self, name: &str);

    /// A step has completed (pass or fail).
    fn step_finished(&mut self, result: &StepResult);

    /// Steps were skipped because an earlier stage failed.
    fn steps_skipped(&mut self, names: &[String]);

    /// The run was cancelled mid-flight (file change triggered restart).
    fn run_cancelled(&mut self);

    /// The run completed normally.
    fn run_finished(&mut self, results: &[StepResult]);

    /// Store the file(s) that triggered this run. Called just before the next
    /// `run_started` when a file-change restart occurs. Not called for the
    /// initial run. Implementations should consume and clear on `run_started`.
    fn set_trigger(&mut self, _paths: &[PathBuf]) {}

    /// Show the startup banner. Called once before the first pipeline run.
    /// `profile` is the active profile name (from `--profile <name>`) when
    /// one narrowed the step list; `None` means every configured step is in
    /// play.
    fn banner(
        &mut self,
        _root: &Path,
        _config_path: &Path,
        _step_count: usize,
        _profile: Option<&str>,
    ) {
    }

    /// Advance the spinner animation by one frame. Only redraws if there are
    /// steps in the Running state. Default is a no-op (PlainDisplay).
    fn tick(&mut self) {}

    /// Enter post-run interactive browse mode. Called by the idle loop after
    /// `run_finished`. Default is a no-op (PlainDisplay).
    fn enter_browse_mode(&mut self) {}

    /// Exit browse mode. Called when a file change arrives, Ctrl+C is pressed,
    /// or the user presses `q`. Default is a no-op (PlainDisplay).
    fn exit_browse_mode(&mut self) {}

    /// Handle a keypress during browse mode. Returns a `BrowseAction` telling
    /// the caller whether to redraw or exit browse mode. Default is a no-op.
    fn handle_key(&mut self, _key: crossterm::event::KeyEvent) -> BrowseAction {
        BrowseAction::Noop
    }

    /// Redraw the browse step list if browse mode is currently active.
    /// Default is a no-op (PlainDisplay).
    fn browse_redraw_if_active(&mut self) {}

    /// Show output from the configured `[on_failure]` hook. Called from the
    /// main loop after the hook subprocess completes successfully.
    /// Default is a no-op.
    fn hook_output(&mut self, _text: &str) {}

    /// A `[on_failure]` hook task has been spawned. Implementations should
    /// show a "running" indicator to make the async work visible.
    /// Default is a no-op.
    fn hook_started(&mut self) {}

    /// The hook task has settled (completed normally, timed out, exited
    /// non-zero, or was aborted). Implementations should clear any "running"
    /// indicator. Always called when the task exits, regardless of outcome.
    /// Default is a no-op.
    fn hook_finished(&mut self) {}
}
