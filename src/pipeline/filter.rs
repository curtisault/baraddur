//! Path-based step filtering and `{files}` template substitution.
//!
//! Run-time policy:
//! - Initial run (`trigger == None`): every step runs. `{files}` substitutes to
//!   an empty string, so `cargo test {files}` becomes `cargo test`.
//! - File-change run (`trigger == Some(paths)`):
//!   - A step with no `if_changed` runs unconditionally; `{files}` substitutes
//!     to every changed path.
//!   - A step with `if_changed` runs only if at least one changed path matches
//!     a pattern; `{files}` substitutes to only the matched paths.
//!
//! Substituted file lists are shell-quoted via `shell_words::quote` so paths
//! containing spaces tokenize correctly.

use anyhow::{Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use std::path::Path;

use crate::config::Step;

/// Returns the subset of `steps` that should run given `trigger`, with
/// `{files}` substituted in each step's `cmd`. Step order is preserved.
pub fn filter_and_template<P: AsRef<Path>>(
    steps: &[Step],
    trigger: Option<&[P]>,
) -> Result<Vec<Step>> {
    let mut out = Vec::with_capacity(steps.len());
    for step in steps {
        let matched: Vec<std::path::PathBuf> = match trigger {
            None => Vec::new(),
            Some(paths) => {
                if step.if_changed.is_empty() {
                    paths.iter().map(|p| p.as_ref().to_path_buf()).collect()
                } else {
                    let set = build_globset(&step.if_changed)
                        .with_context(|| format!("step `{}`", step.name))?;
                    paths
                        .iter()
                        .map(|p| p.as_ref().to_path_buf())
                        .filter(|p| set.is_match(p))
                        .collect()
                }
            }
        };

        // Filter decision:
        // - No trigger (initial run): always include.
        // - Trigger + no if_changed: always include.
        // - Trigger + if_changed: include only if at least one match.
        let include = trigger.is_none() || step.if_changed.is_empty() || !matched.is_empty();
        if !include {
            continue;
        }

        let mut new_step = step.clone();
        if step.cmd.contains("{files}") {
            new_step.cmd = substitute_files(&step.cmd, &matched);
        }
        out.push(new_step);
    }
    Ok(out)
}

fn build_globset(patterns: &[String]) -> Result<GlobSet> {
    let mut builder = GlobSetBuilder::new();
    for pat in patterns {
        builder.add(Glob::new(pat).with_context(|| format!("invalid glob `{pat}`"))?);
    }
    builder.build().context("building globset")
}

fn substitute_files(cmd: &str, paths: &[std::path::PathBuf]) -> String {
    let joined = paths
        .iter()
        .map(|p| shell_words::quote(&p.to_string_lossy()).into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    cmd.replace("{files}", &joined)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn step(name: &str, cmd: &str, if_changed: &[&str]) -> Step {
        Step {
            name: name.into(),
            cmd: cmd.into(),
            parallel: false,
            if_changed: if_changed.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn paths(ps: &[&str]) -> Vec<PathBuf> {
        ps.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn no_trigger_runs_all_steps() {
        let steps = vec![step("a", "echo a", &[]), step("b", "echo b", &["**/*.rs"])];
        let out = filter_and_template::<PathBuf>(&steps, None).unwrap();
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn no_trigger_substitutes_files_to_empty() {
        let steps = vec![step("a", "cargo test {files}", &[])];
        let out = filter_and_template::<PathBuf>(&steps, None).unwrap();
        assert_eq!(out[0].cmd, "cargo test ");
    }

    #[test]
    fn trigger_with_no_if_changed_passes_all_files() {
        let steps = vec![step("a", "lint {files}", &[])];
        let p = paths(&["src/foo.rs", "README.md"]);
        let out = filter_and_template(&steps, Some(&p)).unwrap();
        assert_eq!(out[0].cmd, "lint src/foo.rs README.md");
    }

    #[test]
    fn trigger_with_if_changed_passes_only_matches() {
        let steps = vec![step("a", "cargo test {files}", &["**/*.rs"])];
        let p = paths(&["src/foo.rs", "README.md"]);
        let out = filter_and_template(&steps, Some(&p)).unwrap();
        assert_eq!(out[0].cmd, "cargo test src/foo.rs");
    }

    #[test]
    fn step_with_no_matches_is_excluded() {
        let steps = vec![
            step("rust", "cargo test", &["**/*.rs"]),
            step("ts", "tsc", &["**/*.ts"]),
        ];
        let p = paths(&["README.md"]);
        let out = filter_and_template(&steps, Some(&p)).unwrap();
        assert!(out.is_empty(), "no globs match — both should be excluded");
    }

    #[test]
    fn step_without_if_changed_always_included_on_trigger() {
        let steps = vec![
            step("always", "true", &[]),
            step("rust", "cargo test", &["**/*.rs"]),
        ];
        let p = paths(&["README.md"]);
        let out = filter_and_template(&steps, Some(&p)).unwrap();
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].name, "always");
    }

    #[test]
    fn paths_with_spaces_get_quoted() {
        let steps = vec![step("a", "ls {files}", &[])];
        let p = paths(&["a b.rs"]);
        let out = filter_and_template(&steps, Some(&p)).unwrap();
        // shell-words quotes with single quotes; sufficient to verify the
        // arg is preserved as one token.
        assert!(
            shell_words::split(&out[0].cmd)
                .unwrap()
                .contains(&"a b.rs".to_string())
        );
    }

    #[test]
    fn invalid_glob_returns_error() {
        let steps = vec![step("a", "true", &["[invalid"])];
        let p = paths(&["x.rs"]);
        let err = filter_and_template(&steps, Some(&p)).unwrap_err();
        assert!(err.to_string().contains("step `a`"));
    }

    #[test]
    fn original_step_unchanged_when_no_files_template() {
        let steps = vec![step("a", "cargo check", &["**/*.rs"])];
        let p = paths(&["src/foo.rs"]);
        let out = filter_and_template(&steps, Some(&p)).unwrap();
        assert_eq!(out[0].cmd, "cargo check");
    }
}

#[cfg(test)]
mod proptests {
    use super::*;
    use proptest::prelude::*;
    use std::path::PathBuf;

    /// Path-like strings covering the characters that trip up naive shell
    /// joining: spaces, quotes, `$`, backslashes, tabs and newlines.
    fn hostile_path() -> impl Strategy<Value = String> {
        "[a-zA-Z0-9_./ '\"$\\\\!*?\\t\\n-]{1,24}"
    }

    /// Paths with a recognisable extension so a glob can partition them.
    fn ext_path() -> impl Strategy<Value = String> {
        "[a-z][a-z/]{0,8}\\.(rs|md|toml)"
    }

    fn step(name: &str, cmd: &str, if_changed: &[&str]) -> Step {
        Step {
            name: name.into(),
            cmd: cmd.into(),
            parallel: false,
            if_changed: if_changed.iter().map(|s| (*s).into()).collect(),
        }
    }

    fn arb_step() -> impl Strategy<Value = Step> {
        (
            "[a-z]{1,8}",
            prop::bool::ANY,
            prop::collection::vec(
                prop::sample::select(vec!["*.rs", "*.md", "src/**", "*.toml"]),
                0..3,
            ),
        )
            .prop_map(|(name, templated, globs)| {
                let cmd = if templated { "run {files}" } else { "run" };
                step(&name, cmd, &globs)
            })
    }

    proptest! {
        /// Whatever the trigger, the output is a subsequence of the input
        /// (order preserved, nothing invented) and steps without `if_changed`
        /// always survive.
        #[test]
        fn output_is_an_ordered_subsequence(
            steps in prop::collection::vec(arb_step(), 0..8),
            trigger in prop::option::of(prop::collection::vec(ext_path(), 0..6)),
        ) {
            let trigger_paths: Option<Vec<PathBuf>> =
                trigger.map(|ps| ps.into_iter().map(PathBuf::from).collect());
            let out = filter_and_template(&steps, trigger_paths.as_deref()).unwrap();

            let mut cursor = 0;
            for kept in &out {
                let pos = steps[cursor..]
                    .iter()
                    .position(|s| s.name == kept.name)
                    .expect("output step must come from input, in order");
                cursor += pos + 1;
            }
            for s in steps.iter().filter(|s| s.if_changed.is_empty()) {
                prop_assert!(out.iter().any(|o| o.name == s.name));
            }
            let untemplated = out.iter().all(|o| !o.cmd.contains("{files}"));
            prop_assert!(untemplated);
        }

        /// The initial run keeps every step and blanks the template.
        #[test]
        fn no_trigger_keeps_everything(steps in prop::collection::vec(arb_step(), 0..8)) {
            let out = filter_and_template::<PathBuf>(&steps, None).unwrap();
            prop_assert_eq!(out.len(), steps.len());
            for (i, o) in out.iter().enumerate() {
                let expected = steps[i].cmd.replace("{files}", "");
                prop_assert_eq!(&o.cmd, &expected);
            }
        }

        /// `{files}` substitution must round-trip through the same tokenizer
        /// the step runner uses, one argv entry per path, however hostile
        /// the path is.
        #[test]
        fn substituted_files_roundtrip_through_shell_split(
            paths in prop::collection::vec(hostile_path(), 0..6),
        ) {
            let steps = vec![step("a", "run {files}", &[])];
            let bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
            let out = filter_and_template(&steps, Some(&bufs)).unwrap();
            let argv = shell_words::split(&out[0].cmd).unwrap();
            prop_assert_eq!(&argv[0], "run");
            prop_assert_eq!(&argv[1..], &paths[..]);
        }

        /// With `if_changed = ["*.rs"]`, the step runs iff some path ends in
        /// `.rs`, and `{files}` receives exactly those paths in order.
        #[test]
        fn glob_partitions_trigger_paths(paths in prop::collection::vec(ext_path(), 0..8)) {
            let steps = vec![step("rs", "run {files}", &["*.rs"])];
            let bufs: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
            let out = filter_and_template(&steps, Some(&bufs)).unwrap();

            let matched: Vec<&str> = paths
                .iter()
                .map(String::as_str)
                .filter(|p| p.ends_with(".rs"))
                .collect();
            if matched.is_empty() {
                prop_assert!(out.is_empty());
            } else {
                let argv = shell_words::split(&out[0].cmd).unwrap();
                prop_assert_eq!(&argv[1..], &matched[..]);
            }
        }
    }
}
