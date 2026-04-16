use anyhow::{Context, Result};
use std::path::Path;
use std::time::Duration;
use tokio::task::JoinSet;

use super::group_into_stages;
use super::step::{self, StepResult};
use crate::config::Config;
use crate::output::Display;

/// Runs the full pipeline: groups steps into stages, executes each stage
/// (sequentially or in parallel), stops after the first failing stage.
///
/// `spinner_interval` — when `Some`, ticks the display spinner at that rate
/// while steps are executing. Pass `None` in non-TTY mode or tests.
///
/// Returns all `StepResult`s for steps that actually ran (not skipped).
pub async fn run_pipeline(
    config: &Config,
    cwd: &Path,
    display: &mut dyn Display,
    spinner_interval: Option<Duration>,
) -> Result<Vec<StepResult>> {
    let step_names: Vec<String> = config.steps.iter().map(|s| s.name.clone()).collect();
    display.run_started(&step_names);

    let stages = group_into_stages(&config.steps);
    let mut all_results: Vec<StepResult> = Vec::with_capacity(config.steps.len());
    let mut stage_failed = false;

    for stage in &stages {
        if stage_failed {
            let skipped_names: Vec<String> =
                stage.steps.iter().map(|s| s.name.clone()).collect();
            display.steps_skipped(&skipped_names);
            continue;
        }

        let stage_results = if stage.is_parallel() {
            run_parallel_stage(stage, cwd, display, spinner_interval).await?
        } else {
            run_sequential_step(stage.steps[0], cwd, display, spinner_interval).await?
        };

        if stage_results.iter().any(|r| !r.success) {
            stage_failed = true;
        }

        all_results.extend(stage_results);
    }

    display.run_finished(&all_results);
    Ok(all_results)
}

/// Runs a single step, ticking the spinner while it executes.
async fn run_sequential_step(
    step_cfg: &crate::config::Step,
    cwd: &Path,
    display: &mut dyn Display,
    spinner_interval: Option<Duration>,
) -> Result<Vec<StepResult>> {
    display.step_running(&step_cfg.name);

    let result = if let Some(dur) = spinner_interval {
        let mut ticker = tokio::time::interval(dur);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // consume the first immediate tick

        let step_fut = step::run(step_cfg, cwd);
        tokio::pin!(step_fut);

        loop {
            tokio::select! {
                biased;
                result = &mut step_fut => break result?,
                _ = ticker.tick() => display.tick(),
            }
        }
    } else {
        step::run(step_cfg, cwd).await?
    };

    display.step_finished(&result);
    Ok(vec![result])
}

/// Runs all steps in a parallel stage concurrently via JoinSet.
/// Ticks the spinner while waiting for tasks to complete.
/// All steps run to completion before returning — even if some fail.
async fn run_parallel_stage(
    stage: &super::Stage<'_>,
    cwd: &Path,
    display: &mut dyn Display,
    spinner_interval: Option<Duration>,
) -> Result<Vec<StepResult>> {
    // Mark all steps in the stage as running before any start executing.
    for &step_cfg in &stage.steps {
        display.step_running(&step_cfg.name);
    }

    // Spawn each step as a separate task.
    let mut set = JoinSet::new();
    for &step_cfg in &stage.steps {
        let step_owned = step_cfg.clone();
        let cwd_owned = cwd.to_path_buf();
        set.spawn(async move { step::run(&step_owned, &cwd_owned).await });
    }

    let mut stage_results = Vec::with_capacity(stage.steps.len());

    if let Some(dur) = spinner_interval {
        let mut ticker = tokio::time::interval(dur);
        ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ticker.tick().await; // consume the first immediate tick

        while !set.is_empty() {
            tokio::select! {
                biased;
                Some(join_result) = set.join_next() => {
                    let step_result = join_result
                        .context("step task panicked")?
                        .context("step execution failed")?;
                    display.step_finished(&step_result);
                    stage_results.push(step_result);
                }
                _ = ticker.tick() => {
                    display.tick();
                }
            }
        }
    } else {
        while let Some(join_result) = set.join_next().await {
            let step_result = join_result
                .context("step task panicked")?
                .context("step execution failed")?;
            display.step_finished(&step_result);
            stage_results.push(step_result);
        }
    }

    Ok(stage_results)
}
