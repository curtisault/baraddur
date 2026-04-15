pub mod schema;
pub use schema::{Config, OutputConfig, Step, WatchConfig};

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};

pub const CONFIG_FILENAME: &str = ".baraddur.toml";

/// Loads config from an explicit path or `./.baraddur.toml` in the current directory.
///
/// Walk-up discovery and global fallback are Phase 2 features.
pub fn load(cli_override: Option<&Path>) -> Result<(Config, PathBuf)> {
    let path = match cli_override {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir()
            .context("getting current directory")?
            .join(CONFIG_FILENAME),
    };

    if !path.is_file() {
        anyhow::bail!(
            "no config file found at {}\n\ncreate a {} in your project root. minimal example:\n\n  [watch]\n  extensions = [\"rs\"]\n\n  [[steps]]\n  name = \"check\"\n  cmd  = \"cargo check\"",
            path.display(),
            CONFIG_FILENAME
        );
    }

    let contents =
        std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;

    let config: Config =
        toml::from_str(&contents).with_context(|| format!("parsing {}", path.display()))?;

    Ok((config, path))
}
