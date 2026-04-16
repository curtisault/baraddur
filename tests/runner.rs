use baraddur::config::{Config, OutputConfig, Step, SummarizeConfig, WatchConfig};
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
        summarize: SummarizeConfig::default(),
        steps,
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
        },
        Step {
            name: "second".into(),
            cmd: "false".into(),
            parallel: false,
        },
        Step {
            name: "third".into(),
            cmd: "true".into(),
            parallel: false,
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
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
        },
        Step {
            name: "b".into(),
            cmd: "true".into(),
            parallel: false,
        },
        Step {
            name: "c".into(),
            cmd: "true".into(),
            parallel: false,
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
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
        },
        Step {
            name: "b".into(),
            cmd: "true".into(),
            parallel: true,
        },
        Step {
            name: "c".into(),
            cmd: "true".into(),
            parallel: true,
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
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
        },
        Step {
            name: "fail".into(),
            cmd: "false".into(),
            parallel: true,
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
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
        },
        Step {
            name: "slow_b".into(),
            cmd: "sleep 0.3".into(),
            parallel: true,
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let start = std::time::Instant::now();
    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
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
        },
        Step {
            name: "par_a".into(),
            cmd: "true".into(),
            parallel: true,
        },
        Step {
            name: "par_b".into(),
            cmd: "true".into(),
            parallel: true,
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
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
        },
        Step {
            name: "skip_a".into(),
            cmd: "true".into(),
            parallel: true,
        },
        Step {
            name: "skip_b".into(),
            cmd: "true".into(),
            parallel: true,
        },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
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
    }]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_pipeline(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert!(results[0].stdout.contains("out"));
    assert!(results[0].stderr.contains("err"));
}
