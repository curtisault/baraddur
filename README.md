# Barad-dûr

<p align="center">
    <img src="barad-dur.svg" alt="Barad-dûr">
</p>

A project-agnostic file watcher that runs your check pipeline on every save
and surfaces failures before CI does. The same pipeline runs as a one-shot
(`baraddur check`) or as a gate around another command
(`baraddur gate git push`), so the rules you tune for save-time also gate
your commits and pushes.

```
━━━ #1 14:32:08 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓                                                   0.2s
▸ compile   ✓                                                   1.1s
▸ credo     ✗   3 issues                                        1.8s
▸ test      ✓                                                   2.3s

1 failed · 3 passed · 5.4s
```

## Install

```bash
cargo install baraddur
```

Pre-built binaries, source builds, and other platforms: see
[docs/install.md](docs/install.md).

## Quick start

```bash
baraddur init   # writes .baraddur.toml in the current directory
baraddur        # start watching
```

baraddur runs the pipeline immediately on launch, then re-runs it on every
file change. Steps are killed and restarted if a file changes mid-run.

## Documentation

| Topic | What it covers |
|---|---|
| [Install](docs/install.md) | crates.io, pre-built binaries, source builds |
| [Configuration](docs/configuration.md) | Schema, step filtering, examples, on-failure hook |
| [Watch mode](docs/watch-mode.md) | Browse mode, keybindings, in-place rendering |
| [One-shot mode](docs/one-shot.md) | `check`, `gate`, git-aware filtering, pre-commit |
| [CLI reference](docs/cli.md) | Flags, verbosity, output modes, log file |
| [Security](docs/security.md) | Trust model for `.baraddur.toml` |
| [JSON event stream](docs/json-events.md) | NDJSON output for integrations |
| [UX design](docs/ux-design.md) | Terminal UX states and transitions |

## Future ideas

- Homebrew tap
