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

    /// Steps were skipped because an earlier stage failed.
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
