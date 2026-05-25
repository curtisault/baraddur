# Security

`.baraddur.toml` is **executable trust**: every `cmd` you list runs as your
user on every file change. Treat the file the same way you'd treat a
`Makefile`, a `justfile`, or a shell script — review it before running
baraddur in a directory you don't fully control.

Two specifics worth knowing:

- **Walk-up discovery.** Like `git` and `.gitignore`, baraddur searches
  upward from `cwd` for a `.baraddur.toml`. A config dropped in any ancestor
  directory will be picked up automatically. After a fresh `git clone` of an
  unfamiliar project, `cat .baraddur.toml` before running.
- **Banner confirms which file loaded.** On every start, baraddur prints the
  resolved config path. If it points somewhere you didn't expect, exit and
  investigate.

To pin to a specific file and disable walk-up discovery, pass `-c`:

```bash
baraddur -c ./.baraddur.toml
```
