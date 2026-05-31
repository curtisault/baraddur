default: test

build:
    cargo build

release:
    cargo build --release
    @just _sign ./target/release/baraddur

install: release
    cp ./target/release/baraddur ~/.local/bin/baraddur
    @just _sign ~/.local/bin/baraddur

# macOS AMFI caches code signatures by path; an in-place `cp` (or even a fresh
# cargo build over an existing artifact) can leave AMFI rejecting the new file
# with SIGKILL even though it's a valid ad-hoc-signed Mach-O. Force a fresh
# ad-hoc signature so the kernel re-evaluates on next exec. No-op on non-macOS.
_sign path:
    @if [ "$(uname)" = "Darwin" ]; then codesign --force --sign - {{path}}; fi

test:
    cargo test

check:
    cargo check

lint:
    cargo clippy --all-targets -- -D warnings

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

ci: fmt-check lint crap

# Regenerate docs/crap-and-code-cov.md from current coverage + complexity.
# Requires `cargo-llvm-cov` and `cargo-crap` to be installed locally. Wired
# into `ci` so the tracked snapshot stays in sync with each green run.
# `cargo llvm-cov` runs the test suite under instrumentation and propagates
# any test failure, so `ci` doesn't need a separate `test` step.
crap:
    cargo llvm-cov --lcov --output-path lcov.info
    ./scripts/crap.sh

# Audit GitHub Action SHA pins against current upstream.
check-pins:
    ./scripts/check-action-pins.sh

# Update drifted GitHub Action SHA pins in place; review with `git diff`.
update-pins:
    ./scripts/check-action-pins.sh --update

run *args:
    cargo run -- {{args}}

# Step-level results from `check --format json`, filtered to {name, success, ms}.
# Pass extra args through (e.g. `just json-steps --profile quick`).
json-steps *args:
    cargo run --quiet -- check --format json {{args}} | jq -c 'select(.event == "step_finished") | {name, success, ms: .duration_ms}'

# Tail run-level events in watch mode (run_started / run_cancelled / run_finished).
# Ctrl-C to exit. Edit a watched file to see a new run.
json-watch *args:
    cargo run --quiet -- --format json {{args}} | jq -c 'select(.event | test("^run_"))'

clean:
    cargo clean
