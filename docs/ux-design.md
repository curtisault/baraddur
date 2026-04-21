# Barad-dûr — UX Design

This document describes the terminal UX as implemented: every state, the
transitions between them, and the visual language that holds it together.

---

## Design Principles

1. **Calm, not loud.** Most of the time, things are fine. The UI fades into
   the background when nothing is wrong.
2. **Fail loudly, pass quietly.** Failures get real estate and color. Passes
   get a checkmark.
3. **Latest truth beats history.** On a new run, the old run is gone from
   view — but scrollback preserves it.
4. **TTY and non-TTY are both first-class.** CI logs and interactive terminals
   get different output, both correct.
5. **One screen, no scrolling.** The viewport clips to terminal height and
   auto-scrolls to keep the cursor step visible during browse.

---

## Visual Language

### Glyphs

| Glyph | Meaning |
|---|---|
| `▸` | Step prefix (neutral) |
| `▶` | Cursor indicator (no-color mode only; replaces `▸` on selected row) |
| `✓` | Passed |
| `✗` | Failed |
| `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` | Running (braille spinner, 10 frames) |
| `⊘` | Skipped (upstream stage failed) |
| `·` | Queued (not yet started) |
| `━` | Run divider (full width) |

### Colors

| Color | Use |
|---|---|
| Green (bold) | `✓` pass glyph; run divider when all steps pass |
| Red (bold) | `✗` fail glyph; run divider when any step fails; "N failed" in footer |
| Yellow | Spinner frame while running |
| Reverse-video | Cursor row highlight in browse mode (color mode only) |
| Dim / gray | Durations, short diagnostics, skipped/queued glyphs, footer, help bar |

`NO_COLOR` suppresses all color and reverse-video. Color is also
auto-disabled when stdout is not a TTY.

### Layout

- Step names left-aligned, padded to the longest name in the run
- Status glyph in a fixed column after the name
- Duration (e.g. `1.8s`) right-aligned after the status glyph
- Short diagnostic (e.g. `cannot find function`, `3 lines`) dim, after the
  duration, only on failed steps
- Expanded output indented 2 spaces, below the step row
- Timestamps in `HH:MM:SS`; run count as `#N`

### Spinner Animation

Frames: `⠋ ⠙ ⠹ ⠸ ⼴ ⠦ ⠧ ⠇ ⠏` — 10 frames, 80 ms per frame.
All `⟳`-equivalent running steps share a single frame clock and advance in
sync. Animation runs only in TTY mode.

---

## States

### S1 — Startup

```
━━━ baraddur 0.1.0 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
watching:     /Users/alice/code/my-project
config:       .baraddur.toml  (4 steps)
press ^C to exit
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Shown once at launch. The first run begins immediately after.

### S2 — Idle (watching, no active run)

After a successful run, browse mode (S11) holds the screen. After a failed
run, browse mode also holds. The idle state is only visible in non-TTY mode,
where no browse mode is entered and the terminal simply awaits the next file
change with no persistent display.

### S3 — Running (sequential)

```
━━━ #2 14:32:08  ·  3 files ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ⠙
▸ compile   ·
▸ credo     ·
▸ test      ·
```

The run divider is dim while any step is still running or queued. Queued
steps show `·` (dim). The spinner advances in-place at 80 ms intervals.
Completed steps show their glyph and duration immediately.

### S4 — Running (parallel stage)

```
━━━ #2 14:32:08  ·  3 files ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▸ compile   ✓   1.1s
▸ credo     ⠸
▸ test      ⠸
```

Both parallel steps show the same spinner frame concurrently. Each settles
independently — its row updates in place when it finishes.

### S5 — All passing (run complete)

The run divider turns green. The display immediately transitions into Browse
Mode (S11) with the cursor on row 0 and no output expanded:

```
━━━ #2 14:32:08  ·  3 files ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▶ format    ✓   0.2s
▸ compile   ✓   1.1s
▸ credo     ✓   1.8s
▸ test      ✓   2.3s

4 passed · 5.4s

  j/k ↑/↓  navigate · Enter/o  toggle output · O  expand all · q  quit
```

The user can navigate and expand any step's output. On file change, browse
exits and a new run starts.

### S6 — Failure (TTY)

The run divider turns red. The display immediately transitions into Browse
Mode (S11) with the first failing step pre-selected and its output
pre-expanded:

```
━━━ #2 14:32:08  ·  3 files ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▸ compile   ✓   1.1s
▶ credo     ✗   1.8s   3 lines
  lib/foo.ex:42:3 [C] Modules should have a @moduledoc tag.
  lib/foo.ex:58:5 [R] Function is too complex (cyclomatic: 11).
  lib/bar.ex:17:1 [D] TODO comment found.
▸ test      ✓   2.3s

1 failed · 3 passed · 5.4s

  j/k ↑/↓  navigate · Enter/o  toggle output · O  expand all · q  quit
```

- Short diagnostic on the step row shows the first non-empty output line
  (truncated to 40 chars) or `N lines` if output spans multiple lines
- Output expands inline below the step row, indented 2 spaces
- Footer summarizes the run; help bar appears below
- On file change: browse exits, new run starts
- `q` quits baraddur entirely

### S6 — Failure (non-TTY / CI)

Append-only timestamped output, no browse mode:

```
[14:32:08] run #2 started (3 files changed)
[14:32:08] format: pass (0.2s)
[14:32:09] compile: pass (1.1s)
[14:32:11] credo: FAIL (1.8s)
  lib/foo.ex:42:3 [C] Modules should have a @moduledoc tag.
  lib/foo.ex:58:5 [R] Function is too complex (cyclomatic: 11).
  lib/bar.ex:17:1 [D] TODO comment found.
[14:32:11] test: pass (2.3s)
[14:32:11] run complete: 1 failed · 3 passed · 5.4s
```

### S7 — Skipped steps (upstream failure)

When a stage fails, all subsequent stages are skipped. The step list in
browse mode (TTY) or static output (non-TTY) shows:

```
━━━ #2 14:32:08  ·  3 files ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▶ compile   ✗   0.9s   warnings as errors
  ...
▸ credo     ⊘
▸ test      ⊘

1 failed · 1 passed · 2 skipped · 0.9s

  j/k ↑/↓  navigate · Enter/o  toggle output · O  expand all · q  quit
```

Skipped steps show `⊘` (dim). No duration column for skipped steps.

### S8 — Cancelled mid-run (restart)

```
━━━ #2 14:32:08  ·  3 files ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▸ compile   cancelled — restarting...
```

Shown briefly while the cancelled run winds down. The screen then clears and
a fresh run begins at S3.

### S9 — Error states

**E1 — config not found**
```
baraddur: no .baraddur.toml found in this directory or any parent,
          and no ~/.config/baraddur/config.toml.

create a .baraddur.toml in your project root. minimal example:

  [watch]
  extensions = ["rs"]

  [[steps]]
  name = "check"
  cmd  = "cargo check"
```

**E2 — config invalid**
```
baraddur: config error in .baraddur.toml

  unknown field `parralel` in step "credo" (line 15)
  did you mean `parallel`?
```

**E3 — command not found**
```
▸ credo     ✗   0.0s   command not found
```

**E4 — watcher died**
```
baraddur: file watcher stopped unexpectedly
          error: ...
          exiting. restart baraddur to continue watching.
```

### S10 — Shutdown (Ctrl+C)

```
━━━ 14:34:22 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
^C received. stopping watcher, killing child processes... done.
```

Exits cleanly with code 0. A second Ctrl+C within 2 s force-exits with code
130.

### S11 — Browse Mode (TTY only, post-run)

Entered automatically after every pipeline run in TTY mode. A cursor
highlight and help bar appear; the user navigates steps and
expands/collapses output.

**Keybindings:**

| Key | Action |
|---|---|
| `j` / `↓` | Move cursor down |
| `k` / `↑` | Move cursor up |
| `gg` | Jump to first step |
| `G` | Jump to last step |
| `Enter` / `o` | Toggle output for selected step |
| `O` | Expand all / collapse all (toggle) |
| `q` | Quit baraddur entirely |

**Cursor highlight:** reverse-video on the `▸ name` row in color mode; `▶`
replaces `▸` as a fallback indicator when color is disabled.

**Viewport:** when the step list plus expanded output exceeds the terminal
height, the display clips to fit and auto-scrolls to keep the cursor row
visible. Partial-line splits at the clip boundary are avoided.

**Initial state on entry:**
- Failure run: first failing step is selected; its output is pre-expanded.
- Passing run: cursor on row 0; no output expanded.

**Exit conditions:**
- File change detected → browse exits, cursor/help bar clear, new run starts
  (S3).
- `q` pressed → baraddur shuts down (same as Ctrl+C).

**Terminal mode:** `enable_raw_mode()` is active during browse. `OPOST` and
`ISIG` are re-enabled immediately after crossterm's raw mode to keep
`println!` and Ctrl+C working. Raw mode is disabled and the cursor is
restored on exit.

---

## State Transitions

```
         ┌─────────────┐
         │ S1 Startup  │
         └──────┬──────┘
                │ initial run
                ▼
   ┌─────────────────────────────┐
   │   S3 / S4 Running           │◄──────────────────────────────┐
   └──┬──────────────────────────┘                               │
      │                                                          │
      │ pipeline complete                                        │ file change
      ▼                                                          │ during run
   ┌─────────────┐   ┌─────────────┐                            │ → S8 cancel
   │  S5 Pass    │   │   S6 Fail   │                            │ → restart
   └──────┬──────┘   └──────┬──────┘                            │
          │                 │                                    │
          └────────┬────────┘                                    │
                   │ TTY: enter browse (S11)                     │
                   ▼                                             │
        ┌──────────────────┐  file change / q                   │
        │  S11 Browse Mode │─────────────────────────────────► (restart / quit)
        └──────────────────┘

   Failure path: S3/S4 → (stage fails) → remaining stages skipped → S7
                 → S6 → TTY: S11 Browse

   non-TTY: no browse mode. S5/S6 print output and return to watching.

   Shutdown: any state → S10 → exit
             S11: q key → S10
```

---

## Output Modes

### TTY mode (default)

Full-block redraw with in-place cursor movement, colors, spinner animation,
and screen clearing between renders.

**Echo suppression during pipeline runs:** `ECHO` and `ECHOE` are cleared on
`TtyDisplay` construction so keystrokes typed while a pipeline runs do not
corrupt the step-status block. `ISIG` and `OPOST` are preserved. Settings
are restored on drop.

**Browse mode after each run:** see S11.

### Non-TTY mode (piped, CI, no terminal)

Line-oriented append-only output. No cursor movement, no clearing, no
animation, no color (unless `--force-color`), no browse mode.

Detection: `std::io::IsTerminal::is_terminal()` on stdout.  
Override: `--no-tty` forces append-only mode even on a TTY.

### Verbosity Gradient

| Level | Flag | Behavior |
|---|---|---|
| Quiet | `-q` | Failures only. Footer suppressed when all pass. |
| Normal | *(none)* | Step list with glyphs; failed step output on run complete. |
| Verbose | `-v` | Normal + stdout of passing steps (non-TTY: printed after run; TTY: available in browse). |
| Debug | `-vv` | Verbose + internal events logged to stderr (file changes, stage transitions, cancel/restart). |

`-q` and `-v`/`-vv` are mutually exclusive; last flag wins.

---

## Output Truncation

Step output can be large. The following limits apply:

- Each step's captured output is capped at **100 KiB**. Beyond that, capture
  stops and `... [output truncated at 100 KiB] ...` is appended.
- Display is further capped: if output exceeds **50 lines**, the first 25
  lines and last 25 lines are shown separated by
  `... [N lines elided — see .baraddur/last-run.log] ...`
- All captured output (up to 100 KiB) is written to `.baraddur/last-run.log`
  regardless of display truncation.
- Lines are prefixed with 2 spaces when rendered in the step list.
- Terminal wrapping is accounted for in viewport height calculations: a line
  that wraps at terminal width counts as multiple visual rows.
