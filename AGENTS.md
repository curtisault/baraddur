# Barad-dûr — Agent Guide

Barad-dûr is a project-agnostic file-watcher CLI written in Rust. It monitors source code changes and automatically runs a configurable pipeline of check/build/test steps, surfacing issues before CI does.

## Build & Verification

```sh
just build       # debug build
just release     # optimized build
just install     # release + copy to ~/.local/bin
just ci          # fmt-check + clippy + test  ← the gate; must pass clean
just test        # cargo test only
just lint        # cargo clippy -- -D warnings
```

`just ci` is the definition of "done." All three checks (fmt, clippy, test) must pass with zero warnings before any change is considered complete.

## Architecture

```
src/
├── main.rs          # CLI arg parsing via clap; calls lib::run()
├── lib.rs           # Main async loop: tokio::select! over file events, pipeline results, Ctrl-C
├── config/          # Walk-up .baraddur.toml discovery, serde structs, validation
├── pipeline/        # Stage grouping, parallel execution (tokio::task::JoinSet), step runner
├── output/          # Display trait + PlainDisplay (CI) and TtyDisplay (interactive) impls
└── watcher/         # notify-debouncer-mini setup, extension filtering
```

### Key patterns

- **Display trait** (`src/output/mod.rs`): the *only* place that touches the terminal. `PlainDisplay` for non-TTY/CI, `TtyDisplay` for interactive. Both are `Box<dyn Display>` at runtime. Never call crossterm directly outside `src/output/`.
- **Stage-based pipeline** (`src/pipeline/`): steps are grouped into stages by consecutive `parallel` flag. Sequential steps each get their own stage; consecutive parallel steps batch into one stage run via `JoinSet`. A failing stage skips all subsequent stages.
- **Config** (`src/config/`): TOML deserialized with `deny_unknown_fields` on every struct. Adding a config field requires updating `schema.rs` *and* `validate.rs` together.
- **Mid-run restart**: when a file change arrives while the pipeline is running, child processes are killed and the run restarts. Do not simplify this cancellation path away.

## Invariants — Do Not Break

1. All terminal I/O goes through the `Display` trait.
2. `deny_unknown_fields` on config structs — schema and validation must stay in sync.
3. Mid-run file changes cancel and restart the pipeline.
4. `just ci` must pass with zero warnings (clippy is `-D warnings`).
5. LLM integration spawns an **external CLI binary** (`cmd = "claude"`), not a library. Keep it that way.

## Phase Status

| Phase | Status | Notes |
|-------|--------|-------|
| 1–5   | Done   | Watch, config, parallel execution, terminal polish, browse mode |
| 6     | Planned | LLM failure summaries — `SummarizeConfig` is parsed in `src/config/schema.rs` but not yet executed |
| 7     | Planned | Distribution: GitHub Actions CI, release binaries |

**Phase 6 hook point:** after a stage fails in `src/pipeline/runner.rs`, pipe combined stdout/stderr to the command in `SummarizeConfig.cmd` via stdin.

## Testing

- Unit tests live alongside source modules in `src/`
- Integration tests in `tests/`
- For display changes: verify both `--no-tty` (PlainDisplay) and interactive (TtyDisplay) modes
- `NO_COLOR=1` must suppress all color output — enforced in `src/output/style.rs`

## Config Schema Reference

```toml
[watch]
extensions = ["rs", "toml"]
debounce_ms = 500
ignore = ["target", ".git"]

[output]
clear_screen = true
show_passing = false   # hide stdout from passing steps

[summarize]            # Phase 6: parsed, not yet executed
enabled = false
cmd = "claude"
prompt = "Summarize failures in under 5 lines..."
timeout_secs = 15

[[steps]]
name = "check"
cmd = "cargo check"
parallel = false       # sequential — blocks next step

[[steps]]
name = "clippy"
cmd = "cargo clippy -- -D warnings"
parallel = true        # runs concurrently with other parallel steps
```
