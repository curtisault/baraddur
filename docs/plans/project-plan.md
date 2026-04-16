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

## Companion Documents

- **[`ux-design.md`](./ux-design.md)** — full UX design: state mockups,
  transitions, error states, visual language, output modes. **Check here
  first** before raising UX-related questions.
- **[`phase-1-plan.md`](./phase-1-plan.md)** — detailed implementation guide
  for Phase 1: Cargo.toml, module-by-module code sketches, tests, smoke-test
  procedure, and common pitfalls.

---

## Behavioral Specifications

- **Run on start.** Execute the full pipeline immediately on launch — no
  waiting for the first file change.
- **Mid-run cancel + restart.** If file events arrive while a pipeline is
  running, abort the in-flight run (kill all child processes) and start a
  fresh one. The user always sees results for the latest tree state.
- **Graceful shutdown on Ctrl+C.** Stop the watcher, kill any child
  processes, drain channels, exit clean with code 0. A second Ctrl+C within
  2s force-exits with code 130.
- **Non-TTY fallback.** When stdout is not a terminal (piped, CI), emit
  append-only line-oriented output — no cursor movement, no colors, no
  animation. Detected via `std::io::IsTerminal`; overridable with `--no-tty`.
- **Command parsing.** `cmd` strings are tokenized with `shell-words` (POSIX
  shell-style splitting, no shell features). For pipes / `&&` / globs, use
  an explicit `cmd = "sh -c '...'"`.
- **LLM summarization is opt-in.** Disabled by default; enabled via config
  (`[summarize] enabled = true`) or `--summarize` flag. Step output is piped
  to the LLM command via stdin, the prompt template is passed as trailing
  argument, and the call is bounded by a configurable timeout (default 15s).

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

[summarize]
enabled = false          # opt-in LLM failure summaries
cmd = "claude"           # LLM CLI binary; step output piped via stdin
prompt = "Summarize these check failures in under 5 lines. Focus on root cause and cite file:line where possible."
timeout_secs = 15

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

> **Full design in [`ux-design.md`](./ux-design.md)** — state mockups,
> transitions, error states, visual language, non-TTY mode, truncation.
> This section is the happy-path reference.

```
━━━ 14:32:01 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▸ compile   ✓   1.1s
▸ credo     ✗   3 issues   1.8s
▸ test      ✓   2.3s

── credo output ────────────────────────────────────────────────────
  lib/foo.ex:42:3 [C] Modules should have a @moduledoc tag
  ...

── summary ─────────────────────────────────────────────────────────
3 Credo issues in lib/foo.ex: missing @moduledoc (line 42), ...

1 failed · 3 passed · 5.4s
```

Headline behaviors:
- Screen clears before each run (if `clear_screen = true` and TTY)
- Passing steps show glyph + duration (or nothing if `show_passing = false` and `-q`)
- Failing steps show inline diagnostic + raw output + LLM summary (if enabled)
- Parallel steps show `⟳` concurrently; each updates in place when done
- Non-TTY mode emits append-only line output (see UX doc)

---

## CLI Interface

```
baraddur [OPTIONS]

Options:
  -c, --config <FILE>          Config file [default: .baraddur.toml]
  -w, --watch-dir <DIR>        Directory to watch [default: current dir]
      --no-clear               Don't clear screen between runs
      --no-tty                 Force non-TTY (append-only) output
      --summarize              Enable LLM failure summaries (override config)
      --no-summarize           Disable LLM failure summaries (override config)
      --prompt-template <T>    Inline string, or @path/to/file.txt
      --summarize-timeout <S>  Max seconds to wait for LLM [default: 15]
  -v, --verbose                Show output from passing steps (-vv for debug)
  -q, --quiet                  Only show failures
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

> **Detailed implementation guide: [`phase-1-plan.md`](./phase-1-plan.md)** —
> module-by-module code sketches, crate references, unit tests, and smoke-test
> checklist.

- [x] `cargo new baraddur`
- [x] Set up `Cargo.toml` with all dependencies and release profile
- [x] `config/schema.rs` — define `Config`, `Step`, `WatchConfig`, `OutputConfig` structs with serde
- [x] `config/mod.rs` — load `.baraddur.toml` by walking up from cwd, fall back to `~/.config/baraddur/config.toml`
- [x] `watcher/mod.rs` — set up `notify-debouncer-mini`, filter events by extension, send to pipeline via `tokio::sync::mpsc`
- [x] `pipeline/step.rs` — `Step::run()` using `tokio::process::Command`, capture stdout+stderr, return `StepResult`
- [x] `pipeline/runner.rs` — sequential stage execution, stop on first failure
- [x] `output/display.rs` — print step name, pass/fail, raw output on failure (no Claude yet, no screen clearing)
- [x] Wire everything in `lib.rs` and `main.rs`
- [ ] Smoke test: watches an Elixir project, runs `mix compile` on change

### Phase 2 — Config Polish + Ignore Patterns
- [x] Validate config on load — surface helpful errors (unknown field, missing cmd, etc.)
- [x] Implement `ignore` patterns — skip events from ignored paths
- [x] Config discovery walk-up (`.baraddur.toml` in parent dirs)
- [x] Global `~/.config/baraddur/config.toml` fallback
- [x] CLI `--config` override

### Phase 3 — Parallel Execution + Lifecycle + Basic Redraw
- [x] `pipeline/mod.rs` — group steps into stages by consecutive `parallel` flag
- [x] `pipeline/runner.rs` — parallel stages via `tokio::task::JoinSet`
- [x] Collect all parallel step results before deciding to continue
- [x] `output/display.rs` — full-block redraw on each state change so parallel
      step statuses can update cleanly (no per-step cursor math yet)
- [x] TTY detection at startup (`std::io::IsTerminal`); non-TTY path emits
      append-only line output
- [x] Graceful shutdown: trap Ctrl+C, cancel in-flight pipeline, kill children,
      double-tap within 2s force-exits
- [x] Mid-run cancel+restart: on new file events while pipeline is running,
      abort current run and launch a fresh one
- [ ] Test with `mix credo` + `mix test --failed` running concurrently

### Phase 4 — Terminal Polish
- [x] Clear screen with `crossterm` before each run (respect `--no-clear`)
- [x] Colors: green `✓`, red `✗`, yellow `⟳`, cyan section headers, dim metadata
- [x] Respect `NO_COLOR` env var; auto-disable in non-TTY mode
- [x] Spinner animation for in-progress steps (single shared frame clock)
- [x] Step timings right-aligned; short diagnostics inline with failing steps
- [x] Output truncation: cap per-step capture at 100 KiB, show head+tail on
      display; write full capture to `.baraddur/last-run.log`
- [x] Verbosity levels: `-q` quiet, default, `-v` verbose, `-vv` debug
- [x] Startup banner and idle footer

### Phase 5 — Browse Mode (Post-Run Interactive Navigation)
- [x] Enter browse mode (S11) automatically after every pipeline run in TTY mode
- [x] Cursor highlight: reverse-video on `▸ name` in color mode; `▶` glyph
      fallback when color is disabled
- [x] Keybindings: `j`/`k`/`↑`/`↓` navigate, `gg` first, `G` last,
      `Enter`/`o` toggle output, `O` expand-all toggle, `q` quit
- [x] Initial state on failure: first failing step selected + output pre-expanded
- [x] Initial state on pass: cursor on row 0, no output expanded
- [x] Exit browse on file change → cursor/help bar clear, new run starts
- [x] `q` in browse mode → clean shutdown (same path as Ctrl+C)
- [x] `enable_raw_mode()` during browse; `OPOST`/`ISIG` re-enabled immediately
      after so `println!` and Ctrl+C keep working; raw mode restored on exit
- [x] Echo suppression (`ECHO`/`ECHOE` cleared) during pipeline runs so
      keystrokes don't corrupt the step-status block; restored on drop

### Phase 6 — LLM Summarization (Optional, Off by Default)
- [ ] `summarize/mod.rs` — feature is opt-in; off by default
- [ ] Invoke configured `cmd` via `tokio::process::Command`, pipe step output
      via stdin, prompt template as trailing argument
- [ ] Config: `[summarize] enabled / cmd / prompt / timeout_secs`
- [ ] CLI: `--summarize` / `--no-summarize` override config
- [ ] CLI: `--prompt-template "<inline>"` or `--prompt-template @path/to/file`
- [ ] Enforce `timeout_secs` with `tokio::time::timeout` — skip silently on
      timeout, note in footer (`summary skipped (timeout)`)
- [ ] Graceful degradation: if `cmd` binary not in `$PATH`, skip with a dim
      footer note (`summary skipped (command not found)`)
- [ ] Render summary in its own section below raw output, cyan divider

### Phase 7 — Distribution
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
- UX-specific open questions (spinner in non-TTY, summary streaming,
  persistent status line) live in [`ux-design.md`](./ux-design.md).
