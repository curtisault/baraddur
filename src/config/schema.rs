use serde::Deserialize;

#[derive(Debug, Deserialize)]
pub struct Config {
    pub watch: WatchConfig,

    #[serde(default)]
    pub output: OutputConfig,

    pub steps: Vec<Step>,
}

#[derive(Debug, Deserialize)]
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

#[derive(Debug, Deserialize)]
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
}
