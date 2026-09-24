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

        for pat in &s.if_changed {
            if let Err(e) = globset::Glob::new(pat) {
                errs.push(format!(
                    "step {idx} (`{}`) has an invalid if_changed glob `{pat}`: {e}",
                    s.name
                ));
            }
        }
    }

    if cfg.watch.debounce_ms < 50 {
        errs.push(format!(
            "watch.debounce_ms = {} is too small; minimum is 50",
            cfg.watch.debounce_ms
        ));
    }

    let step_names: std::collections::HashSet<&str> =
        cfg.steps.iter().map(|s| s.name.as_str()).collect();
    for (profile_name, members) in &cfg.profiles {
        if profile_name.trim().is_empty() {
            errs.push("profile name is empty".into());
        }
        if members.is_empty() {
            errs.push(format!("profile `{profile_name}` has no members"));
        }
        for member in members {
            if !step_names.contains(member.as_str()) {
                errs.push(format!(
                    "profile `{profile_name}` references unknown step `{member}`"
                ));
            }
        }
    }

    if cfg.on_failure.enabled {
        if cfg.on_failure.cmd.trim().is_empty() {
            errs.push("on_failure.enabled = true but on_failure.cmd is empty".into());
        } else if shell_words::split(&cfg.on_failure.cmd).is_err() {
            errs.push(format!(
                "on_failure.cmd is unparseable: {}",
                cfg.on_failure.cmd
            ));
        }
        if cfg.on_failure.timeout_secs == 0 {
            errs.push("on_failure.timeout_secs must be > 0".into());
        }
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
    use crate::config::schema::{Config, OnFailureConfig, OutputConfig, Step, WatchConfig};

    fn base() -> Config {
        Config {
            watch: WatchConfig {
                extensions: vec!["rs".into()],
                debounce_ms: 1000,
                ignore: vec![],
            },
            output: OutputConfig::default(),
            on_failure: OnFailureConfig::default(),
            steps: vec![Step {
                name: "x".into(),
                cmd: "true".into(),
                parallel: false,
                if_changed: Vec::new(),
            }],
            profiles: std::collections::HashMap::new(),
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
                if_changed: Vec::new(),
            },
            Step {
                name: "x".into(),
                cmd: "true".into(),
                parallel: false,
                if_changed: Vec::new(),
            },
        ];
        let err = validate(&c).unwrap_err();
        assert!(err.to_string().contains("duplicate step name `x`"));
    }

    #[test]
    fn rejects_invalid_glob() {
        let mut c = base();
        c.steps[0].if_changed = vec!["[invalid".into()];
        let err = validate(&c).unwrap_err();
        assert!(err.to_string().contains("invalid if_changed glob"));
    }

    #[test]
    fn accepts_valid_globs() {
        let mut c = base();
        c.steps[0].if_changed = vec!["**/*.rs".into(), "src/**/*.toml".into()];
        assert!(validate(&c).is_ok());
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

    #[test]
    fn on_failure_enabled_requires_cmd() {
        let mut c = base();
        c.on_failure.enabled = true;
        c.on_failure.cmd = String::new();
        let err = validate(&c).unwrap_err();
        assert!(err.to_string().contains("on_failure.cmd"));
    }

    #[test]
    fn on_failure_disabled_ignores_empty_cmd() {
        let mut c = base();
        c.on_failure.enabled = false;
        c.on_failure.cmd = String::new();
        assert!(validate(&c).is_ok());
    }

    #[test]
    fn rejects_profile_referencing_unknown_step() {
        let mut c = base();
        c.profiles
            .insert("quick".into(), vec!["x".into(), "ghost".into()]);
        let err = validate(&c).unwrap_err();
        let s = err.to_string();
        assert!(s.contains("profile `quick`"));
        assert!(s.contains("ghost"));
    }

    #[test]
    fn accepts_profile_with_defined_steps() {
        let mut c = base();
        c.steps.push(Step {
            name: "y".into(),
            cmd: "true".into(),
            parallel: false,
            if_changed: Vec::new(),
        });
        c.profiles.insert("quick".into(), vec!["x".into()]);
        c.profiles
            .insert("full".into(), vec!["x".into(), "y".into()]);
        assert!(validate(&c).is_ok());
    }

    #[test]
    fn rejects_empty_profile() {
        let mut c = base();
        c.profiles.insert("quick".into(), vec![]);
        let err = validate(&c).unwrap_err();
        assert!(err.to_string().contains("no members"));
    }

    #[test]
    fn on_failure_rejects_zero_timeout() {
        let mut c = base();
        c.on_failure.enabled = true;
        c.on_failure.cmd = "cat".into();
        c.on_failure.timeout_secs = 0;
        let err = validate(&c).unwrap_err();
        assert!(err.to_string().contains("timeout_secs"));
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use crate::config::schema::{Config, OnFailureConfig, OutputConfig, Step, WatchConfig};
    use proptest::prelude::*;
    use std::collections::HashMap;

    fn valid_glob() -> impl Strategy<Value = String> {
        prop::sample::select(vec!["**/*.rs", "src/**", "*.toml", "docs/*.md"])
            .prop_map(str::to_owned)
    }

    /// A config that satisfies every rule `validate` checks: unique
    /// non-blank names, tokenizable non-blank cmds, valid globs, sane
    /// debounce, profiles that reference real steps, and a hook that is
    /// either off or fully specified.
    fn valid_config() -> impl Strategy<Value = Config> {
        let step_names = prop::collection::hash_set("[a-z][a-z0-9_-]{0,7}", 1..6);
        step_names
            .prop_flat_map(|names| {
                let names: Vec<String> = names.into_iter().collect();
                let n = names.len();
                let steps = prop::collection::vec(
                    (
                        "[a-z]+( [a-z0-9=-]+){0,3}",
                        any::<bool>(),
                        prop::collection::vec(valid_glob(), 0..3),
                    ),
                    n,
                )
                .prop_map(move |specs| {
                    specs
                        .into_iter()
                        .zip(names.iter())
                        .map(|((cmd, parallel, if_changed), name)| Step {
                            name: name.clone(),
                            cmd,
                            parallel,
                            if_changed,
                        })
                        .collect::<Vec<Step>>()
                });
                let profiles = prop::collection::hash_map(
                    "[a-z]{1,6}",
                    prop::collection::vec(any::<prop::sample::Index>(), 1..4),
                    0..3,
                );
                (steps, profiles)
            })
            .prop_map(|(steps, profiles)| {
                let profiles: HashMap<String, Vec<String>> = profiles
                    .into_iter()
                    .map(|(k, idxs)| {
                        let members = idxs
                            .iter()
                            .map(|i| steps[i.index(steps.len())].name.clone())
                            .collect();
                        (k, members)
                    })
                    .collect();
                (steps, profiles)
            })
            .prop_flat_map(|(steps, profiles)| {
                (
                    Just(steps),
                    Just(profiles),
                    50u64..10_000,
                    any::<bool>(),
                    "[a-z]+( [a-z]+){0,2}",
                    1u64..120,
                )
            })
            .prop_map(
                |(steps, profiles, debounce_ms, enabled, cmd, timeout_secs)| Config {
                    watch: WatchConfig {
                        extensions: vec!["rs".into()],
                        debounce_ms,
                        ignore: vec![],
                    },
                    output: OutputConfig::default(),
                    on_failure: OnFailureConfig {
                        enabled,
                        cmd,
                        prompt: String::new(),
                        timeout_secs,
                    },
                    steps,
                    profiles,
                },
            )
    }

    proptest! {
        /// Any config built inside the rules validates cleanly. Guards
        /// against a rule accidentally tightening past its documented bound
        /// (e.g. rejecting `debounce_ms = 50` or a one-step profile).
        #[test]
        fn well_formed_configs_validate(cfg in valid_config()) {
            let result = validate(&cfg);
            prop_assert!(result.is_ok(), "unexpected errors: {}", result.unwrap_err());
        }

        /// Each independent fault injected into a valid config yields
        /// exactly one error, and they accumulate rather than short-circuit.
        #[test]
        fn faults_are_reported_one_each(
            mut cfg in valid_config(),
            dup_name in any::<bool>(),
            blank_cmd in any::<bool>(),
            bad_glob in any::<bool>(),
            tiny_debounce in any::<bool>(),
            ghost_profile in any::<bool>(),
        ) {
            let mut expected = 0;
            if dup_name {
                let clone = Step {
                    name: cfg.steps[0].name.clone(),
                    cmd: "true".into(),
                    parallel: false,
                    if_changed: Vec::new(),
                };
                cfg.steps.push(clone);
                expected += 1;
            }
            if blank_cmd {
                cfg.steps[0].cmd = "   ".into();
                expected += 1;
            }
            if bad_glob {
                cfg.steps[0].if_changed.push("[unclosed".into());
                expected += 1;
            }
            if tiny_debounce {
                cfg.watch.debounce_ms = 49;
                expected += 1;
            }
            if ghost_profile {
                cfg.profiles.insert("zz-ghost".into(), vec!["no-such-step".into()]);
                expected += 1;
            }

            match validate(&cfg) {
                Ok(()) => prop_assert_eq!(expected, 0),
                Err(errs) => prop_assert_eq!(errs.0.len(), expected, "errors: {}", errs),
            }
        }
    }
}
