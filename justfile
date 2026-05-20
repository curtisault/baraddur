default: test

build:
    cargo build

release:
    cargo build --release

install: release
    cp ./target/release/baraddur ~/.local/bin/baraddur

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
