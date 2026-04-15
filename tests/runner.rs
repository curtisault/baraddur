use baraddur::config::{Config, OutputConfig, Step, WatchConfig};
use baraddur::output::Display;
use baraddur::pipeline;
use baraddur::pipeline::StepResult;

#[derive(Default)]
struct RecordingDisplay {
    events: Vec<String>,
}

impl Display for RecordingDisplay {
    fn run_started(&mut self) {
        self.events.push("run_started".into());
    }
    fn step_started(&mut self, name: &str) {
        self.events.push(format!("start:{name}"));
    }
    fn step_finished(&mut self, r: &StepResult) {
        self.events.push(format!("finish:{}:{}", r.name, r.success));
    }
    fn run_finished(&mut self, _: &[StepResult]) {
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
        steps,
    }
}

#[tokio::test]
async fn stops_at_first_failure() {
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

    let results = pipeline::run_sequential(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    assert_eq!(results.len(), 2, "should stop after the failing step");
    assert!(results[0].success, "first step should pass");
    assert!(!results[1].success, "second step should fail");
    assert!(
        !display.events.iter().any(|e| e.contains("third")),
        "third step must never start"
    );
    assert_eq!(display.events.last().unwrap(), "run_finished");
}

#[tokio::test]
async fn all_steps_run_when_all_pass() {
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

    let results = pipeline::run_sequential(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    assert_eq!(results.len(), 3);
    assert!(results.iter().all(|r| r.success));
}

#[tokio::test]
async fn captures_stdout_and_stderr_on_failure() {
    let cfg = make_config(vec![Step {
        name: "noisyfail".into(),
        cmd: "sh -c 'echo out; echo err >&2; exit 1'".into(),
        parallel: false,
    }]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_sequential(&cfg, &cwd, &mut display)
        .await
        .unwrap();

    assert_eq!(results.len(), 1);
    assert!(!results[0].success);
    assert!(results[0].stdout.contains("out"));
    assert!(results[0].stderr.contains("err"));
}
