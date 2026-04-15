use anyhow::Result;
use std::path::Path;

use super::step::{self, StepResult};
use crate::config::Config;
use crate::output::Display;

/// Runs all steps sequentially, stopping at the first failure.
///
/// Returns all results collected up to and including the first failure.
/// Parallel grouping is a Phase 3 feature; all steps run in declared order here.
pub async fn run_sequential(
    config: &Config,
    cwd: &Path,
    display: &mut dyn Display,
) -> Result<Vec<StepResult>> {
    display.run_started();

    let mut results = Vec::with_capacity(config.steps.len());

    for step_cfg in &config.steps {
        display.step_started(&step_cfg.name);
        let result = step::run(step_cfg, cwd).await?;
        let success = result.success;
        display.step_finished(&result);
        results.push(result);
        if !success {
            break;
        }
    }

    display.run_finished(&results);
    Ok(results)
}
