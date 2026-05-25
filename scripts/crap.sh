#!/usr/bin/env bash
# crap.sh — local CRAP (Change Risk Anti-Patterns) analysis for review.
#
# cargo-crap is NOT a baraddur dependency. It must be installed separately
# by each developer who wants to run this analysis. See the crate page for
# install instructions and details on what the score means:
#
#     https://crates.io/crates/cargo-crap
#
# This script wraps cargo-crap with defaults tuned for reviewing the
# baraddur source tree:
#   - production code only (tests/ excluded)
#   - top 20 crappiest functions
#   - markdown output so it's readable both in a terminal and pasted
#     into a PR comment
#   - coverage-aware when an `lcov.info` is present at the repo root
#     (generate one with `cargo llvm-cov --lcov --output-path lcov.info`);
#     without coverage every function is scored as 0% covered, which is
#     still useful for spotting complexity hotspots but isn't a true
#     CRAP run.
#
# Any extra args are forwarded to cargo-crap, so you can override or
# extend the defaults, e.g.:
#     scripts/crap.sh --summary
#     scripts/crap.sh --top 5 --format json --output crap.json
#     scripts/crap.sh --threshold 50

set -euo pipefail

if ! command -v cargo-crap >/dev/null 2>&1; then
    cat >&2 <<'EOF'
error: cargo-crap is not installed.

This script depends on cargo-crap, which is NOT a baraddur dependency
and must be installed separately. See the crate page for instructions:

    https://crates.io/crates/cargo-crap

Typical install:
    cargo install cargo-crap
EOF
    exit 1
fi

ARGS=(--top 20 --threshold 30 --format markdown --exclude 'tests/**')

if [[ -f lcov.info ]]; then
    ARGS+=(--lcov lcov.info)
else
    cat >&2 <<'EOF'
note: lcov.info not found at repo root — running complexity-only (no coverage).
      generate one with: cargo llvm-cov --lcov --output-path lcov.info
EOF
fi

exec cargo-crap "${ARGS[@]}" "$@"
