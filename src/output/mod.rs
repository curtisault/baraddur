pub mod display;
pub mod style;
pub use display::{PlainDisplay, TtyDisplay};
pub use style::Theme;

use crate::pipeline::StepResult;
use std::path::Path;

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

    /// Show the startup banner. Called once before the first pipeline run.
    fn banner(&mut self, _root: &Path, _config_path: &Path, _step_count: usize) {}

    /// Advance the spinner animation by one frame. Only redraws if there are
    /// steps in the Running state. Default is a no-op (PlainDisplay).
    fn tick(&mut self) {}
}
