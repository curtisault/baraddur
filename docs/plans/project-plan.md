# Barad-dûr — Build Plan

A cross-platform, project-agnostic file watcher CLI that surfaces issues before CI does.
Named after the tower where the Eye of Sauron is seated — it watches everything.

---

## Goals

- Event-driven file watching (OS-level, not polling)
- Configurable check steps per project via `.baraddur.toml`
- Sequential and parallel step execution
- Clean terminal output — clear screen, show only what matters
- Failure summarization via `claude -p`
- Single binary, distributable to the team

---

## Project Structure

```
baraddur/
├── Cargo.toml
├── src/
│   ├── main.rs            # CLI entry point — thin, just parses args and calls lib
│   ├── lib.rs             # Public API, wires modules together
│   ├── config/
│   │   ├── mod.rs         # Config loading (file discovery, merging)
│   │   └── schema.rs      # Config types (WatchConfig, Step, OutputConfig)
│   ├── watcher/
│   │   └── mod.rs         # notify-debouncer-mini setup, event filtering
│   ├── pipeline/
│   │   ├── mod.rs         # Pipeline orchestration — stages, results
│   │   ├── step.rs        # Individual step definition and execution
│   │   └── runner.rs      # Stage runner — parallel via tokio::JoinSet
│   ├── output/
│   │   ├── mod.rs         # Output coordinator
│   │   └── display.rs     # Terminal control (crossterm), formatting, colors
│   └── summarize/
│       └── mod.rs         # Claude CLI integration via std::process::Command
```

---

## Config Schema (`.baraddur.toml`)

```toml
[watch]
extensions = ["ex", "exs", "heex"]
debounce_ms = 1500
ignore = ["_build", "deps", ".git", "node_modules"]

[output]
clear_screen = true
show_passing = false     # hide stdout from passing steps
claude_summaries = true
max_summary_lines = 5

[[steps]]
name = "format"
cmd = "mix format --check-formatted"
parallel = false         # must pass before continuing

[[steps]]
name = "compile"
cmd = "mix compile --warnings-as-errors"
parallel = false

[[steps]]
name = "credo"
cmd = "mix credo"
parallel = true          # runs alongside other parallel steps

[[steps]]
name = "test"
cmd = "mix test --failed"
parallel = true
```

Config is discovered by walking up from the current directory, like `.gitignore`.
A global fallback lives at `~/.config/baraddur/config.toml`.

---

## Pipeline Execution Model

Steps are grouped into **stages** based on their `parallel` flag. Consecutive parallel
steps are batched into a single stage. Sequential steps are each their own stage.

```
stage 1: [format]           # parallel=false → runs alone, must pass
stage 2: [compile]          # parallel=false → runs alone, must pass
stage 3: [credo, test]      # parallel=true  → run concurrently via JoinSet
```

Rules:
- If any step in a stage fails, subsequent stages are skipped
- Within a parallel stage, all steps run to completion before reporting
- Exit code non-zero = failure; stderr is captured alongside stdout

---

## Terminal Output Design

```
━━━ 14:32:01 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format   ✓
▸ compile  ✓
▸ credo    ✗  (3 issues)
▸ test     ✓

── credo output ────────────────────────────────────────────────────
lib/foo.ex:42 [C] Modules should have a @moduledoc tag
...

── Claude summary ───────────────────────────────────────────────────
3 Credo issues in lib/foo.ex: missing @moduledoc (line 42), ...
────────────────────────────────────────────────────────────────────
```

Behavior:
- Screen clears before each run (if `clear_screen = true`)
- Passing steps show name + checkmark only (or nothing if `show_passing = false`)
- Failing steps show name, raw output, then Claude summary
- Parallel steps show as "running..." until complete, then update in place

---

## CLI Interface

```
baraddur [OPTIONS]

Options:
  -c, --config <FILE>     Config file [default: .baraddur.toml]
  -w, --watch-dir <DIR>   Directory to watch [default: current dir]
      --no-clear          Don't clear screen between runs
      --no-claude         Disable Claude summaries
  -v, --verbose           Show output from passing steps
  -h, --help
  -V, --version
```

---

## Key Crates

| Crate | Purpose |
|---|---|
| `notify-debouncer-mini` | OS-level file watching with event batching |
| `tokio` (full features) | Async runtime, `JoinSet` for parallel steps |
| `clap` (derive) | CLI argument parsing |
| `serde` + `toml` | Config deserialization |
| `crossterm` | Terminal control — clear screen, colors, cursor |
| `anyhow` | Application-level error handling |
| `thiserror` | Typed errors for library-facing code |

---

## Implementation Phases

### Phase 1 — Core Watcher + Sequential Pipeline
- [ ] `cargo new baraddur`
- [ ] Set up `Cargo.toml` with all dependencies and release profile
- [ ] `config/schema.rs` — define `Config`, `Step`, `WatchConfig`, `OutputConfig` structs with serde
- [ ] `config/mod.rs` — load `.baraddur.toml` by walking up from cwd, fall back to `~/.config/baraddur/config.toml`
- [ ] `watcher/mod.rs` — set up `notify-debouncer-mini`, filter events by extension, send to pipeline via `tokio::sync::mpsc`
- [ ] `pipeline/step.rs` — `Step::run()` using `tokio::process::Command`, capture stdout+stderr, return `StepResult`
- [ ] `pipeline/runner.rs` — sequential stage execution, stop on first failure
- [ ] `output/display.rs` — print step name, pass/fail, raw output on failure (no Claude yet, no screen clearing)
- [ ] Wire everything in `lib.rs` and `main.rs`
- [ ] Smoke test: watches an Elixir project, runs `mix compile` on change

### Phase 2 — Config Polish + Ignore Patterns
- [ ] Validate config on load — surface helpful errors (unknown field, missing cmd, etc.)
- [ ] Implement `ignore` patterns — skip events from ignored paths
- [ ] Config discovery walk-up (`.baraddur.toml` in parent dirs)
- [ ] Global `~/.config/baraddur/config.toml` fallback
- [ ] CLI `--config` override

### Phase 3 — Parallel Stage Execution
- [ ] `pipeline/mod.rs` — group steps into stages by consecutive `parallel` flag
- [ ] `pipeline/runner.rs` — parallel stages via `tokio::task::JoinSet`
- [ ] Collect all parallel step results before deciding to continue
- [ ] Test with `mix credo` + `mix test --failed` running concurrently

### Phase 4 — Terminal Output
- [ ] `output/display.rs` — clear screen with `crossterm` before each run
- [ ] Live "running..." indicators for parallel steps
- [ ] Update step status in place when parallel steps complete (use cursor movement)
- [ ] Respect `show_passing` — suppress stdout from passing steps
- [ ] Color: green ✓ for pass, red ✗ for fail, yellow ⟳ for in-progress

### Phase 5 — Claude Summarization
- [ ] `summarize/mod.rs` — check if `claude` binary is in `$PATH`
- [ ] Pipe step output to `claude -p "..."` via `std::process::Command`
- [ ] Respect `claude_summaries = false` in config and `--no-claude` CLI flag
- [ ] Respect `max_summary_lines` — truncate or instruct Claude in the prompt
- [ ] Graceful degradation — if Claude isn't available or fails, skip silently

### Phase 6 — Distribution
- [ ] GitHub repository: `baraddur`
- [ ] GitHub Actions CI: `cargo test`, `cargo clippy`, `cargo fmt --check` on PRs
- [ ] GitHub Actions release: build binaries for `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-linux-gnu` on tag push
- [ ] Attach binaries to GitHub release
- [ ] Install script: `curl ... | sh` that puts `baraddur` in `~/.local/bin`
- [ ] Publish to crates.io (optional, later)

---

## Cargo.toml (planned)

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

[dependencies]
notify-debouncer-mini = "0.4"
tokio = { version = "1", features = ["full"] }
clap = { version = "4", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
toml = "0.8"
crossterm = "0.27"
anyhow = "1"
thiserror = "2"

[profile.release]
opt-level = 3
lto = "fat"
codegen-units = 1
panic = "abort"
strip = true
```

---

## Open Questions

- Should `parallel = true` steps ever be able to depend on each other? (Probably no — keep it simple.)
- Should there be a `timeout_secs` per step to kill runaway processes?
- Should Barad-dûr support watching multiple directories?
- What's the install story for Windows? (Low priority for now.)
