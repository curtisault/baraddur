use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub watch: WatchConfig,

    #[serde(default)]
    pub output: OutputConfig,

    #[serde(default)]
    pub on_failure: OnFailureConfig,

    #[serde(default)]
    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WatchConfig {
    pub extensions: Vec<String>,

    #[serde(default = "default_debounce_ms")]
    pub debounce_ms: u64,

    #[serde(default)]
    pub ignore: Vec<String>,
}

fn default_debounce_ms() -> u64 {
    1500
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputConfig {
    #[serde(default = "default_true")]
    pub clear_screen: bool,

    #[serde(default)]
    pub show_passing: bool,
}

impl Default for OutputConfig {
    fn default() -> Self {
        Self {
            clear_screen: true,
            show_passing: false,
        }
    }
}

fn default_true() -> bool {
    true
}

/// Post-failure hook. When `enabled` and any step in a completed run fails,
/// `cmd` is spawned with the combined stdout/stderr of failed steps on stdin
/// (optionally preceded by `prompt`). Its stdout is shown below the failure
/// summary. Cancelled on file change or shutdown.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OnFailureConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default)]
    pub cmd: String,

    #[serde(default)]
    pub prompt: String,

    #[serde(default = "default_on_failure_timeout")]
    pub timeout_secs: u64,
}

impl Default for OnFailureConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cmd: String::new(),
            prompt: String::new(),
            timeout_secs: default_on_failure_timeout(),
        }
    }
}

fn default_on_failure_timeout() -> u64 {
    30
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    pub cmd: String,

    #[serde(default)]
    pub parallel: bool,

    /// Optional glob patterns. When a run is triggered by file changes, the
    /// step runs only if at least one changed path matches a pattern. Empty
    /// (default) means "always run". Matched against project-relative paths.
    #[serde(default)]
    pub if_changed: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_config() {
        let src = r#"
            [watch]
            extensions = ["rs"]

            [[steps]]
            name = "check"
            cmd = "cargo check"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert_eq!(cfg.watch.extensions, vec!["rs"]);
        assert_eq!(cfg.watch.debounce_ms, 1500);
        assert!(cfg.output.clear_screen);
        assert_eq!(cfg.steps.len(), 1);
        assert_eq!(cfg.steps[0].name, "check");
        assert!(!cfg.steps[0].parallel);
    }

    #[test]
    fn parses_full_config() {
        let src = r#"
            [watch]
            extensions = ["ex", "exs", "heex"]
            debounce_ms = 1000
            ignore = ["_build", "deps", ".git"]

            [output]
            clear_screen = false
            show_passing = true

            [[steps]]
            name = "format"
            cmd = "mix format --check-formatted"
            parallel = false

            [[steps]]
            name = "credo"
            cmd = "mix credo"
            parallel = true
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert_eq!(cfg.watch.debounce_ms, 1000);
        assert_eq!(cfg.watch.ignore, vec!["_build", "deps", ".git"]);
        assert!(!cfg.output.clear_screen);
        assert!(cfg.output.show_passing);
        assert_eq!(cfg.steps.len(), 2);
        assert!(cfg.steps[1].parallel);
    }

    #[test]
    fn rejects_unknown_top_level_field() {
        let src = r#"
            nonsense = true
            [watch]
            extensions = ["rs"]
            [[steps]]
            name = "c"
            cmd = "true"
        "#;
        let err = toml::from_str::<Config>(src).unwrap_err();
        assert!(err.to_string().contains("unknown field"));
        assert!(err.to_string().contains("nonsense"));
    }

    #[test]
    fn rejects_unknown_step_field() {
        let src = r#"
            [watch]
            extensions = ["rs"]
            [[steps]]
            name = "c"
            cmd = "true"
            parralel = false
        "#;
        let err = toml::from_str::<Config>(src).unwrap_err();
        assert!(err.to_string().contains("parralel"));
    }

    #[test]
    fn parses_on_failure_table() {
        let src = r#"
            [watch]
            extensions = ["rs"]

            [on_failure]
            enabled = true
            cmd = "claude -p"
            prompt = "summarize"
            timeout_secs = 45

            [[steps]]
            name = "c"
            cmd = "true"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert!(cfg.on_failure.enabled);
        assert_eq!(cfg.on_failure.cmd, "claude -p");
        assert_eq!(cfg.on_failure.prompt, "summarize");
        assert_eq!(cfg.on_failure.timeout_secs, 45);
    }

    #[test]
    fn on_failure_defaults_disabled() {
        let src = r#"
            [watch]
            extensions = ["rs"]
            [[steps]]
            name = "c"
            cmd = "true"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert!(!cfg.on_failure.enabled);
        assert!(cfg.on_failure.cmd.is_empty());
        assert_eq!(cfg.on_failure.timeout_secs, 30);
    }
}
