default: test

# Idempotent: rustup components no-op if present, cargo-installed binaries are
# skipped when already on PATH, and `jq` (a system package used by json-steps/
# json-watch) only triggers a warning since cargo can't install it.
# Install every external tool the other recipes need (run once after cloning).
setup:
    @echo "▸ rustup components (rustfmt, clippy, llvm-tools-preview)"
    rustup component add rustfmt clippy llvm-tools-preview
    @echo "▸ cargo-llvm-cov (coverage → lcov.info for the crap recipe)"
    @cargo llvm-cov --version >/dev/null 2>&1 || cargo install --locked cargo-llvm-cov
    @echo "▸ cargo-crap (CRAP complexity gate)"
    @command -v cargo-crap >/dev/null 2>&1 || cargo install --locked cargo-crap
    @command -v jq >/dev/null 2>&1 || echo "⚠ jq not found — needed for json-steps/json-watch; install it via your system package manager"
    @echo "✓ setup complete — try 'just ci'"

# Lists any missing tool and exits non-zero without installing anything, so it
# can gate CI or a pre-flight check ('just setup' installs what's missing).
# Read-only check that every dev tool is present.
setup-check:
    @missing=0; \
    for c in rustfmt clippy; do \
        rustup component list --installed 2>/dev/null | grep -q "^$c" || { echo "✗ rustup component '$c' missing"; missing=1; }; \
    done; \
    rustup component list --installed 2>/dev/null | grep -q "^llvm-tools" || { echo "✗ rustup component 'llvm-tools-preview' missing"; missing=1; }; \
    cargo llvm-cov --version >/dev/null 2>&1 || { echo "✗ cargo-llvm-cov missing"; missing=1; }; \
    command -v cargo-crap >/dev/null 2>&1 || { echo "✗ cargo-crap missing"; missing=1; }; \
    command -v jq >/dev/null 2>&1 || { echo "✗ jq missing"; missing=1; }; \
    if [ "$missing" -eq 0 ]; then echo "✓ all dev tools present"; else echo "→ run 'just setup' to install the missing tools"; exit 1; fi

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

ci: fmt-check lint crap crap-gate

# Regenerate docs/crap-and-code-cov.md from current coverage + complexity.
# Requires `cargo-llvm-cov` and `cargo-crap` to be installed locally. Wired
# into `ci` so the tracked snapshot stays in sync with each green run.
# `cargo llvm-cov` runs the test suite under instrumentation and propagates
# any test failure, so `ci` doesn't need a separate `test` step.
crap:
    cargo llvm-cov --lcov --output-path lcov.info
    ./scripts/crap.sh

# Regression gate: fail CI if any function's CRAP score climbs past the locked
# ceiling. The doc report (recipe `crap`) flags everything over 30; this gate
# only fails the build on a regression past the current worst score so the bar
# can't silently creep up. Reuses the lcov.info that `crap` just generated.
#
# CRAP_CEILING is set just above the current worst offender (`App::run_until`,
# ~58 — its TTY browse block isn't headless-testable; see
# docs/plans/crap-cleanup.md). Ratchet it DOWN as scores improve, never up.
CRAP_CEILING := "60"
crap-gate:
    cargo-crap --lcov lcov.info --threshold {{CRAP_CEILING}} --fail-above --exclude 'tests/**' --summary

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
