pub mod filter;
pub mod hook;
pub mod runner;
pub mod step;

pub use filter::filter_and_template;
pub use hook::{combine_failed_output, run_hook};
pub use runner::run_pipeline;
pub use step::{StepResult, run as run_step};

use crate::config::Step;

/// A group of steps that execute together as one stage of the pipeline.
///
/// - A stage with one step runs that step directly.
/// - A stage with multiple steps runs them all concurrently via `JoinSet`.
#[derive(Debug)]
pub struct Stage<'a> {
    pub steps: Vec<&'a Step>,
}

impl Stage<'_> {
    /// A stage is parallel when it contains more than one step.
    /// (A single `parallel = true` step is still alone in its stage
    /// and doesn't need concurrent machinery.)
    pub fn is_parallel(&self) -> bool {
        self.steps.len() > 1
    }
}

/// Groups steps into stages by consecutive `parallel` flag.
///
/// Sequential steps (`parallel = false`) each become their own stage.
/// Consecutive parallel steps (`parallel = true`) are batched into one stage.
pub fn group_into_stages(steps: &[Step]) -> Vec<Stage<'_>> {
    let mut stages = Vec::new();
    let mut i = 0;

    while i < steps.len() {
        if steps[i].parallel {
            // Collect consecutive parallel steps into one stage.
            let start = i;
            while i < steps.len() && steps[i].parallel {
                i += 1;
            }
            stages.push(Stage {
                steps: steps[start..i].iter().collect(),
            });
        } else {
            // Sequential step = its own stage.
            stages.push(Stage {
                steps: vec![&steps[i]],
            });
            i += 1;
        }
    }

    stages
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Step;

    fn step(name: &str, parallel: bool) -> Step {
        Step {
            name: name.into(),
            cmd: "true".into(),
            parallel,
            if_changed: Vec::new(),
        }
    }

    #[test]
    fn all_sequential() {
        let steps = vec![step("a", false), step("b", false), step("c", false)];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].steps.len(), 1);
        assert_eq!(stages[0].steps[0].name, "a");
        assert!(!stages[0].is_parallel());
    }

    #[test]
    fn all_parallel() {
        let steps = vec![step("a", true), step("b", true), step("c", true)];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 1);
        assert_eq!(stages[0].steps.len(), 3);
        assert!(stages[0].is_parallel());
    }

    #[test]
    fn mixed_sequential_then_parallel() {
        let steps = vec![
            step("format", false),
            step("compile", false),
            step("credo", true),
            step("test", true),
        ];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].steps[0].name, "format");
        assert_eq!(stages[1].steps[0].name, "compile");
        assert_eq!(stages[2].steps.len(), 2);
        assert_eq!(stages[2].steps[0].name, "credo");
        assert_eq!(stages[2].steps[1].name, "test");
    }

    #[test]
    fn parallel_then_sequential() {
        let steps = vec![step("a", true), step("b", true), step("c", false)];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 2);
        assert_eq!(stages[0].steps.len(), 2);
        assert_eq!(stages[1].steps.len(), 1);
    }

    #[test]
    fn single_step() {
        let steps = vec![step("only", false)];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 1);
        assert!(!stages[0].is_parallel());
    }

    #[test]
    fn empty_steps() {
        let stages = group_into_stages(&[]);
        assert!(stages.is_empty());
    }

    #[test]
    fn alternating_parallel_sequential() {
        let steps = vec![
            step("a", true),
            step("b", true),
            step("c", false),
            step("d", true),
            step("e", true),
        ];
        let stages = group_into_stages(&steps);
        assert_eq!(stages.len(), 3);
        assert_eq!(stages[0].steps.len(), 2); // a, b
        assert_eq!(stages[1].steps.len(), 1); // c
        assert_eq!(stages[2].steps.len(), 2); // d, e
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;

    /// Steps whose only interesting property is the `parallel` flag; names
    /// are unique so the flattened output can be compared positionally.
    fn steps() -> impl Strategy<Value = Vec<Step>> {
        prop::collection::vec(any::<bool>(), 0..32).prop_map(|flags| {
            flags
                .into_iter()
                .enumerate()
                .map(|(i, parallel)| Step {
                    name: format!("s{i}"),
                    cmd: "true".into(),
                    parallel,
                    if_changed: Vec::new(),
                })
                .collect()
        })
    }

    proptest! {
        /// Grouping is a partition: flattening the stages gives back every
        /// step exactly once, in declaration order.
        #[test]
        fn stages_partition_steps_in_order(steps in steps()) {
            let stages = group_into_stages(&steps);
            let flattened: Vec<&str> = stages
                .iter()
                .flat_map(|st| st.steps.iter().map(|s| s.name.as_str()))
                .collect();
            let expected: Vec<&str> = steps.iter().map(|s| s.name.as_str()).collect();
            prop_assert_eq!(flattened, expected);
            prop_assert!(stages.iter().all(|st| !st.steps.is_empty()));
        }

        /// A sequential step is always alone in its stage, so `is_parallel`
        /// can only be true for stages made entirely of parallel steps.
        #[test]
        fn sequential_steps_are_never_batched(steps in steps()) {
            for stage in group_into_stages(&steps) {
                if stage.steps.iter().any(|s| !s.parallel) {
                    prop_assert_eq!(stage.steps.len(), 1);
                    prop_assert!(!stage.is_parallel());
                }
            }
        }

        /// Runs of consecutive parallel steps are batched maximally: two
        /// adjacent stages are never both parallel-only.
        #[test]
        fn parallel_runs_are_maximal(steps in steps()) {
            let stages = group_into_stages(&steps);
            for pair in stages.windows(2) {
                let both_parallel = pair
                    .iter()
                    .all(|st| st.steps.iter().all(|s| s.parallel));
                prop_assert!(!both_parallel, "adjacent parallel stages should have merged");
            }
        }

        /// Stage count equals the number of sequential steps plus the number
        /// of maximal parallel runs — a direct restatement of the grouping rule.
        #[test]
        fn stage_count_matches_flag_runs(steps in steps()) {
            let mut expected = 0;
            let mut prev_parallel = false;
            for s in &steps {
                if !s.parallel || !prev_parallel {
                    expected += 1;
                }
                prev_parallel = s.parallel;
            }
            prop_assert_eq!(group_into_stages(&steps).len(), expected);
        }
    }
}
