pub mod display;
pub use display::PlainDisplay;

use crate::pipeline::StepResult;

/// Sink for pipeline events.
///
/// Phase 1 has one implementation (`PlainDisplay`).
/// Phase 4 will add a `TtyDisplay` with colors, spinners, and in-place redraw.
pub trait Display: Send {
    fn run_started(&mut self);
    fn step_started(&mut self, name: &str);
    fn step_finished(&mut self, result: &StepResult);
    fn run_finished(&mut self, results: &[StepResult]);
}
