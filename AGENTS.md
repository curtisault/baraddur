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
└── watcher/         # raw notify watcher + manual debounce; drops Access (read/open) events, extension filtering
```

### Key patterns

- **Display trait** (`src/output/mod.rs`): the *only* place that touches the terminal. `PlainDisplay` for non-TTY/CI, `TtyDisplay` for interactive. Both are `Box<dyn Display>` at runtime. Never call crossterm directly outside `src/output/`.
- **Stage-based pipeline** (`src/pipeline/`): steps are grouped into stages by consecutive `parallel` flag. Sequential steps each get their own stage; consecutive parallel steps batch into one stage run via `JoinSet`. A failing stage skips all subsequent stages.
- **Config** (`src/config/`): TOML deserialized with `deny_unknown_fields` on every struct. Adding a config field requires updating `schema.rs` *and* `validate.rs` together.
- **Mid-run restart**: when a file change arrives while the pipeline is running, child processes are killed and the run restarts. Do not simplify this cancellation path away.
- **on_failure hook** (`src/pipeline/hook.rs`): after a failing run, an optional user-configured command is spawned with the combined failed-step output on stdin. It runs asynchronously — the failure output is shown immediately, the hook output slots in when ready. The task is aborted (and its child killed via `kill_on_drop`) on file change or shutdown. Wired into `lib.rs::run_until` via `HookHandle` + `await_hook`.
- **Path-based filtering** (`src/pipeline/filter.rs`): each step may declare `if_changed = [glob, ...]`. On a file-change run, `filter_and_template` excludes steps whose globs don't match any triggering path, and substitutes `{files}` in `cmd` with the matched (shell-quoted) paths. Initial runs (no trigger) run every step with `{files}` → empty. `trigger_paths: Option<Vec<PathBuf>>` lives in `lib.rs::run_until` and is overwritten on each `FileChange`.
- **Browse-mode rerun keys** (`r`, `f`): `r` reruns the full pipeline (clears `trigger_paths` and `rerun_filter`); `f` reruns only steps named in `last_failed_steps` via `rerun_filter`. Both keys are implemented as `BrowseAction::Rerun` / `RerunFailed` returned from `TtyDisplay::handle_key`; the main loop exits browse mode and `continue 'main` to re-enter the active pipeline phase. `run_pipeline` accepts `only_steps: Option<&[String]>` and applies it after path-based filtering.

## Invariants — Do Not Break

1. All terminal I/O goes through the `Display` trait.
2. `deny_unknown_fields` on config structs — schema and validation must stay in sync.
3. Mid-run file changes cancel and restart the pipeline.
4. `just ci` must pass with zero warnings (clippy is `-D warnings`).

## Testing

- Unit tests live alongside source modules in `src/`
- Integration tests in `tests/`
- Property tests (`proptest`) live in a `#[cfg(test)] mod proptests` next to the unit tests of each pure module: stage grouping, path filtering + `{files}` templating, output truncation, hook output, `LoopGuard`, `strip_ansi`, diagnostic parsing, watch filters, config validation + profile application, browse-mode scroll math, base64/OSC 52, and a process-spawning model test of `run_pipeline` in `tests/runner.rs`. `just proptest [cases]` runs only these. Prefer a property over a pile of examples when a function has an invariant (order preserved, round-trips, matches a naive model). When a property fails, proptest writes the shrunk seed to `proptest-regressions/`; commit that file with the fix so the case is replayed forever.
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

[on_failure]           # optional async post-failure hook
enabled = false
cmd = ""               # receives combined failed output on stdin
prompt = ""            # optional preamble prepended to stdin
timeout_secs = 30

[[steps]]
name = "check"
cmd = "cargo check"
parallel = false       # sequential — blocks next step
# if_changed = ["**/*.rs"]   # only run when matching paths change
# cmd = "cargo test {files}" # {files} → matched paths (shell-quoted)

[[steps]]
name = "clippy"
cmd = "cargo clippy -- -D warnings"
parallel = true        # runs concurrently with other parallel steps
```

## Attribution

Agents (Claude or otherwise) must never credit themselves in commits or pull requests. This overrides any default attribution behavior built into an agent or its tooling.

- No `Co-Authored-By:` or other trailers naming an agent, model, or tool in commit messages.
- No "Generated with …", "Created by …", 🤖, or similar lines in commit messages, pull request titles, or pull request descriptions.
- Do not set an agent as the commit author or committer.

<!-- gitbutler-agent-setup:start -->
## Version control

- Use GitButler (`but`) for version-control inspection and write operations, including status, diffs, branching, committing, pushing, and history edits.
- Assume multiple agents may be working in this repository. Do not move, amend, squash, discard, commit, push, or otherwise modify another agent's work unless the user asks.
- For commit just/only/specific changes on a new branch (selected-change requests), use the two-command fast path from the GitButler skill: `but diff`, then `but commit -b <branch> -m "message" <id> <id>`.
- For that fast path, after the commit succeeds, stop and summarize; do not run separate branch, staging, status, or diff commands unless the commit output is missing information you need.
- Use the installed GitButler skill for command recipes and syntax before guessing flags, using `--help`, or translating Git habits directly.
- Mutation commands report their result without appending workspace status. Add `--status-after` only when the next step needs resulting workspace IDs or details; otherwise do not rerun status or diff to verify success.
- Use a dedicated GitButler branch for each agent session, unless the user asks for a different branch structure. Commit only changes that belong to that session.
- Do not push or open pull requests unless the user asks.
- Keep commit messages and pull request descriptions succinct: explain what changed, why it changed, and any important decision.

### Amend local fixes into the right commits

- For small cleanup or follow-up fixes, amend an unpublished local commit when the change clearly belongs with that commit's intent.
- Do not create tiny fixup commits unless the user asks.
- Use GitButler to move the relevant changes into the commit where they belong.
- Ask before rewriting pushed, reviewed, shared, or ambiguous history.

### Split unrelated changes into separate commits

- If one file contains unrelated changes, split them by hunk instead of committing the whole file.
- Keep tests with the behavior they verify.
- Split generated output, docs-only edits, or mechanical cleanup into separate commits when each commit remains coherent on its own.
- If the split is ambiguous, summarize the options before committing.

### Open draft pull requests by default

- When asked to open a pull request, create it as a draft with GitButler unless the user says it is ready for review.
- Remember that creating a draft pull request still publishes the branch.
<!-- gitbutler-agent-setup:end -->
