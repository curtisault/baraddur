# Install

## From crates.io

```bash
cargo install baraddur
```

## Pre-built binaries

Download the tarball for your platform from the
[latest release](https://github.com/curtisault/baraddur/releases/latest) and
place `baraddur` somewhere on your `$PATH`. Supported targets:

| Platform | Target triple |
|---|---|
| macOS (Apple Silicon) | `aarch64-apple-darwin` |
| macOS (Intel) | `x86_64-apple-darwin` |
| Linux (x86_64) | `x86_64-unknown-linux-gnu` |
| Linux (aarch64) | `aarch64-unknown-linux-gnu` |

Each tarball ships with `README.md`, `LICENSE-MIT`, and `LICENSE-APACHE`
alongside the binary, and a `.sha256` checksum file is attached to the
release.

## Homebrew

Coming soon — tap not yet published.

## From source

Requires Rust 1.85 or newer.

```bash
just install
# or manually:
cargo build --release && cp ./target/release/baraddur ~/.local/bin/baraddur
```
