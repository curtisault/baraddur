use std::io::Write as _;

use super::Display;
use crate::pipeline::StepResult;

/// Phase 1 plain display — line-by-line, no colors, no cursor movement.
pub struct PlainDisplay;

impl Display for PlainDisplay {
    fn run_started(&mut self) {
        println!("--- run started ---");
    }

    fn step_started(&mut self, name: &str) {
        println!("▸ {name} ...");
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

    fn run_finished(&mut self, results: &[StepResult]) {
        let failed = results.iter().filter(|r| !r.success).count();
        let passed = results.iter().filter(|r| r.success).count();
        let total: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();
        println!("--- run complete: {failed} failed, {passed} passed, {total:.1}s ---");
        let _ = std::io::stdout().flush();
    }
}
