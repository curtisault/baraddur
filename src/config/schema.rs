use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub watch: WatchConfig,

    #[serde(default)]
    pub output: OutputConfig,

    #[serde(default)]
    pub summarize: SummarizeConfig,

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

/// Reserved for Phase 5. Parsed and stored, but not consumed anywhere yet.
/// Defining it now means `deny_unknown_fields` on `Config` doesn't reject
/// users who add `[summarize]` early.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SummarizeConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "default_summarize_cmd")]
    pub cmd: String,

    #[serde(default = "default_summarize_prompt")]
    pub prompt: String,

    #[serde(default = "default_summarize_timeout")]
    pub timeout_secs: u64,
}

impl Default for SummarizeConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            cmd: default_summarize_cmd(),
            prompt: default_summarize_prompt(),
            timeout_secs: default_summarize_timeout(),
        }
    }
}

fn default_summarize_cmd() -> String {
    "claude".into()
}

fn default_summarize_prompt() -> String {
    "Summarize these check failures in under 5 lines. Focus on root cause \
     and cite file:line where possible."
        .into()
}

fn default_summarize_timeout() -> u64 {
    15
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    pub cmd: String,

    #[serde(default)]
    pub parallel: bool,
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
    fn accepts_summarize_table() {
        let src = r#"
            [watch]
            extensions = ["rs"]

            [summarize]
            enabled = true
            cmd = "claude"
            timeout_secs = 30

            [[steps]]
            name = "c"
            cmd = "true"
        "#;
        let cfg: Config = toml::from_str(src).unwrap();
        assert!(cfg.summarize.enabled);
        assert_eq!(cfg.summarize.timeout_secs, 30);
        assert!(cfg.summarize.prompt.contains("Summarize"));
    }
}
