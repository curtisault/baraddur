# One-shot mode

Beyond the watch loop, baraddur exposes two subcommands that run the
pipeline exactly once. Both read the same `.baraddur.toml`, honor
`if_changed` filtering and `{files}` substitution, and write
`.baraddur/last-run.log` — they're the watch-mode pipeline with a different
trigger and a scriptable exit code.

## `baraddur check`

Run the pipeline once and exit. Output is plain append-only — safe to pipe,
safe to use in CI:

```bash
baraddur check            # exit 0 on pass, 1 on any step failure, 2 on config error
baraddur check --no-hook  # skip [on_failure] even if enabled
```

## `baraddur gate <command…>`

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

## Git-aware filtering: `--staged` and `--since`

Both `check` and `gate` accept a git-aware path source. The paths feed the
existing `if_changed` filter and `{files}` substitution exactly like a
watch-mode file-change event:

```bash
baraddur check --staged                       # files staged for commit (git diff --cached)
baraddur check --since main                   # files changed since merge-base with main
baraddur gate --staged git commit -m "wip"    # only relevant steps run before the commit
```

`--since <ref>` includes untracked-but-not-ignored files so "what's new
since `base`" matches the working-tree view, not just what's committed.
`.gitignore`d paths are excluded.

With `--staged` and nothing staged, `gate` skips the pipeline entirely and
execs the wrapped command immediately — there's nothing to verify.

`--staged` and `--since` are mutually exclusive.

## Example: pre-commit hook

```bash
# .git/hooks/pre-commit
#!/bin/sh
exec baraddur check --staged
```
