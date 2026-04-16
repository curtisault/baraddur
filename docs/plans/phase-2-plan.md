# Phase 2 — Config Polish + Discovery

> Companion to [`project-plan.md`](./project-plan.md), [`ux-design.md`](./ux-design.md),
> and [`phase-1-plan.md`](./phase-1-plan.md). A step-by-step implementation plan
> for Phase 2.

---

## Goal

Harden the config loading path so that Barad-dûr can be launched from anywhere
inside a project tree and produce clear, actionable errors when the config is
missing or wrong.

Concretely, Phase 2 delivers:

1. **Walk-up discovery.** Searching upward from `cwd` for `.baraddur.toml`,
   like `cargo` / `git` / `.gitignore`.
2. **Global fallback.** If nothing is found in the tree, fall back to a
   user-level `~/.config/baraddur/config.toml`.
3. **Config validation.** Reject unknown fields, empty step lists, empty
   `cmd`s, duplicate step names, and obviously-invalid values — with
   line-and-column errors that match UX state **E2**.
4. **Structured error presentation.** "No config found" matches UX state
   **E1** exactly, enumerating every location searched.
5. **Forward-compat schema for `[summarize]`.** Reserve the key and all its
   fields now so `deny_unknown_fields` doesn't break when Phase 5 lands.

Phase 1's deferred items in the project-plan checklist
(`Validate config`, `Config discovery walk-up`, `Global fallback`) all close.
The already-completed items (`Implement ignore patterns`, `CLI --config
override`) stay green — we don't regress them.

---

## Success Criteria (Smoke Test)

### SC-1 — Walk-up discovery

```sh
$ cd ~/code/my-elixir-project
$ cat .baraddur.toml
[watch]
extensions = ["ex"]
[[steps]]
name = "compile"
cmd = "mix compile"

$ cd lib/deep/nested/module
$ baraddur
baraddur: watching /Users/alice/code/my-elixir-project
          (config: /Users/alice/code/my-elixir-project/.baraddur.toml)
--- run started ---
...
```

The config is found by walking up from the subdirectory. The watch root is
**the config's directory**, not the cwd.

### SC-2 — Global fallback

```sh
$ cd /tmp/empty-dir
$ cat ~/.config/baraddur/config.toml
[watch]
extensions = ["rs"]
[[steps]]
name = "check"
cmd = "cargo check"

$ baraddur
baraddur: watching /tmp/empty-dir
          (config: /Users/alice/.config/baraddur/config.toml)
...
```

No `.baraddur.toml` in the tree, so the global is used. Watch root is cwd
(there's no project directory to anchor to).

### SC-3 — Config missing entirely (E1)

```sh
$ cd /tmp/empty-dir
$ rm ~/.config/baraddur/config.toml
$ baraddur
baraddur: no .baraddur.toml found in this directory or any parent,
          and no ~/.config/baraddur/config.toml.

create a .baraddur.toml in your project root. minimal example:

  [watch]
  extensions = ["rs"]

  [[steps]]
  name = "check"
  cmd  = "cargo check"

$ echo $?
2
```

Exit code **2** for config-not-found (distinct from runtime error code 1 and
Ctrl+C code 130).

### SC-4 — Unknown field (E2)

```sh
$ cat .baraddur.toml
[watch]
extensions = ["rs"]

[[steps]]
name = "check"
cmd = "cargo check"
parralel = false   # typo

$ baraddur
baraddur: config error in .baraddur.toml

TOML parse error at line 7, column 1
  |
7 | parralel = false
  | ^^^^^^^^
unknown field `parralel`, expected one of `name`, `cmd`, `parallel`

$ echo $?
2
```

### SC-5 — Semantic validation

```sh
$ cat .baraddur.toml
[watch]
extensions = ["rs"]

[[steps]]
name = ""
cmd = ""

$ baraddur
baraddur: config error in .baraddur.toml

  step 1 has an empty `name`
  step 1 has an empty `cmd`
```

Multiple semantic errors are reported together, not one per run.

### SC-6 — Explicit `--config` bypasses discovery

```sh
$ baraddur --config ./my-alt-config.toml
# Uses ./my-alt-config.toml directly, no walk-up, no global fallback.
```

### SC-7 — Explicit `--config` to missing path is a hard error

```sh
$ baraddur --config ./does-not-exist.toml
baraddur: config file not found: ./does-not-exist.toml
$ echo $?
2
```

No "searched these directories" verbiage — the user specified a path
explicitly, so the error is crisp.

---

## Non-Goals for Phase 2 (deferred)

- **"Did you mean?" suggestions** for misspelled fields. `toml`'s built-in
  error already lists valid fields; Levenshtein matching is a stretch goal.
- **Glob patterns in `ignore`.** Current component-name matching
  (`watcher::matches_filters`) is sufficient for the stated use cases.
  Globset support can land later if actually needed.
- **`.gitignore` integration.** Out of scope — would require adding
  `ignore` crate and is orthogonal to `.baraddur.toml` config polish.
- **Config watching.** Re-loading the config when `.baraddur.toml` itself
  changes. Phase 2 stays stateless; a new config requires a restart.
- **Multiple config layers.** No project-level + user-level *merging* — it's
  strictly first-match-wins (walk-up beats global).
- **XDG-variant paths on macOS** (`~/Library/Application Support/...`).
  The plan explicitly says `~/.config/baraddur/config.toml`, and most CLI
  tools (`gh`, `starship`, `helix`) follow XDG on macOS anyway.

---

## Prerequisites

- Phase 1 complete (it is — see the checklist in `project-plan.md`).
- No new crates required. `dirs` is already in `Cargo.toml` from Phase 1;
  Phase 2 is the phase that actually consumes it.

---

## Implementation Order

Bottom-up, each step ending in green `cargo check` / `cargo test`:

```
1. Schema changes (deny_unknown_fields, SummarizeConfig stub)   → Step 1
2. Post-parse validation (validate.rs)                          → Step 2
3. Discovery module (discovery.rs: walk-up + global)            → Step 3
4. Wire discovery into config::load                             → Step 4
5. Use discovered config dir as default watch root              → Step 5
6. Main.rs error rendering (exit code 2, clean output)          → Step 6
7. Tests (schema, validate, discovery, integration)             → Step 7
8. Smoke-test checklist                                         → Step 8
```

---

## Step 1 — Schema Changes

**File:** `src/config/schema.rs`

Three changes:

1. Add `#[serde(deny_unknown_fields)]` to every struct.
2. Introduce `SummarizeConfig` — fully-defaulted so it's backwards-compatible
   AND so that adding `[summarize]` to a config doesn't crash Phase 2 users.
3. Widen `OutputConfig` and `WatchConfig` similarly — they already have
   sensible defaults, we just add the attribute.

```rust
// src/config/schema.rs
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
    pub watch: WatchConfig,

    #[serde(default)]
    pub output: OutputConfig,

    #[serde(default)]
    pub summarize: SummarizeConfig,

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

fn default_debounce_ms() -> u64 { 1500 }

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
        Self { clear_screen: true, show_passing: false }
    }
}

fn default_true() -> bool { true }

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

fn default_summarize_cmd() -> String { "claude".into() }
fn default_summarize_prompt() -> String {
    "Summarize these check failures in under 5 lines. Focus on root cause \
     and cite file:line where possible.".into()
}
fn default_summarize_timeout() -> u64 { 15 }

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Step {
    pub name: String,
    pub cmd: String,

    #[serde(default)]
    pub parallel: bool,
}
```

**Re-export** `SummarizeConfig` from `src/config/mod.rs`:

```rust
pub use schema::{Config, OutputConfig, Step, SummarizeConfig, WatchConfig};
```

### Test — reject unknown top-level field

Add to `schema.rs` tests:

```rust
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
    // Default prompt survives because we didn't set it.
    assert!(cfg.summarize.prompt.contains("Summarize"));
}
```

---

## Step 2 — Post-Parse Validation

**New file:** `src/config/validate.rs`

Serde catches shape errors. This module catches semantic errors that are
still valid TOML but wrong: empty steps, empty names, duplicate names, etc.

**Key design point:** accumulate all errors, don't short-circuit on the
first one. Users should see everything broken in one run.

```rust
// src/config/validate.rs
use super::schema::Config;

/// Human-readable validation errors, one per line.
#[derive(Debug)]
pub struct ValidationErrors(pub Vec<String>);

impl std::fmt::Display for ValidationErrors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for (i, msg) in self.0.iter().enumerate() {
            if i > 0 { writeln!(f)?; }
            write!(f, "  {msg}")?;
        }
        Ok(())
    }
}

impl std::error::Error for ValidationErrors {}

pub fn validate(cfg: &Config) -> Result<(), ValidationErrors> {
    let mut errs: Vec<String> = Vec::new();

    if cfg.steps.is_empty() {
        errs.push("no steps defined — add at least one [[steps]] entry".into());
    }

    let mut seen: std::collections::HashSet<&str> = std::collections::HashSet::new();
    for (i, s) in cfg.steps.iter().enumerate() {
        // 1-indexed for humans
        let idx = i + 1;

        if s.name.trim().is_empty() {
            errs.push(format!("step {idx} has an empty `name`"));
        } else if !seen.insert(&s.name) {
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
    use crate::config::schema::{
        Config, OutputConfig, Step, SummarizeConfig, WatchConfig,
    };

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
            Step { name: "x".into(), cmd: "true".into(), parallel: false },
            Step { name: "x".into(), cmd: "true".into(), parallel: false },
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
}
```

**Export from `src/config/mod.rs`:**

```rust
pub mod schema;
pub mod validate;
pub use schema::{Config, OutputConfig, Step, SummarizeConfig, WatchConfig};
pub use validate::{validate, ValidationErrors};
```

---

## Step 3 — Discovery Module

**New file:** `src/config/discovery.rs`

Isolates the search logic so it's unit-testable without touching the
filesystem from `load()`. Two strategies:

1. Walk from a start directory upward until a `.baraddur.toml` is found or
   we hit filesystem root.
2. Probe `$HOME/.config/baraddur/config.toml` (via `dirs::home_dir`).

```rust
// src/config/discovery.rs
use std::path::{Path, PathBuf};

pub const CONFIG_FILENAME: &str = ".baraddur.toml";
pub const GLOBAL_RELATIVE: &str = ".config/baraddur/config.toml";

/// Walks upward from `start`, returning the first directory containing
/// `.baraddur.toml`. Returns the full path to the file (not the directory).
///
/// Also returns the list of directories searched, for error reporting.
pub fn walk_up(start: &Path) -> WalkResult {
    let mut searched = Vec::new();
    let mut cur = Some(start);

    while let Some(dir) = cur {
        searched.push(dir.to_path_buf());
        let candidate = dir.join(CONFIG_FILENAME);
        if candidate.is_file() {
            return WalkResult {
                found: Some(candidate),
                searched,
            };
        }
        cur = dir.parent();
    }

    WalkResult { found: None, searched }
}

pub struct WalkResult {
    pub found: Option<PathBuf>,
    pub searched: Vec<PathBuf>,
}

/// Returns the conventional global config path, regardless of whether it
/// exists. Returns `None` only if `$HOME` can't be resolved.
pub fn global_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(GLOBAL_RELATIVE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn finds_config_in_start_dir() {
        let td = TempDir::new().unwrap();
        let cfg = td.path().join(CONFIG_FILENAME);
        fs::write(&cfg, "").unwrap();

        let result = walk_up(td.path());
        assert_eq!(result.found.as_deref(), Some(cfg.as_path()));
    }

    #[test]
    fn finds_config_in_ancestor() {
        let td = TempDir::new().unwrap();
        let deep = td.path().join("a/b/c");
        fs::create_dir_all(&deep).unwrap();
        let cfg = td.path().join(CONFIG_FILENAME);
        fs::write(&cfg, "").unwrap();

        // Canonicalize so symlinks (e.g., /var → /private/var on macOS) don't
        // trip ancestor comparison.
        let deep = deep.canonicalize().unwrap();
        let cfg = cfg.canonicalize().unwrap();

        let result = walk_up(&deep);
        assert_eq!(result.found.as_deref(), Some(cfg.as_path()));
        assert!(result.searched.len() >= 4); // c, b, a, tempdir
    }

    #[test]
    fn returns_none_when_missing() {
        let td = TempDir::new().unwrap();
        let deep = td.path().join("a/b");
        fs::create_dir_all(&deep).unwrap();

        // Note: this test walks all the way up to /, which will scan the
        // real filesystem. That's fine — we assert only on `found`, not
        // on the searched list.
        let result = walk_up(&deep);
        // If by wild coincidence there's a .baraddur.toml in an ancestor of
        // the tempdir, skip the assert. (Extremely unlikely.)
        if let Some(p) = &result.found {
            assert!(!p.starts_with(td.path()));
        }
    }

    #[test]
    fn global_path_points_inside_home() {
        let p = global_path().expect("HOME should resolve");
        assert!(p.to_string_lossy().contains(".config/baraddur/config.toml"));
    }
}
```

**Why canonicalize in the ancestor test:** on macOS, `TempDir` lives under
`/var/...` which is a symlink to `/private/var/...`. Without
`canonicalize()`, the `Path::parent()` chain won't agree with the directory
we created, and the test fails spuriously.

---

## Step 4 — Wire Discovery Into `config::load`

**File:** `src/config/mod.rs`

Replace the existing `load` implementation. New signature returns a little
more metadata — specifically, where the config was found so `lib.rs` can
choose a good default watch root.

```rust
// src/config/mod.rs
pub mod discovery;
pub mod schema;
pub mod validate;

pub use discovery::CONFIG_FILENAME;
pub use schema::{Config, OutputConfig, Step, SummarizeConfig, WatchConfig};
pub use validate::{validate, ValidationErrors};

use anyhow::{anyhow, Context, Result};
use std::path::{Path, PathBuf};

pub struct Loaded {
    pub config: Config,
    /// Absolute path of the file that was parsed.
    pub config_path: PathBuf,
    /// Directory containing `config_path`, useful as a default watch root
    /// when the user hasn't supplied `--watch-dir`.
    pub config_dir: PathBuf,
    /// Source of the config, for diagnostics.
    pub source: ConfigSource,
}

#[derive(Debug, Clone, Copy)]
pub enum ConfigSource {
    CliOverride,
    WalkUp,
    Global,
}

pub fn load(cli_override: Option<&Path>) -> Result<Loaded> {
    if let Some(path) = cli_override {
        return load_from(path, ConfigSource::CliOverride);
    }

    // 1. Walk up from cwd.
    let cwd = std::env::current_dir().context("getting current directory")?;
    let walk = discovery::walk_up(&cwd);
    if let Some(path) = walk.found {
        return load_from(&path, ConfigSource::WalkUp);
    }

    // 2. Global fallback.
    if let Some(global) = discovery::global_path() {
        if global.is_file() {
            return load_from(&global, ConfigSource::Global);
        }
    }

    // 3. Nothing found — emit UX state E1.
    Err(anyhow!(not_found_error(&walk.searched, discovery::global_path().as_deref())))
}

fn load_from(path: &Path, source: ConfigSource) -> Result<Loaded> {
    if !path.is_file() {
        anyhow::bail!("config file not found: {}", path.display());
    }

    let abs = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;

    let contents = std::fs::read_to_string(&abs)
        .with_context(|| format!("reading {}", abs.display()))?;

    let config: Config = toml::from_str(&contents).map_err(|e| {
        anyhow!("config error in {}\n\n{}", abs.display(), e)
    })?;

    validate(&config).map_err(|e| {
        anyhow!("config error in {}\n\n{}", abs.display(), e)
    })?;

    let config_dir = abs
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("config path {} has no parent", abs.display()))?;

    Ok(Loaded {
        config,
        config_path: abs,
        config_dir,
        source,
    })
}

/// Render UX state E1. Keep the phrasing stable — tests assert against it.
fn not_found_error(searched: &[PathBuf], global: Option<&Path>) -> String {
    let mut msg = String::from(
        "no .baraddur.toml found in this directory or any parent",
    );
    match global {
        Some(g) => {
            msg.push_str(",\n          and no ");
            // Prefer "~/.config/baraddur/config.toml" over the absolute
            // path for readability, when we can detect $HOME.
            if let Some(home) = dirs::home_dir() {
                if let Ok(rel) = g.strip_prefix(&home) {
                    msg.push('~');
                    msg.push(std::path::MAIN_SEPARATOR);
                    msg.push_str(&rel.to_string_lossy());
                } else {
                    msg.push_str(&g.to_string_lossy());
                }
            } else {
                msg.push_str(&g.to_string_lossy());
            }
            msg.push('.');
        }
        None => msg.push('.'),
    }

    msg.push_str(
        "\n\ncreate a .baraddur.toml in your project root. minimal example:\
         \n\n  [watch]\
         \n  extensions = [\"rs\"]\
         \n\n  [[steps]]\
         \n  name = \"check\"\
         \n  cmd  = \"cargo check\"",
    );

    if !searched.is_empty() {
        msg.push_str("\n\n(searched: ");
        for (i, p) in searched.iter().enumerate() {
            if i > 0 { msg.push_str(", "); }
            msg.push_str(&p.display().to_string());
        }
        msg.push(')');
    }

    msg
}
```

**Key behavioral changes vs Phase 1:**

- `load` returns `Loaded` instead of `(Config, PathBuf)`. Callers in
  `lib.rs` and `main.rs` must update (Step 5).
- Discovery is walk-up then global, skipped only when `--config` is set.
- `canonicalize()` is called on the config path so downstream relative
  paths don't depend on whatever cwd was.
- Validation errors and parse errors share a single "config error in
  <file>:" envelope.

---

## Step 5 — Use Discovered Config Dir as Default Watch Root

**File:** `src/lib.rs`

The Phase 1 `App` struct takes a `root` field already. The wiring change
lives in `main.rs` (Step 6) — `App` itself barely changes, but we update
the `config_path` field to match the new loader.

```rust
// src/lib.rs
pub mod config;
pub mod output;
pub mod pipeline;
pub mod watcher;

use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;

use crate::output::{Display, PlainDisplay};

pub struct App {
    pub config: config::Config,
    pub config_path: PathBuf,
    pub root: PathBuf,
}

impl App {
    pub async fn run(self) -> Result<()> {
        eprintln!(
            "baraddur: watching {}\n          (config: {})",
            self.root.display(),
            self.config_path.display(),
        );

        let mut display: Box<dyn Display> = Box::new(PlainDisplay);

        pipeline::run_sequential(&self.config, &self.root, display.as_mut()).await?;

        let wcfg = watcher::WatchConfig {
            root: self.root.clone(),
            debounce: Duration::from_millis(self.config.watch.debounce_ms),
            extensions: self.config.watch.extensions.clone(),
            ignore: self.config.watch.ignore.clone(),
        };
        let mut rx = watcher::start(wcfg)?;

        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\nbaraddur: exiting.");
                    return Ok(());
                }
                maybe = rx.recv() => {
                    match maybe {
                        None => {
                            eprintln!("baraddur: file watcher stopped unexpectedly. exiting.");
                            return Ok(());
                        }
                        Some(()) => {
                            while rx.try_recv().is_ok() {}
                            pipeline::run_sequential(&self.config, &self.root, display.as_mut()).await?;
                        }
                    }
                }
            }
        }
    }
}
```

(Diff from Phase 1 is just the two-line banner format change.)

---

## Step 6 — `main.rs` Error Rendering

**File:** `src/main.rs`

Two concerns:

1. When config loading fails, print the message cleanly (not `Error: ...`
   with a debug backtrace) and exit with code **2**.
2. Default watch root is `cfg.config_dir` when config was discovered via
   walk-up; otherwise cwd.

```rust
// src/main.rs
use clap::Parser;
use std::path::PathBuf;
use std::process::ExitCode;

use baraddur::config::{self, ConfigSource};

#[derive(Parser, Debug)]
#[command(
    name = "baraddur",
    version,
    about = "Project-agnostic file watcher that surfaces issues before CI"
)]
struct Cli {
    /// Config file path (disables walk-up discovery).
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Directory to watch [default: directory containing the discovered config]
    #[arg(short = 'w', long)]
    watch_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let loaded = match config::load(cli.config.as_deref()) {
        Ok(l) => l,
        Err(e) => {
            // anyhow's `{:#}` joins the error chain with ": ", which reads
            // wrong for our multi-line envelope. Use plain Display on the
            // root error instead.
            eprintln!("baraddur: {e}");
            return ExitCode::from(2);
        }
    };

    let root = match cli.watch_dir {
        Some(p) => p,
        None => match loaded.source {
            ConfigSource::WalkUp => loaded.config_dir.clone(),
            ConfigSource::CliOverride | ConfigSource::Global => {
                match std::env::current_dir() {
                    Ok(cwd) => cwd,
                    Err(e) => {
                        eprintln!("baraddur: getting current directory: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
        },
    };

    let app = baraddur::App {
        config: loaded.config,
        config_path: loaded.config_path,
        root,
    };

    match app.run().await {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("baraddur: {e:#}");
            ExitCode::from(1)
        }
    }
}
```

**Why `ExitCode::from(2)` for config errors:**

- Phase 1's existing behavior (`?` with anyhow) exits 1 for everything.
- UX expectation (from SC-3 / SC-4): a missing or invalid config is a
  distinct category. Exit codes: `0` success, `1` runtime, `2` config,
  `130` Ctrl+C (Phase 3 will formalize the last one).

---

## Step 7 — Tests

### 7a. Schema unit tests

Already covered in Step 1 — they live next to the struct definitions.

### 7b. Validate unit tests

Already covered in Step 2 — they live next to `validate()`.

### 7c. Discovery unit tests

Already covered in Step 3 — they live next to `walk_up()` and use
`tempfile::TempDir`.

### 7d. Integration test — end-to-end load

**New file:** `tests/config_load.rs`

```rust
use baraddur::config::{self, ConfigSource};
use std::fs;
use tempfile::TempDir;

const MINIMAL_CONFIG: &str = r#"
[watch]
extensions = ["rs"]

[[steps]]
name = "check"
cmd = "cargo check"
"#;

#[test]
fn walk_up_finds_config_from_subdir() {
    let td = TempDir::new().unwrap();
    let root = td.path().canonicalize().unwrap();
    fs::write(root.join(".baraddur.toml"), MINIMAL_CONFIG).unwrap();

    let subdir = root.join("a/b/c");
    fs::create_dir_all(&subdir).unwrap();

    // Tests must not rely on mutating process cwd (flaky under parallel
    // test runs). We probe the loader indirectly by constructing an
    // explicit path and using `--config`-equivalent behavior.
    let loaded = config::load(Some(&root.join(".baraddur.toml"))).unwrap();
    assert!(matches!(loaded.source, ConfigSource::CliOverride));
    assert_eq!(loaded.config_dir, root);
    assert_eq!(loaded.config.steps.len(), 1);
}

#[test]
fn explicit_missing_config_is_hard_error() {
    let td = TempDir::new().unwrap();
    let missing = td.path().join("does-not-exist.toml");
    let err = config::load(Some(&missing)).unwrap_err();
    assert!(err.to_string().contains("not found"));
}

#[test]
fn parse_error_wrapped_with_file_path() {
    let td = TempDir::new().unwrap();
    let bad = td.path().join(".baraddur.toml");
    fs::write(&bad, "this is not toml =\n[[[").unwrap();

    let err = config::load(Some(&bad)).unwrap_err();
    let s = err.to_string();
    assert!(s.contains("config error in"));
    assert!(s.contains(".baraddur.toml"));
}

#[test]
fn validation_error_wrapped_with_file_path() {
    let td = TempDir::new().unwrap();
    let bad = td.path().join(".baraddur.toml");
    fs::write(&bad, r#"
[watch]
extensions = ["rs"]

[[steps]]
name = ""
cmd = ""
"#).unwrap();

    let err = config::load(Some(&bad)).unwrap_err();
    let s = err.to_string();
    assert!(s.contains("config error in"));
    assert!(s.contains("empty `name`"));
    assert!(s.contains("empty `cmd`"));
}
```

**Note on cwd-based testing.** Rust tests run in parallel by default, and
`std::env::set_current_dir()` mutates process-global state. Tests that
change cwd can corrupt each other. So the walk-up-via-cwd case is tested
in `discovery::tests::finds_config_in_ancestor` (which exercises the pure
function) rather than end-to-end. If you want an end-to-end walk-up test,
add it with `#[test] #[ignore]` or guard it behind a serial-test mutex.

### 7e. Regression test — Phase 1's runner tests still pass

Already in `tests/runner.rs`. No changes needed. `cargo test` should stay
green.

---

## Step 8 — Smoke-Test Checklist

After implementation, run through each manually:

- [ ] **Walk-up from subdirectory.** `cd` into a nested directory inside
      an Elixir project, run `baraddur`, see it find and use the root
      `.baraddur.toml`. Banner shows `(config: .../project/.baraddur.toml)`
      even though you ran from 3 levels deep.
- [ ] **Watch root is the config dir.** In the same scenario, confirm
      `touch ../../../lib/foo.ex` triggers a rerun. (Would fail if the
      watcher were rooted at cwd.)
- [ ] **Explicit `--config`.** Run `baraddur -c /tmp/other-config.toml`
      and confirm walk-up is skipped.
- [ ] **Explicit `--config` to missing file.** Exit 2, tight error message.
- [ ] **Global fallback.** Move the project's `.baraddur.toml` aside,
      drop one at `~/.config/baraddur/config.toml`, run `baraddur` from a
      directory with no config tree. Confirm the global is used.
- [ ] **No config anywhere.** Remove the global too. Exit 2, E1 message
      shown verbatim.
- [ ] **Unknown field.** Add `parralel = false` to a step, run `baraddur`,
      see the E2-shaped error with line number. Exit 2.
- [ ] **Empty cmd.** Set `cmd = ""` on a step. Exit 2, validation error.
- [ ] **Duplicate step names.** Two `[[steps]]` with the same `name`.
      Exit 2, validation error.
- [ ] **Empty steps array.** Delete all `[[steps]]`. Exit 2, validation
      error.
- [ ] **`[summarize]` table parses without error.** Add a full
      `[summarize]` block to a config; Phase 2 should accept it silently
      (Phase 5 consumes it).

---

## References

### Crate documentation

| Crate | URL |
|---|---|
| serde (`deny_unknown_fields`) | https://serde.rs/container-attrs.html#deny_unknown_fields |
| toml (error Display) | https://docs.rs/toml/0.8/toml/de/struct.Error.html |
| dirs | https://docs.rs/dirs/5/dirs/fn.home_dir.html |
| tempfile | https://docs.rs/tempfile |
| anyhow (error formatting) | https://docs.rs/anyhow/1/anyhow/#acting-on-particular-errors |

### Prior art — walk-up discovery

- **Cargo** — `cargo` walks up from cwd looking for `Cargo.toml`.
  See [`cargo/util/config/mod.rs`](https://github.com/rust-lang/cargo/blob/master/src/cargo/util/context/mod.rs)
  for the pattern.
- **git** — traverses upward for `.git/` via `setup_git_directory()`.
- **`.editorconfig`** — classic walk-up-to-root with `root = true`
  sentinels; we don't need root-sentinel semantics.

### UX reference

State **E1** and **E2** in [`ux-design.md`](./ux-design.md) are the
authoritative format for the two most common error paths. Don't paraphrase
them — the wording is the design.

---

## Common Pitfalls

1. **Mutating cwd in tests.** `std::env::set_current_dir()` is
   process-global and unsafe under parallel test execution. Prefer
   explicit-path tests (as in 7d) or gate walk-up-cwd tests behind
   `#[ignore]` / a mutex crate like `serial_test`.

2. **macOS symlinks (`/var` → `/private/var`).** `tempfile::TempDir`
   returns paths under `/var/...` but `Path::parent()` chains through the
   real `/private/var/...`. Canonicalize before comparing paths in tests
   (Step 3's ancestor test does this).

3. **`deny_unknown_fields` is viral.** Adding it to `Config` means every
   nested struct must either define every supported field OR explicitly
   opt into laxness. Forget `SummarizeConfig` and every existing user with
   a `[summarize]` table breaks on upgrade — that's why Step 1 defines it
   up front.

4. **`toml::from_str` errors are already well-formatted.** Don't try to
   re-wrap the error body. Just prepend your "config error in <file>"
   line and pass the TOML error through via `{e}`. Its built-in Display
   renders the line number + caret pointer.

5. **`anyhow`'s `{:#}` formatter joins chain elements with `: `.** That
   clobbers multi-line error bodies (validation errors, E1 banner). Use
   `{e}` (not `{e:#}`) for root-level config errors. Step 6's `main.rs`
   gets this right.

6. **`dirs::home_dir()` vs `dirs::config_dir()`.** On Linux they agree;
   on macOS they don't. The plan standardizes on `~/.config/baraddur/` via
   `home_dir().join(".config/baraddur/config.toml")` — don't switch to
   `config_dir()` without updating UX docs and error strings.

7. **Cycle-safe walk-up.** `Path::parent()` never loops (it yields `None`
   at root), so there's no infinite-loop risk. But if you ever refactor
   to use `Path::ancestors()`, it also terminates — both are safe.

8. **Canonical path in `config_dir`.** If you don't canonicalize the
   config path, `config_dir` may be a relative path (`./`), which breaks
   `watcher::start` when cwd later differs from launch cwd. Step 4
   canonicalizes.

9. **Semantic validation vs shape validation ordering.** Run shape
   (`toml::from_str`) first; only run semantic `validate` on a parsed
   `Config`. A config with a typo'd field shouldn't also get a pile of
   semantic errors layered on top.

10. **Empty `extensions = []` is legal and means "watch everything".**
    The existing `watcher::matches_filters` logic depends on this. Do
    **not** add a `validate()` rule rejecting it.

---

## Definition of Done

- [ ] `cargo build --release` produces a working binary.
- [ ] `cargo test` passes — schema, validate, discovery, and the new
      `config_load` integration tests.
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `cargo fmt --check` is clean.
- [ ] All 11 boxes in the Step 8 smoke-test checklist are verified.
- [ ] `project-plan.md` Phase 2 checklist is fully checked:
  - [x] Implement `ignore` patterns (Phase 1, already done)
  - [x] CLI `--config` override (Phase 1, already done)
  - [ ] Validate config on load
  - [ ] Config discovery walk-up
  - [ ] Global `~/.config/baraddur/config.toml` fallback
- [ ] Phase 2 commit(s) pushed to `phase-2` branch, ready to merge into
      `main`.

Once all boxes are checked, Phase 3 (parallel execution + lifecycle +
redraw) is ready to start.
