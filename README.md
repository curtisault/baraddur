# Barad-dûr

<p align="center">
    <img src="barad-dur.svg" alt="Barad-dûr"
</p>

A project-agnostic file watcher that runs your check pipeline on every save and
surfaces failures before CI does. The same pipeline also runs as a one-shot
(`baraddur check`) or as a gate around another command (`baraddur gate git push`),
so the rules you tune for save-time also gate your commits and pushes.

```
━━━ #1 14:32:08 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓                                                   0.2s
▸ compile   ✓                                                   1.1s
▸ credo     ✗   3 issues                                        1.8s
▸ test      ✓                                                   2.3s

1 failed · 3 passed · 5.4s
```

The divider turns green when all steps pass and red when any fail. On file-change
restarts it also shows which file triggered the run:

```
━━━ #2 14:33:01  ·  lib/foo.ex ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

After the run completes, baraddur enters browse mode — navigate the step list
and expand output inline:

```
▸ format    ✓                                                   0.2s
▸ compile   ✓                                                   1.1s
▶ credo     ✗   3 issues                                        1.8s
  lib/foo.ex:42:3 [C] Modules should have a @moduledoc tag.
  lib/foo.ex:58:5 [R] Function is too complex (cyclomatic: 11).
  lib/bar.ex:17:1 [D] TODO comment found.
▸ test      ✓                                                   2.3s

  j/k ↑/↓  navigate · Enter/o  toggle output · O  expand all · q  quit
```

## Install

### From crates.io

```bash
cargo install baraddur
```

### Pre-built binaries

Download the tarball for your platform from the [latest release](https://github.com/curtisault/baraddur/releases/latest) and place `baraddur` somewhere on your `$PATH`. Supported targets:

| Platform | Target triple |
|---|---|
| macOS (Apple Silicon) | `aarch64-apple-darwin` |
| macOS (Intel) | `x86_64-apple-darwin` |
| Linux (x86_64) | `x86_64-unknown-linux-gnu` |
| Linux (aarch64) | `aarch64-unknown-linux-gnu` |

Each tarball ships with `README.md`, `LICENSE-MIT`, and `LICENSE-APACHE` alongside the binary, and a `.sha256` checksum file is attached to the release.

### Homebrew

Coming soon — tap not yet published.

### From source

Requires Rust 1.85 or newer.

```bash
just install
# or manually:
cargo build --release && cp ./target/release/baraddur ~/.local/bin/baraddur
```

## Quick start

Scaffold a starter config and run:

```bash
baraddur init   # writes .baraddur.toml in the current directory
baraddur        # start watching
```

See [Config examples](#examples) below for common stacks.

baraddur runs the pipeline immediately on launch, then re-runs it on every file
change. Steps are killed and restarted if a file changes mid-run.

## Browse mode

After each run, baraddur enters an interactive browse mode:

| Key | Action |
|---|---|
| `j` / `↓` | move cursor down |
| `k` / `↑` | move cursor up |
| `gg` | jump to first step |
| `G` | jump to last step |
| `Enter` / `o` | toggle output for selected step |
| `O` | expand all / collapse all |
| `r` | rerun the full pipeline |
| `f` | rerun only steps that failed last run (no-op if none failed) |
| `q` | quit baraddur |

Failing steps start with their output expanded. Save a file to exit browse mode
and rerun the pipeline immediately.

## One-shot mode

Beyond the watch loop, baraddur exposes two subcommands that run the pipeline
exactly once. Both read the same `.baraddur.toml`, honor `if_changed`
filtering and `{files}` substitution, and write `.baraddur/last-run.log` —
they're the watch-mode pipeline with a different trigger and a scriptable
exit code.

### `baraddur check`

Run the pipeline once and exit. Output is plain append-only — safe to pipe,
safe to use in CI:

```bash
baraddur check            # exit 0 on pass, 1 on any step failure, 2 on config error
baraddur check --no-hook  # skip [on_failure] even if enabled
```

### `baraddur gate <command…>`

Run the pipeline first; on success, `exec` the wrapped command. Pipeline
failure exits 1 without invoking the wrapped command. On Unix the wrapped
process replaces baraddur, so its exit code propagates verbatim. Everything
after `gate` is captured as the wrapped command — no `--` separator needed.

```bash
baraddur gate git push origin main
baraddur gate cargo publish
```

The `[on_failure]` hook timeout is clamped to 15 seconds under `gate` so the
gate fails fast.

### Git-aware filtering: `--staged` and `--since`

Both `check` and `gate` accept a git-aware path source. The paths feed the
existing `if_changed` filter and `{files}` substitution exactly like a
watch-mode file-change event:

```bash
baraddur check --staged                       # files staged for commit (git diff --cached)
baraddur check --since main                   # files changed since merge-base with main
baraddur gate --staged git commit -m "wip"    # only relevant steps run before the commit
```

`--since <ref>` includes untracked-but-not-ignored files so "what's new since
`base`" matches the working-tree view, not just what's committed. `.gitignore`d
paths are excluded.

With `--staged` and nothing staged, `gate` skips the pipeline entirely and
execs the wrapped command immediately — there's nothing to verify.

### Example: pre-commit hook

```bash
# .git/hooks/pre-commit
#!/bin/sh
exec baraddur check --staged
```

## Config

Config is discovered by walking up from the current directory (like `.gitignore`).
A global fallback lives at `~/.config/baraddur/config.toml`.

### Full schema

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

### Path-based step filtering

Each step may declare `if_changed`, a list of glob patterns matched against
paths reported by the file watcher. When set:

- **File-change runs**: the step runs only if at least one changed path matches
  a pattern. Steps with no matches are excluded from the run entirely (they
  don't appear in the step list).
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

Patterns use [`globset`](https://docs.rs/globset) syntax (gitignore-style globs
with `**` and `*`).

### Examples

<details>
<summary>Rust / Cargo</summary>

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

</details>

<details>
<summary>TypeScript / Node.js</summary>

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

All three steps run concurrently as a single stage. Swap in `eslint`, `prettier`,
or any other tool you prefer.

</details>

<details>
<summary>Elixir / Mix</summary>

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

</details>

### Parallel steps

Consecutive `parallel = true` steps run as a batch — all start at once, all
must complete before the next stage begins. `parallel = false` steps always run
alone and gate everything after them.

```
stage 1: [format]         # parallel=false — must pass
stage 2: [compile]        # parallel=false — must pass
stage 3: [credo, test]    # parallel=true  — run concurrently
```

If any stage fails, subsequent stages are skipped.

### Command parsing

`cmd` strings are split with POSIX shell-word rules (`shell-words` crate). Shell
features like pipes, `&&`, and glob expansion are not supported. For those, use
`sh -c`:

```toml
cmd = "sh -c 'mix compile 2>&1 | head -50'"
```

### Post-failure hook

When `[on_failure].enabled = true` and any step in a completed run fails, the
configured `cmd` is spawned with the combined stdout+stderr of failing steps on
stdin. `prompt` (if non-empty) is prepended as a preamble. The captured stdout
is shown below the failure summary; non-zero exits, empty stdout, and timeouts
are silently suppressed (with a stderr diagnostic).

The hook runs asynchronously — your failure output is shown immediately and the
hook output slots in when ready. A file change cancels the in-flight hook and
kills the subprocess.

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

## Security

`.baraddur.toml` is **executable trust**: every `cmd` you list runs as your user
on every file change. Treat the file the same way you'd treat a `Makefile`, a
`justfile`, or a shell script — review it before running baraddur in a
directory you don't fully control.

Two specifics worth knowing:

- **Walk-up discovery.** Like `git` and `.gitignore`, baraddur searches upward
  from `cwd` for a `.baraddur.toml`. A config dropped in any ancestor directory
  will be picked up automatically. After a fresh `git clone` of an unfamiliar
  project, `cat .baraddur.toml` before running.
- **Banner confirms which file loaded.** On every start, baraddur prints the
  resolved config path. If it points somewhere you didn't expect, exit and
  investigate.

To pin to a specific file and disable walk-up discovery, pass `-c`:

```bash
baraddur -c ./.baraddur.toml
```

## CLI flags

```
baraddur [OPTIONS] [COMMAND]

Commands:
  init     Scaffold a starter .baraddur.toml in the current directory
  check    Run the pipeline once and exit (0 pass / 1 fail / 2 config error)
  gate     Run the pipeline; on success exec the wrapped command

Subcommand options (check, gate):
      --staged          Restrict to files staged for commit
      --since <REF>     Restrict to files changed since <REF> (merge-base; adds untracked-not-ignored)
      --no-hook         Skip the [on_failure] hook

Global options:
  -c, --config <FILE>     Config file (disables walk-up discovery)
  -w, --watch-dir <DIR>   Directory to watch [default: config file's directory]
      --no-tty            Force plain append-only output
      --no-clear          Don't clear the screen between runs
  -v, --verbose           Show output from passing steps (-vv for debug events)
  -q, --quiet             Only show failures
  -h, --help
  -V, --version
```

`--staged` and `--since` are mutually exclusive.

### Verbosity

| Flag | Behavior |
|---|---|
| `-q` | Silence everything except failures |
| *(default)* | Step list with pass/fail glyphs; expand output in browse mode |
| `-v` | Also stream stdout/stderr from passing steps (non-TTY/piped mode only) |
| `-vv` | Also print internal debug events to stderr |

### Output modes

In a terminal, baraddur redraws the step block in place with colors, a braille
spinner, and interactive browse mode after each run. When stdout is not a
terminal (piped, CI), it falls back to plain append-only lines with timestamps:

```
[14:32:08] run #1 started
[14:32:08] ▸ format running
[14:32:08] ▸ format  ✓  (0.2s)
[14:32:09] ▸ compile  ✓  (1.1s)
[14:32:11] ▸ credo  ✗  (1.8s)
--- credo output ---
  lib/foo.ex:42:3 [C] Modules should have a @moduledoc tag.
[14:32:11] run complete: 1 failed, 3 passed, 5.4s
```

Force plain mode with `--no-tty`. Disable colors without touching TTY detection
by setting `NO_COLOR=1`.

## Output log

After each run, full step output is written to `.baraddur/last-run.log` relative
to the watch root. Add it to your `.gitignore`:

```
.baraddur/
```

On screen, output longer than 50 lines is truncated to the first 25 and last 25
lines with an elision marker pointing to the log file.

## Future ideas

- Homebrew tap
