# Barad-dûr — UX Design

> Companion to [`project-plan.md`](./project-plan.md). This document defines
> the user experience in detail: each state, the transitions between them,
> and the visual language that holds the design together.

---

## Design Principles

1. **Calm, not loud.** Most of the time, things are fine. The UI should fade
   into the background when nothing is wrong.
2. **Fail loudly, pass quietly.** Failures deserve real estate and color.
   Passes deserve a checkmark.
3. **Latest truth beats history.** On a new run, the old run is gone from
   view — but scrollback preserves it for those who want it.
4. **TTY and non-TTY are both first-class.** CI logs and interactive
   terminals get different output, both good.
5. **One screen, no scrolling.** If the output doesn't fit, truncate with a
   pointer to the full log, not force the user to scroll.

---

## Visual Language

### Glyphs

| Glyph | Meaning |
|---|---|
| `▸` | Step indicator (neutral) |
| `✓` | Passed |
| `✗` | Failed |
| `⟳` | Running |
| `⊘` | Skipped (upstream stage failed) |
| `·` | Queued (not yet started) |
| `━` | Run divider (full width) |
| `──` | Section divider (subordinate to run divider) |

### Colors

| Color | Use |
|---|---|
| Green | `✓` pass, success text |
| Red | `✗` fail, error text |
| Yellow | `⟳` running, warnings |
| Cyan | Section headers (`claude summary`, `credo output`) |
| Dim / gray | Metadata, skipped steps, footer, queued steps |

Respect `NO_COLOR`. Auto-disable color when stdout is not a TTY.

### Layout

- Step names left-aligned, padded to the longest name width
- Status glyph in a fixed column after the name
- Short diagnostic (e.g. `3 issues`, `cancelled`) in a dim column after status
- Duration (e.g. `1.8s`) right-aligned, dim
- Raw output blocks indented 2 spaces
- Timestamps in `HH:MM:SS`, dim

### Animation

- Spinner frames: `⠋⠙⠹⠸⠼⠴⠦⠧⠇⠏` (braille dots, 80ms per frame)
- Single shared frame clock — all `⟳` glyphs advance in sync
- Only in TTY mode

---

## States

### S1 — Startup

```
━━━ baraddur 0.1.0 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
watching: /Users/alice/code/my-project
config:   .baraddur.toml  (4 steps)
press ^C to exit
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

Shown once at launch. The initial run begins immediately after.

### S2 — Idle (watching, no active run)

```
━━━ 14:32:01 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓
▸ compile   ✓
▸ credo     ✓
▸ test      ✓

all passing · 142 files watched · last run 1.4s
```

Footer is dim. Persists on screen until the next file change.

### S3 — Running (sequential)

```
━━━ 14:32:08 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ⟳
▸ compile   ·
▸ credo     ·
▸ test      ·
```

Queued steps use `·` (dim). The status column updates in place as each step
starts and finishes.

### S4 — Running (parallel stage)

```
━━━ 14:32:08 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▸ compile   ✓   1.1s
▸ credo     ⟳
▸ test      ⟳
```

Both parallel steps show `⟳` concurrently. Each settles independently when
done.

### S5 — All passing (terminal state of a successful run)

Same shape as S2. Footer: `all passing · 142 files watched · last run 5.4s`.

### S6 — Failure

```
━━━ 14:32:08 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▸ compile   ✓   1.1s
▸ credo     ✗   3 issues   1.8s
▸ test      ✓   2.3s

── credo output ────────────────────────────────────────────────────
  lib/foo.ex:42:3 [C] Modules should have a @moduledoc tag.
  lib/foo.ex:58:5 [R] Function is too complex (cyclomatic: 11).
  lib/bar.ex:17:1 [D] TODO comment found.

── summary ─────────────────────────────────────────────────────────
3 Credo issues in 2 files. lib/foo.ex needs a @moduledoc and has an
over-complex function at line 58. lib/bar.ex has a lingering TODO.

1 failed · 3 passed · 5.4s
```

- Failing step's short diagnostic and duration are inline with the step line
- Raw output block follows
- LLM summary (if enabled, available, and non-timeout) follows below a cyan
  divider
- Footer summarizes the run

### S7 — Skipped steps (upstream failure)

```
━━━ 14:32:08 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▸ compile   ✗   warnings as errors   0.9s
▸ credo     ⊘   skipped
▸ test      ⊘   skipped

── compile output ──────────────────────────────────────────────────
  ...
```

Skipped steps are rendered dim with the `⊘` glyph. No output block, no
summary, no duration.

### S8 — Cancelled mid-run (restart)

```
━━━ 14:32:08 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
▸ format    ✓   0.2s
▸ compile   ⟳   cancelled — restarting...
```

Transient state shown briefly while the cancelled run winds down. The screen
then clears and a fresh run begins at S3.

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
▸ credo     ✗   command not found: `mix`   0.0s
```

**E4 — watcher died**
```
baraddur: file watcher stopped unexpectedly
          error: ...
          exiting. restart baraddur to continue watching.
```

**E5 — LLM command not available or timed out**

Silent skip. The raw output block appears; the summary section is omitted. A
dim note appears in the footer: `summary skipped (timeout)` or
`summary skipped (command not found: claude)`.

### S10 — Shutdown (Ctrl+C)

```
━━━ 14:34:22 ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
^C received. stopping watcher, killing child processes... done.
```

Exits cleanly with code 0. A second Ctrl+C within 2s force-exits with code
130.

---

## State Transitions

```
         ┌─────────────┐
         │ S1 Startup  │
         └──────┬──────┘
                │ initial run
                ▼
   ┌─────────────────────────────┐
   │   S3 / S4 Running           │◄────────────────┐
   └──┬──────────────────────────┘                 │
      │                                             │
      │ pipeline complete                           │ file change
      ▼                                             │ during run
   ┌─────────────┐                                  │ → S8 cancel
   │  S5 Pass    │◄──┐                              │ → restart
   └──────┬──────┘   │                              │
          │          │ file change                  │
          ▼          │                              │
   ┌─────────────┐   │                              │
   │  S2 Idle    │───┘                              │
   └──────┬──────┘                                  │
          │                                          │
          │ file change                              │
          └──────────────────────────────────────────┘

   Failure path: S3/S4 → (step fails) → remaining steps → S7 → S6 → S2
   Shutdown:     any state → S10 → exit
```

---

## Output Modes

### TTY mode (default)

Everything above. Full-block redraw with cursor movement, colors, spinner
animation, screen clearing.

### Non-TTY mode (piped, CI, no terminal)

Line-oriented append-only output. No cursor movement, no clearing, no
animation, no color (unless `--force-color`).

```
[14:32:08] run started
[14:32:08] format: pass (0.2s)
[14:32:09] compile: pass (1.1s)
[14:32:11] credo: FAIL (3 issues, 1.8s)
[14:32:11] test: pass (2.3s)
[14:32:11] --- credo output ---
lib/foo.ex:42:3 [C] Modules should have a @moduledoc tag.
...
[14:32:11] --- summary ---
3 Credo issues in 2 files. ...
[14:32:11] run complete: 1 failed, 3 passed, 5.4s
```

Detection: `std::io::IsTerminal::is_terminal()` on stdout.
Override: `--no-tty` to force append-only mode even on a TTY (useful for
debugging or recording).

### Verbosity Gradient

| Level | Flag | Behavior |
|---|---|---|
| Quiet | `-q` | Only show failures. No footer. No idle state display. |
| Default | *(none)* | Step list with glyphs; raw output and summary on failure. |
| Verbose | `-v` | Also show stdout of passing steps, indented under the step line. |
| Debug | `-vv` | Also show internal events (file changes, stage transitions, cancel/restart). |

`-q` and `-v` are mutually exclusive (last one wins).

---

## Output Truncation

Step output can be enormous. Defaults:

- Cap each step's captured output at **100 KiB**
- If exceeded on display, show first 50 lines + last 50 lines separated by
  `... [N lines elided — see .baraddur/last-run.log] ...`
- Always write the full captured output to `.baraddur/last-run.log`
- LLM summarizer receives the **full** 100 KiB (not the truncated display
  version), so summaries see the complete failure

---

## Open Questions (UX)

- Spinner behavior when non-TTY — omit entirely, or emit a periodic heartbeat
  line every N seconds for long-running steps?
- Should the LLM summary be **streamed** (appearing as it writes) or
  **rendered atomically** when complete? Streaming is nicer but conflicts
  with the redraw-on-state-change model.
- Should `.baraddur/last-run.log` be opt-in, or written by default? (If
  default, mention in the README that it can be `.gitignore`'d.)
- Is there a place for a persistent bottom status line (watching N files ·
  last run 3s ago · mode: elixir) or does it clutter more than it helps?
