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
    cargo clippy -- -D warnings

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

ci: fmt-check lint test

run *args:
    cargo run -- {{args}}

clean:
    cargo clean
