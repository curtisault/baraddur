use baraddur::config::{Config, OnFailureConfig, OutputConfig, Step, WatchConfig};
use baraddur::output::Display;
use baraddur::pipeline;
use baraddur::pipeline::StepResult;

/// Test display that records all lifecycle events for assertion.
#[derive(Default)]
struct RecordingDisplay {
    events: Vec<String>,
}

impl Display for RecordingDisplay {
    fn run_started(&mut self, step_names: &[String]) {
        self.events
            .push(format!("run_started:{}", step_names.join(",")));
    }

    fn step_running(&mut self, name: &str) {
        self.events.push(format!("running:{name}"));
    }

    fn step_finished(&mut self, r: &StepResult) {
        self.events
            .push(format!("finished:{}:{}", r.name, r.success));
    }

    fn steps_skipped(&mut self, names: &[String]) {
        for name in names {
            self.events.push(format!("skipped:{name}"));
        }
    }

    fn run_cancelled(&mut self) {
        self.events.push("run_cancelled".into());
    }

    fn run_finished(&mut self, _results: &[StepResult]) {
        self.events.push("run_finished".into());
    }
}

fn make_config(steps: Vec<Step>) -> Config {
    Config {
        watch: WatchConfig {
            extensions: vec!["rs".into()],
            debounce_ms: 1000,
            ignore: vec![],
        },
        output: OutputConfig::default(),
        on_failure: OnFailureConfig::default(),
        steps,
        profiles: std::collections::HashMap::new(),
    }
}

// ── Sequential behavior ──────────────────────────────────────────────────────

#[tokio::test]
async fn sequential_stops_at_first_failure() {
    let cfg = make_config(vec![
        Step {
            name: "first".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
        Step {
            name: "second".into(),
            cmd: "false".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
        Step {
            name: "third".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();

    // first passes, second fails, third is skipped.
    assert_eq!(results.len(), 2);
    assert!(results[0].success);
    assert!(!results[1].success);

    assert!(display.events.contains(&"skipped:third".to_string()));
    assert!(
        !display
            .events
            .iter()
            .any(|e| e.starts_with("running:third"))
    );
}

#[tokio::test]
async fn sequential_all_pass() {
    let cfg = make_config(vec![
        Step {
            name: "a".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
        Step {
            name: "b".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
        Step {
            name: "c".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.success));
}

// ── Parallel execution ───────────────────────────────────────────────────────

#[tokio::test]
async fn parallel_steps_all_run() {
    let cfg = make_config(vec![
        Step {
            name: "a".into(),
            cmd: "true".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
        Step {
            name: "b".into(),
            cmd: "true".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
        Step {
            name: "c".into(),
            cmd: "true".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.success));

    // All three should have been marked running before any finished.
    let events = &display.events;
    let running_indices: Vec<usize> = events
        .iter()
        .enumerate()
        .filter(|(_, e)| e.starts_with("running:"))
        .map(|(i, _)| i)
        .collect();
    let first_finished = events
        .iter()
        .position(|e| e.starts_with("finished:"))
        .unwrap();
    assert!(
        running_indices.iter().all(|&i| i < first_finished),
        "all steps should be marked running before any finish"
    );
}

#[tokio::test]
async fn parallel_stage_runs_all_even_if_one_fails() {
    let cfg = make_config(vec![
        Step {
            name: "pass".into(),
            cmd: "true".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
        Step {
            name: "fail".into(),
            cmd: "false".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();

    // Both steps ran — even though one failed.
    assert_eq!(results.len(), 2);
    assert!(results.iter().any(|r| r.success));
    assert!(results.iter().any(|r| !r.success));
}

#[tokio::test]
async fn parallel_wall_clock_is_max_not_sum() {
    // Two steps that each sleep 0.3s. If parallel, wall clock should be
    // ~0.3s, not ~0.6s. Allow generous margin for CI.
    let cfg = make_config(vec![
        Step {
            name: "slow_a".into(),
            cmd: "sleep 0.3".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
        Step {
            name: "slow_b".into(),
            cmd: "sleep 0.3".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let start = std::time::Instant::now();
    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();
    let elapsed = start.elapsed();

    assert_eq!(results.len(), 2);
    assert!(
        elapsed.as_secs_f64() < 0.55,
        "parallel steps took {:.2}s — expected under 0.55s",
        elapsed.as_secs_f64()
    );
}

// ── Mixed stages ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn mixed_stages_sequential_then_parallel() {
    let cfg = make_config(vec![
        Step {
            name: "seq".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
        Step {
            name: "par_a".into(),
            cmd: "true".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
        Step {
            name: "par_b".into(),
            cmd: "true".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.success));

    let events = &display.events;
    // seq must finish before par_a/par_b start running.
    let seq_finished = events
        .iter()
        .position(|e| e == "finished:seq:true")
        .unwrap();
    let par_a_running = events.iter().position(|e| e == "running:par_a").unwrap();
    assert!(seq_finished < par_a_running);
}

#[tokio::test]
async fn stage_failure_skips_subsequent_stages() {
    let cfg = make_config(vec![
        Step {
            name: "fail".into(),
            cmd: "false".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
        Step {
            name: "skip_a".into(),
            cmd: "true".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
        Step {
            name: "skip_b".into(),
            cmd: "true".into(),
            parallel: true,
            if_changed: Vec::new(),
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();

    // Only the failing step ran.
    assert_eq!(results.len(), 1);
    assert!(!results[0].success);

    assert!(display.events.contains(&"skipped:skip_a".to_string()));
    assert!(display.events.contains(&"skipped:skip_b".to_string()));
}

// ── Output capture ───────────────────────────────────────────────────────────

#[tokio::test]
async fn captures_stdout_and_stderr_on_failure() {
    let cfg = make_config(vec![Step {
        name: "noisyfail".into(),
        cmd: "sh -c 'echo out; echo err >&2; exit 1'".into(),
        parallel: false,
        if_changed: Vec::new(),
    }]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert!(results[0].stdout.contains("out"));
    assert!(results[0].stderr.contains("err"));
}

// ── Path-based filtering ─────────────────────────────────────────────────────

#[tokio::test]
async fn trigger_excludes_steps_with_no_glob_matches() {
    use std::path::PathBuf;

    let cfg = make_config(vec![
        Step {
            name: "rust".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: vec!["**/*.rs".into()],
        },
        Step {
            name: "ts".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: vec!["**/*.ts".into()],
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    // Only a .ts file changed — the rust step should be excluded entirely.
    let trigger = vec![PathBuf::from("src/app.ts")];
    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, Some(&trigger), None)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "ts");
    assert!(
        !display.events.iter().any(|e| e.contains("rust")),
        "rust step should not appear in events"
    );
}

#[tokio::test]
async fn files_template_substitutes_matched_paths() {
    use std::path::PathBuf;

    // `printf %s {files}` writes the substituted paths to stdout — easy to
    // assert on.
    let cfg = make_config(vec![Step {
        name: "echo".into(),
        cmd: "printf %s {files}".into(),
        parallel: false,
        if_changed: vec!["**/*.rs".into()],
    }]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let trigger = vec![PathBuf::from("src/a.rs"), PathBuf::from("README.md")];
    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, Some(&trigger), None)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    // Only the .rs path should have been substituted.
    assert_eq!(results[0].stdout, "src/a.rs");
}

#[tokio::test]
async fn initial_run_runs_all_steps_ignoring_if_changed() {
    let cfg = make_config(vec![Step {
        name: "rust".into(),
        cmd: "true".into(),
        parallel: false,
        if_changed: vec!["**/*.rs".into()],
    }]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    // Initial run = trigger is None. Step must run despite if_changed.
    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, None)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(results[0].success);
}

#[tokio::test]
async fn only_steps_narrows_to_named_subset() {
    // Simulates the browse-mode `f` key: rerun only steps that previously failed.
    let cfg = make_config(vec![
        Step {
            name: "a".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
        Step {
            name: "b".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
        Step {
            name: "c".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let only = vec!["b".to_string()];
    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, Some(&only))
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert_eq!(results[0].name, "b");
    // The other steps shouldn't appear in the recorded events.
    assert!(
        !display
            .events
            .iter()
            .any(|e| e.ends_with(":a") || e.ends_with(":c")),
        "only step `b` should appear; got: {:?}",
        display.events
    );
}

#[tokio::test]
async fn only_steps_empty_runs_nothing() {
    // Edge: pressing `f` after an all-pass run gives an empty filter; pipeline
    // should run zero steps but not error.
    let cfg = make_config(vec![Step {
        name: "a".into(),
        cmd: "true".into(),
        parallel: false,
        if_changed: Vec::new(),
    }]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let only: Vec<String> = Vec::new();
    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display, None, None, Some(&only))
        .await
        .unwrap();

    assert!(results.is_empty());
}

// ── Property: failing stage skips everything after it ───────────────────────

mod proptests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::HashSet;

    /// Steps that either pass (`true`) or fail (`false`), with random
    /// parallel flags, so every stage shape and failure position is covered.
    fn arb_steps() -> impl Strategy<Value = Vec<Step>> {
        prop::collection::vec((any::<bool>(), any::<bool>()), 0..8).prop_map(|specs| {
            specs
                .into_iter()
                .enumerate()
                .map(|(i, (passes, parallel))| Step {
                    name: format!("s{i}"),
                    cmd: if passes { "true" } else { "false" }.into(),
                    parallel,
                    if_changed: Vec::new(),
                })
                .collect()
        })
    }

    fn run(cfg: &Config, only: Option<&[String]>) -> (Vec<StepResult>, Vec<String>) {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        let mut display = RecordingDisplay::default();
        let results = rt
            .block_on(pipeline::run_pipeline(
                cfg,
                &std::env::temp_dir(),
                &mut display,
                None,
                None,
                only,
            ))
            .unwrap();
        (results, display.events)
    }

    proptest! {
        // Every case spawns real processes; keep the count modest.
        #![proptest_config(ProptestConfig::with_cases(48))]

        /// Model: after `only_steps` narrowing, stages run in order until
        /// the first stage containing a failing step; that stage completes
        /// in full and every later stage is skipped. Results carry exactly
        /// the steps that ran, each with the success its command implies,
        /// and the display sees the skipped names exactly once each.
        #[test]
        fn failing_stage_skips_all_later_stages(
            steps in arb_steps(),
            only in prop::option::of(prop::collection::hash_set(0usize..8, 0..8)),
        ) {
            let only_names: Option<Vec<String>> =
                only.map(|idxs| idxs.into_iter().map(|i| format!("s{i}")).collect());

            let active: Vec<Step> = steps
                .iter()
                .filter(|s| only_names.as_ref().is_none_or(|n| n.contains(&s.name)))
                .cloned()
                .collect();

            let mut expect_ran: Vec<String> = Vec::new();
            let mut expect_skipped: Vec<String> = Vec::new();
            let mut failed = false;
            for stage in pipeline::group_into_stages(&active) {
                for s in &stage.steps {
                    if failed {
                        expect_skipped.push(s.name.clone());
                    } else {
                        expect_ran.push(s.name.clone());
                    }
                }
                if stage.steps.iter().any(|s| s.cmd == "false") {
                    failed = true;
                }
            }

            let cfg = make_config(steps.clone());
            let (results, events) = run(&cfg, only_names.as_deref());

            // Parallel stages complete in arbitrary order, so compare as sets
            // for membership and rely on stage order for everything else.
            let ran: HashSet<&str> = results.iter().map(|r| r.name.as_str()).collect();
            let expected: HashSet<&str> = expect_ran.iter().map(String::as_str).collect();
            prop_assert_eq!(results.len(), expect_ran.len(), "duplicate or missing results");
            prop_assert_eq!(ran, expected);

            for r in &results {
                let step = steps.iter().find(|s| s.name == r.name).unwrap();
                prop_assert_eq!(r.success, step.cmd == "true", "step {}", r.name);
            }

            let skipped: Vec<String> = events
                .iter()
                .filter_map(|e| e.strip_prefix("skipped:"))
                .map(str::to_owned)
                .collect();
            prop_assert_eq!(skipped, expect_skipped);

            let active_names: Vec<String> = active.iter().map(|s| s.name.clone()).collect();
            prop_assert_eq!(&events[0], &format!("run_started:{}", active_names.join(",")));
            prop_assert_eq!(events.last().map(String::as_str), Some("run_finished"));
        }
    }
}
