# Barad-dûr Documentation

This directory contains the user-facing documentation for Barad-dûr. The
top-level [README](../README.md) covers the elevator pitch and quick start;
everything else lives here.

## User guides

| Page | Purpose |
|---|---|
| [Install](install.md) | How to get baraddur on your machine — crates.io, pre-built tarballs, source. |
| [Configuration](configuration.md) | The `.baraddur.toml` schema, path-based step filtering, stack examples, post-failure hook. |
| [Watch mode](watch-mode.md) | The default interactive mode — browse, navigate, rerun. |
| [One-shot mode](one-shot.md) | `baraddur check` and `baraddur gate`, plus git-aware filtering (`--staged`, `--since`). |
| [CLI reference](cli.md) | All flags, verbosity levels, output modes, and the on-disk log file. |
| [Security](security.md) | Trust model for `.baraddur.toml` and walk-up discovery. |

## Integration references

| Page | Purpose |
|---|---|
| [JSON event stream](json-events.md) | NDJSON output via `--format json` for editor and CI integrations. |
| [UX design](ux-design.md) | The terminal UX as implemented: states, transitions, visual language. |

## Project meta

| Page | Purpose |
|---|---|
| [Governance](governance.md) | crates.io and Homebrew compliance checklist (auto-generated). |
