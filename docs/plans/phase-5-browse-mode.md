# Browse Mode: Interactive Post-Run Navigation

## Status: Implemented

---

## What Was Built

After each pipeline run, baraddur enters an interactive browse mode that lets the user navigate the step list, expand/collapse output per step, and quit.

**Keybindings:**
| Key | Action |
|---|---|
| `j` / `↓` | move cursor down |
| `k` / `↑` | move cursor up |
| `gg` | jump to first step |
| `G` | jump to last step |
| `Enter` / `o` | toggle output for selected step |
| `O` | expand all / collapse all (toggle) |
| `q` | quit baraddur |

A help bar listing these bindings is shown at the bottom of the screen while browse mode is active. It disappears when browse exits (file change or quit).

---

## Architecture

**Event loop stays in `lib.rs`; display methods are synchronous.**

`Display` is a `dyn` trait object. Async methods on a trait require `async-trait` or `Pin<Box<dyn Future>>` — both add ceremony that contradicts the existing pattern. Instead:
- `fn handle_key(&mut self, key: KeyEvent) -> BrowseAction` (synchronous, defaulted no-op)
- `fn enter_browse_mode(&mut self)` / `fn exit_browse_mode(&mut self)` (defaulted no-ops)
- `fn browse_redraw_if_active(&mut self)` (defaulted no-op)
- The async machinery (`tokio::select!`, `spawn_blocking`, file-change channel) lives in `lib.rs`

Keyboard events are read via `tokio::task::spawn_blocking(|| crossterm::event::read())` — no new async crate needed. Orphaned `spawn_blocking` threads after a file-change interrupt are safe: the blocking `read()` unblocks on the next keypress and the result is dropped.

---

## Key Decisions and Deviations from Original Plan

### `q` quits the app, not just browse mode

The original plan had `q` exit browse mode (leaving baraddur alive, waiting for the next file change). This was changed: `q` now quits baraddur entirely, which is simpler and matches user expectation. `BrowseAction::ExitBrowse` was removed; `BrowseAction::Quit` replaced it.

### Output blocks removed from `run_finished`

The original plan kept the failure output block printed below the step list in `run_finished`. This duplicated the output that browse mode shows inline. The output blocks were removed from `TtyDisplay::run_finished` entirely — browse mode owns all output display in TTY contexts. `PlainDisplay` (non-TTY/CI) is unaffected and still prints failure output blocks.

### `rendered_lines` accumulates through `run_finished`

Originally `run_finished` reset `rendered_lines = 0`. This caused browse mode to append the step list below the footer rather than replacing the whole block. Fix: `run_finished` no longer resets `rendered_lines` — it adds the blank line and footer to the running count so `browse_redraw`'s `MoveUp(rendered_lines)` sweeps up the step list and footer together.

### `cfmakeraw()` side-effects require explicit remediation

`crossterm::terminal::enable_raw_mode()` calls `cfmakeraw()` internally, which disables two terminal flags:
- **`OPOST`** — output post-processing; without it `\n` no longer generates `\r\n`, breaking `println!` with a staircase effect
- **`ISIG`** — signal generation; without it `Ctrl+C` no longer generates `SIGINT`, so the `tokio::signal::ctrl_c()` arm never fires

After calling `enable_raw_mode()`, `raw_mode_on()` immediately re-enables both via `libc::tcsetattr`:
```rust
t.c_oflag |= libc::OPOST;
t.c_lflag |= libc::ISIG;
```
This lets crossterm's internal state tracking remain satisfied while restoring the two flags it shouldn't have cleared.

### Visual row counting for accurate `MoveUp`

Long output lines (e.g. a compiler error spanning more than one terminal column-width) wrap to multiple visual rows. Counting them as 1 logical line made `MoveUp(rendered_lines)` fall short, leaving orphaned rows that accumulated on each expand/collapse. Fix: `visual_rows_for(text, width)` uses ceiling division `(visible_len + width - 1) / width`.

### crossterm feature flag

The correct feature name is `"events"` (plural), not `"event"` as originally written in the plan.

---

## Files Modified

| File | Change |
|---|---|
| `Cargo.toml` | Added `features = ["events"]` to crossterm |
| `src/output/mod.rs` | Added `BrowseAction` enum; added `enter_browse_mode`, `exit_browse_mode`, `handle_key`, `browse_redraw_if_active` defaulted trait methods |
| `src/output/style.rs` | Added `Theme::selected` (reverse video highlight) |
| `src/output/display.rs` | `format_truncated_output` refactor; browse state fields; `raw_mode_on/off`; `browse_redraw`; `visual_rows_for`; `handle_key`; `enter/exit_browse_mode`; `run_finished` simplified (no output blocks); `Drop` updated |
| `src/lib.rs` | Added `next_key_event()`; replaced idle wait with browse loop |
| `justfile` | Added `install` recipe |
