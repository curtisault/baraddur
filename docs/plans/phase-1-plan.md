# Phase 1 — Core Watcher + Sequential Pipeline

> Companion to [`project-plan.md`](./project-plan.md) and [`ux-design.md`](./ux-design.md).
> A step-by-step implementation plan for Phase 1.

---

## Goal

Deliver a minimal end-to-end Barad-dûr that:

1. Loads a `.baraddur.toml` from the current directory.
2. Runs its steps **once at startup**, sequentially, stopping at the first failure.
3. Starts a file watcher and re-runs the pipeline whenever a relevant file changes.
4. Prints step pass/fail and raw output on failure to the terminal.
5. Exits cleanly on Ctrl+C.

No colors, no screen clearing, no parallelism, no LLM summaries yet.

---

## Success Criteria (Smoke Test)

At the end of Phase 1, this should work in any Elixir project:

```toml
# .baraddur.toml
[watch]
extensions = ["ex", "exs", "heex"]
debounce_ms = 1000
ignore = ["_build", "deps", ".git"]

[[steps]]
name = "compile"
cmd = "mix compile"
```

```sh
$ cd my-elixir-project
$ baraddur
--- run started ---
▸ compile ...
▸ compile ✓  (0.8s)
--- run complete: 0 failed, 1 passed, 0.8s ---
# (edit a file to introduce a compile error)
--- run started ---
▸ compile ...
▸ compile ✗  (0.6s)
── compile output ──
** (CompileError) lib/foo.ex:12: undefined function bar/0
...
--- run complete: 1 failed, 0 passed, 0.6s ---
^C
baraddur: exiting.
```

---

## Non-Goals for Phase 1 (deferred to later phases)

- Parallel step execution (Phase 3)
- Screen clearing, colors, spinners, in-place updates (Phase 4)
- Mid-run cancel+restart on file change (Phase 3) — in Phase 1 we just ignore events during a run and coalesce them afterward
- LLM summaries (Phase 5)
- Walk-up config discovery and `~/.config/baraddur/config.toml` fallback (Phase 2)
- Ignore-path validation with helpful errors (Phase 2)
- Non-TTY mode detection (Phase 3)

Phase 1 scope is intentionally narrow so there is one working vertical slice to build on.

---

## Prerequisites

- **Rust ≥ 1.85** (edition 2024). Verify with `rustc --version`.
- **tokio** runtime familiarity.
- A local Elixir project (or any project with a checkable command) for smoke testing.

### Pre-flight: verify crate versions

The pinned versions in `project-plan.md` were chosen at design time and may be stale. Before coding, check [crates.io](https://crates.io) for the current major versions of:

- `notify-debouncer-mini` — **API-sensitive**, most important to verify
- `crossterm` (not needed until Phase 4, but good to know now)
- `toml`
- `thiserror`

If `notify-debouncer-mini` has a newer major version with a changed API, adapt the watcher module accordingly — the core idea (callback with batched events) should be stable even if method names shift.

---

## Implementation Order

Build bottom-up so each module compiles and is testable in isolation:

```
1. cargo new                       → Step 0
2. Cargo.toml                      → Step 1
3. module tree skeleton            → Step 2
4. config/schema.rs                → Step 3
5. config/mod.rs                   → Step 4
6. pipeline/step.rs                → Step 5
7. pipeline/runner.rs              → Step 6
8. pipeline/mod.rs (re-exports)    → Step 7
9. output/mod.rs + display.rs      → Step 8
10. watcher/mod.rs                 → Step 9
11. lib.rs (App struct, wiring)    → Step 10
12. main.rs (CLI entry)            → Step 11
13. smoke test                     → Step 12
```

Each step ends with a green `cargo check`.

---

## Step 0 — Scaffold

```sh
cargo new baraddur
cd baraddur
git add -A && git commit -m "cargo new"
```

Inspect the default layout — it creates `Cargo.toml`, `src/main.rs`, and `.gitignore`. We'll replace `main.rs` and add `src/lib.rs` plus the module subdirectories in Step 2.

---

## Step 1 — Cargo.toml

Replace the generated `Cargo.toml` with:

```toml
[package]
name = "baraddur"
version = "0.1.0"
edition = "2024"
description = "Project-agnostic file watcher that surfaces issues before CI"
license = "MIT"

[[bin]]
name = "baraddur"
path = "src/main.rs"

[lib]
name = "baraddur"
path = "src/lib.rs"

[dependencies]
# File watching
notify-debouncer-mini = "0.4"   # verify latest on crates.io before coding

# Async runtime
tokio = { version = "1", features = ["rt-multi-thread", "macros", "process", "sync", "signal", "time"] }

# CLI
clap = { version = "4", features = ["derive"] }

# Config
serde = { version = "1", features = ["derive"] }
toml = "0.8"

# Command parsing
shell-words = "1"

# Path helpers
dirs = "5"                       # for ~/.config/baraddur (lookups used in Phase 2, safe to pull now)

# Error handling
anyhow = "1"

[dev-dependencies]
tempfile = "3"

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

**Tokio features used in Phase 1:**
- `process` — `tokio::process::Command`
- `sync` — `tokio::sync::mpsc`
- `signal` — `tokio::signal::ctrl_c`
- `time` — `tokio::time::timeout` (used later; safe to include)
- `macros`, `rt-multi-thread` — `#[tokio::main]`

`thiserror`, `crossterm` intentionally omitted — not needed until later phases.

Run `cargo build` to confirm all dependencies resolve.

---

## Step 2 — Module Tree Skeleton

Create the directories and empty stub files:

```
src/
├── main.rs
├── lib.rs
├── config/
│   ├── mod.rs
│   └── schema.rs
├── watcher/
│   └── mod.rs
├── pipeline/
│   ├── mod.rs
│   ├── step.rs
│   └── runner.rs
└── output/
    ├── mod.rs
    └── display.rs
```

Stub each module with a single `// Phase 1 stub` comment. In `lib.rs`:

```rust
pub mod config;
pub mod output;
pub mod pipeline;
pub mod watcher;
```

`cargo check` should pass with warnings about unused modules — that's expected.

---

## Step 3 — `config/schema.rs`

Define the deserializable types. Keep defaults lenient; strict validation lives in Phase 2.

```rust
// src/config/schema.rs
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
```

**Notes:**
- The `parallel` field is parsed but ignored in Phase 1 (all steps run sequentially). Accepting it keeps configs forward-compatible.
- The `[summarize]` table is not in this struct yet; toml ignores unknown fields by default, so it won't cause errors if a user's config already has it.
- `#[serde(deny_unknown_fields)]` is deliberately omitted — Phase 2 adds it.

### Test

Add a module-level test to `schema.rs`:

```rust
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
}
```

---

## Step 4 — `config/mod.rs` (discovery + loading)

For Phase 1, keep discovery simple: look at `./.baraddur.toml` only. Walk-up and global fallback come in Phase 2.

```rust
// src/config/mod.rs
pub mod schema;
pub use schema::{Config, OutputConfig, Step, WatchConfig};

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const CONFIG_FILENAME: &str = ".baraddur.toml";

/// Loads the config either from an explicit CLI-provided path, or from
/// `./.baraddur.toml` in the current directory.
pub fn load(cli_override: Option<&Path>) -> Result<(Config, PathBuf)> {
    let path = match cli_override {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir()
            .context("getting current directory")?
            .join(CONFIG_FILENAME),
    };

    if !path.is_file() {
        anyhow::bail!(
            "no config file found at {}. Create a {} to get started.",
            path.display(),
            CONFIG_FILENAME
        );
    }

    let contents = std::fs::read_to_string(&path)
        .with_context(|| format!("reading {}", path.display()))?;

    let config: Config = toml::from_str(&contents)
        .with_context(|| format!("parsing {}", path.display()))?;

    Ok((config, path))
}
```

---

## Step 5 — `pipeline/step.rs`

Defines a single step execution. `tokio::process::Command::output()` reads stdout and stderr to completion and returns them bundled with the exit status — exactly what we want.

```rust
// src/pipeline/step.rs
use anyhow::{anyhow, Context, Result};
use std::path::Path;
use std::time::{Duration, Instant};
use tokio::process::Command;

use crate::config::Step;

#[derive(Debug, Clone)]
pub struct StepResult {
    pub name: String,
    pub success: bool,
    pub exit_code: Option<i32>,
    pub stdout: String,
    pub stderr: String,
    pub duration: Duration,
}

/// Runs a single step and returns its result. Does not return Err for a step
/// that exited non-zero — that is a failure `StepResult`, not an error.
/// Only returns Err for things like malformed `cmd` strings.
pub async fn run(step: &Step, cwd: &Path) -> Result<StepResult> {
    let parts = shell_words::split(&step.cmd)
        .with_context(|| format!("parsing cmd `{}` for step `{}`", step.cmd, step.name))?;

    let (program, args) = parts
        .split_first()
        .ok_or_else(|| anyhow!("empty cmd for step `{}`", step.name))?;

    let start = Instant::now();

    let output = Command::new(program)
        .args(args)
        .current_dir(cwd)
        .kill_on_drop(true)
        .output()
        .await;

    let duration = start.elapsed();

    let result = match output {
        Ok(out) => StepResult {
            name: step.name.clone(),
            success: out.status.success(),
            exit_code: out.status.code(),
            stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
            duration,
        },
        Err(e) => StepResult {
            name: step.name.clone(),
            success: false,
            exit_code: None,
            stdout: String::new(),
            stderr: format!("failed to launch `{program}`: {e}"),
            duration,
        },
    };

    Ok(result)
}
```

**Design notes:**
- `kill_on_drop(true)` ensures that if the future is dropped (e.g. runner cancels), the child process is killed — sets us up for Phase 3's cancel+restart.
- A command that *fails to launch* (binary not found) becomes a failing `StepResult`, not an `Err`. This keeps the runner's control flow simple.
- `String::from_utf8_lossy` is fine for display — we never need to preserve invalid bytes.

---

## Step 6 — `pipeline/runner.rs` (sequential)

```rust
// src/pipeline/runner.rs
use anyhow::Result;
use std::path::Path;

use crate::config::Config;
use crate::output::Display;
use super::step::{self, StepResult};

/// Runs all steps sequentially. Stops at the first failure, returning
/// everything run so far (including the failure).
pub async fn run_sequential(
    config: &Config,
    cwd: &Path,
    display: &mut dyn Display,
) -> Result<Vec<StepResult>> {
    display.run_started();

    let mut results = Vec::with_capacity(config.steps.len());

    for step_cfg in &config.steps {
        display.step_started(&step_cfg.name);
        let result = step::run(step_cfg, cwd).await?;
        let success = result.success;
        display.step_finished(&result);
        results.push(result);
        if !success {
            break;
        }
    }

    display.run_finished(&results);
    Ok(results)
}
```

Phase 1 ignores `step_cfg.parallel` entirely — steps run in declared order. The `parallel` grouping logic lands in Phase 3.

---

## Step 7 — `pipeline/mod.rs`

Just re-exports for ergonomics:

```rust
// src/pipeline/mod.rs
pub mod runner;
pub mod step;

pub use runner::run_sequential;
pub use step::{run as run_step, StepResult};
```

---

## Step 8 — `output/mod.rs` + `output/display.rs`

A trait-based output abstraction so Phase 4 can swap in a fancier TTY implementation without touching the runner.

```rust
// src/output/mod.rs
pub mod display;
pub use display::PlainDisplay;

use crate::pipeline::StepResult;

/// Sink for pipeline events. Phase 1 has one implementation (`PlainDisplay`).
/// Phase 4 will add a `TtyDisplay` with colors, spinners, and redraw.
pub trait Display: Send {
    fn run_started(&mut self);
    fn step_started(&mut self, name: &str);
    fn step_finished(&mut self, result: &StepResult);
    fn run_finished(&mut self, results: &[StepResult]);
}
```

```rust
// src/output/display.rs
use std::io::Write;

use super::Display;
use crate::pipeline::StepResult;

/// Phase 1 "plain" display — prints line-by-line, no colors, no clearing.
pub struct PlainDisplay;

impl Display for PlainDisplay {
    fn run_started(&mut self) {
        println!("--- run started ---");
    }

    fn step_started(&mut self, name: &str) {
        println!("▸ {name} ...");
    }

    fn step_finished(&mut self, result: &StepResult) {
        let status = if result.success { "✓" } else { "✗" };
        println!(
            "▸ {} {}  ({:.1}s)",
            result.name,
            status,
            result.duration.as_secs_f64()
        );

        if !result.success {
            println!("── {} output ──", result.name);
            if !result.stdout.is_empty() {
                print!("{}", result.stdout);
                if !result.stdout.ends_with('\n') {
                    println!();
                }
            }
            if !result.stderr.is_empty() {
                print!("{}", result.stderr);
                if !result.stderr.ends_with('\n') {
                    println!();
                }
            }
        }
    }

    fn run_finished(&mut self, results: &[StepResult]) {
        let failed = results.iter().filter(|r| !r.success).count();
        let passed = results.iter().filter(|r| r.success).count();
        let total: f64 = results.iter().map(|r| r.duration.as_secs_f64()).sum();
        println!("--- run complete: {failed} failed, {passed} passed, {total:.1}s ---");
        let _ = std::io::stdout().flush();
    }
}
```

---

## Step 9 — `watcher/mod.rs`

The trickiest module. `notify-debouncer-mini` wants a `std::sync::mpsc::Sender` (blocking), but our consumer is async. Bridge the two with a dedicated OS thread.

```rust
// src/watcher/mod.rs
use anyhow::{Context, Result};
use notify_debouncer_mini::{new_debouncer, notify::RecursiveMode};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::sync::mpsc;

pub struct WatchConfig {
    pub root: PathBuf,
    pub debounce: Duration,
    pub extensions: Vec<String>,
    pub ignore: Vec<String>,
}

/// Starts the file watcher on a dedicated OS thread. Returns an async
/// receiver that yields `()` whenever a relevant batch of file events has
/// been debounced.
///
/// The channel is deliberately small — if many events pile up, we don't
/// care about the count, only that "something changed". Runner logic can
/// drain the channel after each pipeline run.
pub fn start(cfg: WatchConfig) -> Result<mpsc::Receiver<()>> {
    let (tx, rx) = mpsc::channel::<()>(8);

    std::thread::Builder::new()
        .name("baraddur-watcher".into())
        .spawn(move || watcher_thread(cfg, tx))
        .context("spawning watcher thread")?;

    Ok(rx)
}

fn watcher_thread(cfg: WatchConfig, tx: mpsc::Sender<()>) {
    // notify-debouncer-mini uses std sync mpsc for its event sender.
    let (sync_tx, sync_rx) = std::sync::mpsc::channel();

    let mut debouncer = match new_debouncer(cfg.debounce, sync_tx) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("baraddur: failed to start debouncer: {e}");
            return;
        }
    };

    if let Err(e) = debouncer
        .watcher()
        .watch(&cfg.root, RecursiveMode::Recursive)
    {
        eprintln!("baraddur: failed to watch {}: {e}", cfg.root.display());
        return;
    }

    // Blocking loop — each iteration is one debounced batch.
    for batch in sync_rx {
        let events = match batch {
            Ok(events) => events,
            Err(errs) => {
                for err in errs {
                    eprintln!("baraddur: watch error: {err}");
                }
                continue;
            }
        };

        let relevant = events.iter().any(|ev| matches_filters(&ev.path, &cfg));

        if relevant {
            // blocking_send waits for capacity. If the receiver is dropped,
            // the app is shutting down — exit the thread.
            if tx.blocking_send(()).is_err() {
                break;
            }
        }
    }
}

fn matches_filters(path: &Path, cfg: &WatchConfig) -> bool {
    for ignored in &cfg.ignore {
        if path
            .components()
            .any(|c| c.as_os_str() == ignored.as_str())
        {
            return false;
        }
    }

    if cfg.extensions.is_empty() {
        return true;
    }

    match path.extension().and_then(|e| e.to_str()) {
        Some(ext) => cfg.extensions.iter().any(|want| want == ext),
        None => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(exts: &[&str], ignore: &[&str]) -> WatchConfig {
        WatchConfig {
            root: PathBuf::from("."),
            debounce: Duration::from_millis(1000),
            extensions: exts.iter().map(|s| s.to_string()).collect(),
            ignore: ignore.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn matches_wanted_extension() {
        assert!(matches_filters(Path::new("src/foo.rs"), &cfg(&["rs"], &[])));
    }

    #[test]
    fn rejects_other_extension() {
        assert!(!matches_filters(Path::new("README.md"), &cfg(&["rs"], &[])));
    }

    #[test]
    fn rejects_ignored_dir() {
        assert!(!matches_filters(
            Path::new("target/debug/foo.rs"),
            &cfg(&["rs"], &["target"])
        ));
    }

    #[test]
    fn empty_extensions_matches_all() {
        assert!(matches_filters(Path::new("anything"), &cfg(&[], &[])));
    }
}
```

**Notes on the API** (verify when coding):
- `new_debouncer(timeout, event_handler)` — event handler must implement `DebounceEventHandler`, which is auto-implemented for `std::sync::mpsc::Sender<DebounceEventResult>`.
- `DebounceEventResult = Result<Vec<DebouncedEvent>, Vec<Error>>`.
- `debouncer.watcher().watch(path, mode)` — returns `Result<(), notify::Error>`.
- If the current crate version moved to `notify-debouncer-full` or renamed methods, adjust accordingly; the shape of the solution doesn't change.

---

## Step 10 — `lib.rs` (App struct, wiring)

```rust
// src/lib.rs
pub mod config;
pub mod output;
pub mod pipeline;
pub mod watcher;

use anyhow::Result;
use std::path::PathBuf;
use std::time::Duration;

use crate::config::Config;
use crate::output::{Display, PlainDisplay};

pub struct App {
    pub config: Config,
    pub config_path: PathBuf,
    pub root: PathBuf,
}

impl App {
    pub async fn run(self) -> Result<()> {
        eprintln!(
            "baraddur: watching {} (config: {})",
            self.root.display(),
            self.config_path.display()
        );

        let mut display: Box<dyn Display> = Box::new(PlainDisplay);

        // Initial run on start
        pipeline::run_sequential(&self.config, &self.root, display.as_mut()).await?;

        // Start the watcher
        let wcfg = watcher::WatchConfig {
            root: self.root.clone(),
            debounce: Duration::from_millis(self.config.watch.debounce_ms),
            extensions: self.config.watch.extensions.clone(),
            ignore: self.config.watch.ignore.clone(),
        };
        let mut rx = watcher::start(wcfg)?;

        // Event loop: wait for a trigger or Ctrl+C.
        loop {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {
                    eprintln!("\nbaraddur: exiting.");
                    return Ok(());
                }
                maybe = rx.recv() => {
                    match maybe {
                        None => return Ok(()), // watcher thread died
                        Some(()) => {
                            // Drain any pending triggers so we don't re-run per event
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

**Note:** Phase 1 does not cancel an in-flight pipeline when new events arrive — `tokio::select!` only polls `rx.recv()` between runs. Phase 3 introduces proper mid-run cancel+restart by awaiting events concurrently with the pipeline future.

---

## Step 11 — `main.rs`

```rust
// src/main.rs
use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;

#[derive(Parser, Debug)]
#[command(name = "baraddur", version, about = "Project-agnostic file watcher")]
struct Cli {
    /// Config file path
    #[arg(short = 'c', long)]
    config: Option<PathBuf>,

    /// Directory to watch
    #[arg(short = 'w', long)]
    watch_dir: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let (config, config_path) = baraddur::config::load(cli.config.as_deref())?;

    let root = match cli.watch_dir {
        Some(p) => p,
        None => std::env::current_dir()?,
    };

    let app = baraddur::App {
        config,
        config_path,
        root,
    };

    app.run().await
}
```

Other Phase 1-usable flags (`--no-clear`, `--no-tty`, `--summarize`, etc.) are deliberately **not** added yet — they don't affect Phase 1 behavior and adding stubs for them just invites drift. Add them when their corresponding phase implements them.

---

## Step 12 — Smoke Test

In a real Elixir project (or any project with `mix compile`-equivalent):

```sh
# 1. Install locally
cd ~/projects/baraddur
cargo install --path .

# 2. Configure the target project
cd ~/projects/my-elixir-app
cat > .baraddur.toml <<'TOML'
[watch]
extensions = ["ex", "exs", "heex"]
debounce_ms = 1000
ignore = ["_build", "deps", ".git"]

[[steps]]
name = "compile"
cmd = "mix compile"
TOML

# 3. Run it
baraddur
```

### What to verify manually

- [ ] Initial compile runs immediately on startup.
- [ ] Touching a `.ex` file triggers a re-run (watch a `touch lib/foo.ex`).
- [ ] Touching a `.md` file does **not** trigger a run (extension filter works).
- [ ] Touching a file under `deps/` does **not** trigger a run (ignore filter works).
- [ ] Introducing a compile error shows the error text under `── compile output ──`.
- [ ] Ctrl+C exits cleanly with code 0 (check with `echo $?`).
- [ ] Rapid file saves within the debounce window (< 1s apart) collapse into a single re-run.

---

## Unit Tests to Include

Phase 1 has three cheap, high-signal unit tests that are worth writing:

1. **`config::schema::tests::parses_minimal_config`** (Step 3) — defends against serde-field rename regressions.
2. **`watcher::tests::matches_*`** (Step 9) — defends against filter logic regressions.
3. **Integration test for the runner** — add `tests/runner.rs`:

```rust
// tests/runner.rs
use baraddur::config::{Config, OutputConfig, Step, WatchConfig};
use baraddur::output::Display;
use baraddur::pipeline;
use baraddur::pipeline::StepResult;

#[derive(Default)]
struct RecordingDisplay {
    events: Vec<String>,
}

impl Display for RecordingDisplay {
    fn run_started(&mut self) { self.events.push("run_started".into()); }
    fn step_started(&mut self, name: &str) { self.events.push(format!("start:{name}")); }
    fn step_finished(&mut self, r: &StepResult) {
        self.events.push(format!("finish:{}:{}", r.name, r.success));
    }
    fn run_finished(&mut self, _: &[StepResult]) { self.events.push("run_finished".into()); }
}

fn config(steps: Vec<Step>) -> Config {
    Config {
        watch: WatchConfig { extensions: vec!["rs".into()], debounce_ms: 1000, ignore: vec![] },
        output: OutputConfig::default(),
        steps,
    }
}

#[tokio::test]
async fn stops_at_first_failure() {
    let cfg = config(vec![
        Step { name: "first".into(),  cmd: "true".into(),  parallel: false },
        Step { name: "second".into(), cmd: "false".into(), parallel: false },
        Step { name: "third".into(),  cmd: "true".into(),  parallel: false },
    ]);
    let mut display = RecordingDisplay::default();
    let cwd = std::env::current_dir().unwrap();

    let results = pipeline::run_sequential(&cfg, &cwd, &mut display).await.unwrap();

    assert_eq!(results.len(), 2);
    assert!(results[0].success);
    assert!(!results[1].success);
    // `third` never started
    assert!(!display.events.iter().any(|e| e.contains("third")));
}
```

(May require `Config` fields to be `pub` — they already are in the schema above.)

---

## References

### Crate documentation

| Crate | URL |
|---|---|
| notify-debouncer-mini | https://docs.rs/notify-debouncer-mini |
| notify (underlying) | https://docs.rs/notify |
| tokio::process | https://docs.rs/tokio/latest/tokio/process/index.html |
| tokio::sync::mpsc | https://docs.rs/tokio/latest/tokio/sync/mpsc/index.html |
| tokio::signal::ctrl_c | https://docs.rs/tokio/latest/tokio/signal/fn.ctrl_c.html |
| clap (derive tutorial) | https://docs.rs/clap/latest/clap/_derive/_tutorial/ |
| serde (with toml) | https://serde.rs/ |
| toml crate | https://docs.rs/toml |
| shell-words | https://docs.rs/shell-words |
| anyhow | https://docs.rs/anyhow |

### Conceptual reading

- Rust async book, *Spawning, channels, select* — https://rust-lang.github.io/async-book/
- Tokio "Shared state" mini-redis tutorial — patterns for `mpsc` + `select!`
- `notify` user guide — event semantics, cross-platform backend notes

---

## Common Pitfalls

1. **`notify-debouncer-mini` API drift.** Method names (`new_debouncer` vs `new_debouncer_opt`) and types (`DebouncedEvent` vs `DebouncedEventKind`) have shifted between minor versions. Read the docs.rs page for the exact version you pin.
2. **Bridging `std::sync::mpsc` → `tokio::sync::mpsc`.** The watcher crate's sender is blocking; your pipeline is async. Always use a dedicated OS thread (as in the plan) — do NOT try to receive from a `std::sync::mpsc::Receiver` inside an async context.
3. **`tokio::process::Command` requires the `process` feature flag.** Ensure `features = ["process", ...]` in Cargo.toml.
4. **`kill_on_drop(true)`** is essential if Phase 3 will introduce cancellation — set it now even though Phase 1 never cancels.
5. **File events fire for `.git/` changes too** (e.g. during git operations). The default `ignore = [".git"]` is important — don't omit it from smoke-test configs.
6. **Working directory matters.** `tokio::process::Command` inherits the parent's cwd unless you call `.current_dir()`. We explicitly set it to `root` so `mix compile` runs in the project, not in wherever `baraddur` was launched.
7. **Elixir's `mix` writes to stdout, not stderr.** Don't assume stderr means failure — rely on `ExitStatus::success()`.
8. **Rapid file saves can cause the debouncer to fire twice** if saves span its window. The `rx.try_recv()` drain loop in `lib.rs` handles this.
9. **Config walk-up is NOT implemented** in Phase 1 — if testing, always run `baraddur` from the directory containing `.baraddur.toml`.

---

## Definition of Done

- [ ] `cargo build --release` produces a working binary.
- [ ] `cargo test` passes (schema parse, filter matching, runner integration test).
- [ ] `cargo clippy -- -D warnings` is clean.
- [ ] `cargo fmt --check` is clean.
- [ ] Smoke test checklist (Step 12) completes successfully in a real Elixir project.
- [ ] Ctrl+C exits with code 0.
- [ ] Initial commit(s) pushed with the full Phase 1 implementation.

Once all boxes are checked, Phase 2 (config polish + ignore patterns) is ready to start.
