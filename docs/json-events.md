# JSON Event Stream

`baraddur --format json` emits a stream of newline-delimited JSON (NDJSON)
events to stdout — one JSON object per line. Designed for editor integrations,
CI harnesses, and custom dashboards that need a stable machine-readable view
of pipeline lifecycle.

## Stability contract

- **Additive changes are non-breaking.** New fields may be added to existing
  events, and new event types may be introduced at any time. Parsers must
  ignore unknown fields and unknown `event` values rather than fail.
- **Renames and removals are breaking.** Field names and event names are
  locked in v1; any change here is a major version bump.
- The `--format json` flag is mutually exclusive with `--no-tty`. JSON output
  is non-interactive by definition (no spinner, no browse mode); under
  `watch`, the stream continues until SIGINT.

## Common fields

Every event has:

| Field   | Type     | Meaning                                                            |
| ------- | -------- | ------------------------------------------------------------------ |
| `ts`    | string   | RFC 3339 / ISO 8601 with millisecond precision, UTC (`...Z`).       |
| `event` | string   | The event name. See sections below.                                 |

## Events

### `run_started`

A new pipeline run is about to execute.

```json
{"ts":"2026-05-24T14:32:01.123Z","event":"run_started","run":1,"steps":["fmt","check"]}
```

| Field     | Type            | Notes                                                    |
| --------- | --------------- | -------------------------------------------------------- |
| `run`     | integer         | 1-based counter, monotonic across the process lifetime.   |
| `steps`   | array<string>   | Step names in their configured (declaration) order.       |
| `trigger` | array<string>?  | Paths that triggered this run (file-change or `--staged` / `--since`). Omitted on the initial run. |

### `step_running`

A single step has started executing.

```json
{"ts":"...","event":"step_running","name":"fmt"}
```

| Field  | Type   | Notes               |
| ------ | ------ | ------------------- |
| `name` | string | The step's name.    |

### `step_finished`

A step has completed (pass or fail).

```json
{"ts":"...","event":"step_finished","name":"fmt","success":true,"exit_code":0,
 "duration_ms":340,"stdout":"…","stdout_truncated":false,
 "stderr":"","stderr_truncated":false}
```

| Field              | Type              | Notes                                                                                          |
| ------------------ | ----------------- | ---------------------------------------------------------------------------------------------- |
| `name`             | string            | The step's name.                                                                                |
| `success`          | bool              | True iff the process exited 0.                                                                  |
| `exit_code`        | integer \| null   | `null` when the process was killed by a signal or failed to launch.                             |
| `duration_ms`      | integer           | Wall-clock duration in milliseconds.                                                            |
| `stdout`           | string            | Captured stdout, ANSI escapes stripped, clamped to 100 KiB.                                     |
| `stdout_truncated` | bool              | True when `stdout` was clamped to the size cap.                                                  |
| `stderr`           | string            | Captured stderr, ANSI escapes stripped, clamped to 100 KiB.                                     |
| `stderr_truncated` | bool              | True when `stderr` was clamped to the size cap.                                                  |

### `step_skipped`

A step was skipped because an earlier stage failed.

```json
{"ts":"...","event":"step_skipped","name":"test"}
```

| Field  | Type   | Notes               |
| ------ | ------ | ------------------- |
| `name` | string | The step's name.    |

### `run_cancelled`

A run was interrupted mid-flight (file change triggered restart). No
`run_finished` event will follow for this run; the next event will be the
next run's `run_started`.

```json
{"ts":"...","event":"run_cancelled","run":1}
```

| Field | Type    | Notes                            |
| ----- | ------- | -------------------------------- |
| `run` | integer | The run number that was cancelled. |

### `run_finished`

A run completed normally.

```json
{"ts":"...","event":"run_finished","run":1,"passed":2,"failed":0,
 "skipped":0,"duration_ms":1820}
```

| Field         | Type    | Notes                                                                 |
| ------------- | ------- | --------------------------------------------------------------------- |
| `run`         | integer | The run number that finished.                                         |
| `passed`      | integer | Number of steps that succeeded.                                       |
| `failed`      | integer | Number of steps that failed.                                          |
| `skipped`     | integer | Number of `step_skipped` events emitted earlier in this run.          |
| `duration_ms` | integer | Wall-clock duration from `run_started` to `run_finished`, in ms.      |

## Event ordering

For a single run, the events appear in this order:

```
run_started
  step_running (× N or fewer)
  step_finished (× N or fewer)
  step_skipped (× any number, only after a stage failure)
run_finished | run_cancelled
```

Parallel stages may interleave `step_running` and `step_finished` events
across multiple steps. Use `name` to correlate.

## Parsing tips

- Pipe directly to `jq`:
  `baraddur check --format json | jq -c 'select(.event == "step_finished")'`
- Treat unknown event types and unknown fields as no-ops to stay
  forward-compatible with future versions.
- The stream is unbuffered (each event is flushed immediately), so consumers
  can react incrementally.
