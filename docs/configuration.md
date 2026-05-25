# Configuration

Config is discovered by walking up from the current directory (like
`.gitignore`). A global fallback lives at `~/.config/baraddur/config.toml`.

To pin to a specific file and disable walk-up discovery, pass `-c`:

```bash
baraddur -c ./.baraddur.toml
```

## Full schema

```toml
[watch]
extensions = ["ex", "exs", "heex"]  # file extensions to watch
debounce_ms = 1000                  # wait this long after the last event before running
ignore = ["_build", "deps", ".git", ".baraddur"] # names match any path component; paths with / match by prefix

[output]
clear_screen = true   # clear the terminal between runs
show_passing = false  # hide stdout/stderr from passing steps

[on_failure]            # optional post-failure hook
enabled = false         # off by default; opt in per-project
cmd = ""                # any command; receives combined failed output on stdin
prompt = ""             # optional preamble prepended to stdin before the output
timeout_secs = 30       # killed if it runs longer

[[steps]]
name = "format"
cmd  = "mix format --check-formatted"
parallel = false  # must pass before continuing

[[steps]]
name = "credo"
cmd  = "mix credo"
parallel = true   # runs concurrently with other parallel steps

[[steps]]
name = "test"
cmd  = "mix test --failed"
parallel = true
if_changed = ["**/*.ex", "**/*.exs"]   # only run when matching paths change
# cmd = "mix test {files}"             # {files} → matched paths, shell-quoted
```

## Path-based step filtering

Each step may declare `if_changed`, a list of glob patterns matched against
paths reported by the file watcher. When set:

- **File-change runs**: the step runs only if at least one changed path
  matches a pattern. Steps with no matches are excluded from the run entirely
  (they don't appear in the step list).
- **Initial run** (no triggering files): every step runs, regardless of
  `if_changed`. The empty default means "always run."

The `{files}` token in `cmd` is substituted with the relevant paths,
shell-quoted and space-separated:

- A step with `if_changed` set: `{files}` is the matched subset.
- A step without `if_changed`: `{files}` is every changed path.
- Initial run: `{files}` is empty (so `cargo test {files}` → `cargo test`).

```toml
[[steps]]
name = "type-check"
cmd = "tsc --noEmit"
if_changed = ["**/*.ts", "**/*.tsx"]

[[steps]]
name = "rust-test"
cmd = "cargo test {files}"        # narrows test target to changed files
if_changed = ["**/*.rs"]
```

Patterns use [`globset`](https://docs.rs/globset) syntax (gitignore-style
globs with `**` and `*`).

## Examples

### Rust / Cargo

```toml
[watch]
extensions = ["rs"]
debounce_ms = 500
ignore = ["target", ".git"]

[[steps]]
name = "check"
cmd = "cargo check"
parallel = false

[[steps]]
name = "test"
cmd = "cargo test"
parallel = false
```

### TypeScript / Node.js

```toml
[watch]
extensions = ["ts", "tsx"]
debounce_ms = 500
ignore = ["node_modules", "dist", ".baraddur"]

[output]
clear_screen = true
show_passing = false

[[steps]]
name = "lint"
cmd = "npx biome check ."
parallel = true

[[steps]]
name = "type-check"
cmd = "npx tsc --noEmit"
parallel = true

[[steps]]
name = "unused-exports"
cmd = "npx knip"
parallel = true
```

All three steps run concurrently as a single stage. Swap in `eslint`,
`prettier`, or any other tool you prefer.

### Elixir / Mix

```toml
[watch]
extensions = ["ex", "exs", "heex"]
debounce_ms = 500
ignore = ["_build", "deps", ".git", ".expert"]

[[steps]]
name = "format"
cmd = "mix format --check-formatted"
parallel = false

[[steps]]
name = "compile"
cmd = "mix compile --warnings-as-errors"
parallel = false

[[steps]]
name = "credo"
cmd = "mix credo"
parallel = true

[[steps]]
name = "test"
cmd = "mix test --failed"
parallel = true
```

## Parallel steps

Consecutive `parallel = true` steps run as a batch — all start at once, all
must complete before the next stage begins. `parallel = false` steps always
run alone and gate everything after them.

```
stage 1: [format]         # parallel=false — must pass
stage 2: [compile]        # parallel=false — must pass
stage 3: [credo, test]    # parallel=true  — run concurrently
```

If any stage fails, subsequent stages are skipped.

## Command parsing

`cmd` strings are split with POSIX shell-word rules (`shell-words` crate).
Shell features like pipes, `&&`, and glob expansion are not supported. For
those, use `sh -c`:

```toml
cmd = "sh -c 'mix compile 2>&1 | head -50'"
```

## Post-failure hook

When `[on_failure].enabled = true` and any step in a completed run fails,
the configured `cmd` is spawned with the combined stdout+stderr of failing
steps on stdin. `prompt` (if non-empty) is prepended as a preamble. The
captured stdout is shown below the failure summary; non-zero exits, empty
stdout, and timeouts are silently suppressed (with a stderr diagnostic).

The hook runs asynchronously — your failure output is shown immediately and
the hook output slots in when ready. A file change cancels the in-flight
hook and kills the subprocess.

Examples:

```toml
# Pipe the failure output through an LLM CLI for a short summary.
[on_failure]
enabled = true
cmd = "claude -p"
prompt = "Summarize these failures in under 5 lines. Cite file:line where possible."
timeout_secs = 60
```

```toml
# Just grab the first few error lines — no LLM needed.
[on_failure]
enabled = true
cmd = "sh -c 'grep -E \"(FAIL|panic|error)\" | head -5'"
```

Under `baraddur gate`, the hook timeout is clamped to 15 seconds so the
gate fails fast. See [One-shot mode](one-shot.md).
