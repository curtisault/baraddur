# Watch mode

Running `baraddur` with no subcommand starts the file watcher. The pipeline
runs immediately on launch, then re-runs on every file change. Steps are
killed and restarted if a file changes mid-run.

The run divider turns green when all steps pass and red when any fail. On
file-change restarts it also shows which file triggered the run:

```
━━━ #2 14:33:01  ·  lib/foo.ex ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

After the run completes, baraddur enters browse mode — navigate the step
list and expand output inline:

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

## Browse mode keybindings

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

Failing steps start with their output expanded. Save a file to exit browse
mode and rerun the pipeline immediately.

For a deeper account of every visual state and transition (spinner, color,
viewport behavior, raw-mode handling), see [UX design](ux-design.md).
