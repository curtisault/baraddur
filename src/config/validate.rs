use super::schema::Config;

/// Human-readable validation errors, one per line.
#[derive(Debug)]
pub struct ValidationErrors(pub Vec<String>);

impl std::fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, msg) in self.0.iter().enumerate() {
            if i > 0 {
                writeln!(f)?;
            }
            write!(f, "  {msg}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

/// Validates a parsed config for semantic errors.
///
/// All errors are accumulated and returned together so the user sees
/// everything broken in one run.
pub fn validate(cfg: &Config) -> Result<(), ValidationErrors> {
    let mut errs: Vec<String> = Vec::new();

    if cfg.steps.is_empty() {
        errs.push("no steps defined — add at least one [[steps]] entry".into());
    }

    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (i, s) in cfg.steps.iter().enumerate() {
        let idx = i + 1;

        if s.name.trim().is_empty() {
            errs.push(format!("step {idx} has an empty `name`"));
        } else if !seen.insert(s.name.as_str()) {
            errs.push(format!("duplicate step name `{}` (step {idx})", s.name));
        }

        if s.cmd.trim().is_empty() {
            errs.push(format!("step {idx} (`{}`) has an empty `cmd`", s.name));
        } else if shell_words::split(&s.cmd).is_err() {
            errs.push(format!(
                "step {idx} (`{}`) has an unparseable `cmd`: {}",
                s.name, s.cmd
            ));
        }
    }

    if cfg.watch.debounce_ms < 50 {
        errs.push(format!(
            "watch.debounce_ms = {} is too small; minimum is 50",
            cfg.watch.debounce_ms
        ));
    }

    if cfg.summarize.timeout_secs == 0 {
        errs.push("summarize.timeout_secs must be > 0".into());
    }

    if errs.is_empty() {
        Ok(())
    } else {
        Err(ValidationErrors(errs))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::schema::{Config, OutputConfig, Step, SummarizeConfig, WatchConfig};

    fn base() -> Config {
        Config {
            watch: WatchConfig {
                extensions: vec!["rs".into()],
                debounce_ms: 1000,
                ignore: vec![],
            },
            output: OutputConfig::default(),
            summarize: SummarizeConfig::default(),
            steps: vec![Step {
                name: "x".into(),
                cmd: "true".into(),
                parallel: false,
            }],
        }
    }

    #[test]
    fn accepts_valid_config() {
        assert!(validate(&base()).is_ok());
    }

    #[test]
    fn rejects_empty_steps() {
        let mut c = base();
        c.steps.clear();
        let err = validate(&c).unwrap_err();
        assert!(err.to_string().contains("no steps"));
    }

    #[test]
    fn rejects_duplicate_step_names() {
        let mut c = base();
        c.steps = vec![
            Step {
                name: "x".into(),
                cmd: "true".into(),
                parallel: false,
            },
            Step {
                name: "x".into(),
                cmd: "true".into(),
                parallel: false,
            },
        ];
        let err = validate(&c).unwrap_err();
        assert!(err.to_string().contains("duplicate step name `x`"));
    }

    #[test]
    fn rejects_empty_name_and_cmd() {
        let mut c = base();
        c.steps[0].name = "".into();
        c.steps[0].cmd = "".into();
        let err = validate(&c).unwrap_err();
        let s = err.to_string();
        assert!(s.contains("empty `name`"));
        assert!(s.contains("empty `cmd`"));
    }

    #[test]
    fn rejects_tiny_debounce() {
        let mut c = base();
        c.watch.debounce_ms = 5;
        let err = validate(&c).unwrap_err();
        assert!(err.to_string().contains("debounce_ms"));
    }

    #[test]
    fn reports_all_errors_at_once() {
        let mut c = base();
        c.steps[0].name = "".into();
        c.steps[0].cmd = "".into();
        c.watch.debounce_ms = 5;
        let err = validate(&c).unwrap_err();
        assert_eq!(err.0.len(), 3, "should accumulate all errors");
    }

    #[test]
    fn empty_extensions_is_valid() {
        let mut c = base();
        c.watch.extensions.clear();
        assert!(validate(&c).is_ok());
    }
}
