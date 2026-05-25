# CLI reference

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

`--staged` and `--since` are mutually exclusive. See
[One-shot mode](one-shot.md) for how they interact with the pipeline.

## Verbosity

| Flag | Behavior |
|---|---|
| `-q` | Silence everything except failures |
| *(default)* | Step list with pass/fail glyphs; expand output in browse mode |
| `-v` | Also stream stdout/stderr from passing steps (non-TTY/piped mode only) |
| `-vv` | Also print internal debug events to stderr |

## Output modes

In a terminal, baraddur redraws the step block in place with colors, a
braille spinner, and interactive browse mode after each run. When stdout is
not a terminal (piped, CI), it falls back to plain append-only lines with
timestamps:

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

Force plain mode with `--no-tty`. Disable colors without touching TTY
detection by setting `NO_COLOR=1`.

For machine-readable output, see [JSON event stream](json-events.md).

## Output log

After each run, full step output is written to `.baraddur/last-run.log`
relative to the watch root. Add it to your `.gitignore`:

```
.baraddur/
```

On screen, output longer than 50 lines is truncated to the first 25 and last
25 lines with an elision marker pointing to the log file.
