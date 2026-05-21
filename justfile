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

ci: fmt-check lint test

# Audit GitHub Action SHA pins against current upstream.
check-pins:
    ./scripts/check-action-pins.sh

# Update drifted GitHub Action SHA pins in place; review with `git diff`.
update-pins:
    ./scripts/check-action-pins.sh --update

run *args:
    cargo run -- {{args}}

clean:
    cargo clean
