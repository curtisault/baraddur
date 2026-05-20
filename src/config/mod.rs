pub mod discovery;
pub mod schema;
pub mod validate;

pub use schema::{Config, OnFailureConfig, OutputConfig, Step, WatchConfig};
pub use validate::{ValidationErrors, validate};

use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};

/// Starter template written by `baraddur init`. Same content as the example
/// in `not_found_error`, so users see one config shape across both UX paths.
const STARTER_TEMPLATE: &str = "\
# baraddur config — see https://github.com/curtisault/baraddur#config

[watch]
extensions = [\"rs\"]
debounce_ms = 500
ignore = [\"target\", \".git\", \".baraddur\"]

# [output]
# clear_screen = true    # clear the terminal between runs
# show_passing = false   # hide stdout/stderr from passing steps

# [on_failure]            # optional: run a command after a failing run
# enabled = false         # set true to activate
# cmd = \"\"                # receives combined failed output on stdin
# prompt = \"\"             # optional preamble prepended to stdin
# timeout_secs = 30

[[steps]]
name = \"check\"
cmd = \"cargo check\"
parallel = false
";

/// The result of a successful config load.
#[derive(Debug)]
pub struct Loaded {
    pub config: Config,
    /// Absolute path of the file that was parsed.
    pub config_path: PathBuf,
    /// Directory containing `config_path`. Used as the default watch root
    /// when the config was discovered via walk-up so that the watcher is
    /// anchored at the project root, not the subdirectory where `baraddur`
    /// was invoked.
    pub config_dir: PathBuf,
    /// How the config was located, for deciding the default watch root.
    pub source: ConfigSource,
}

#[derive(Debug, Clone, Copy)]
pub enum ConfigSource {
    CliOverride,
    WalkUp,
    Global,
}

/// Loads and validates the config.
///
/// Resolution order:
/// 1. `cli_override` path if provided (skips discovery entirely).
/// 2. Walk up from `cwd` looking for `.baraddur.toml`.
/// 3. `~/.config/baraddur/config.toml`.
/// 4. Error — matches UX state E1.
pub fn load(cli_override: Option<&Path>) -> Result<Loaded> {
    if let Some(path) = cli_override {
        return load_from(path, ConfigSource::CliOverride);
    }

    let cwd = std::env::current_dir().context("getting current directory")?;
    let walk = discovery::walk_up(&cwd);

    if let Some(path) = walk.found {
        return load_from(&path, ConfigSource::WalkUp);
    }

    if let Some(global) = discovery::global_path()
        && global.is_file()
    {
        return load_from(&global, ConfigSource::Global);
    }

    Err(anyhow!(not_found_error(
        &walk.searched,
        discovery::global_path().as_deref()
    )))
}

fn load_from(path: &Path, source: ConfigSource) -> Result<Loaded> {
    if !path.is_file() {
        anyhow::bail!("config file not found: {}", path.display());
    }

    let abs = path
        .canonicalize()
        .with_context(|| format!("resolving {}", path.display()))?;

    let contents =
        std::fs::read_to_string(&abs).with_context(|| format!("reading {}", abs.display()))?;

    let config: Config = toml::from_str(&contents)
        .map_err(|e| anyhow!("config error in {}\n\n{}", abs.display(), e))?;

    validate(&config).map_err(|e| anyhow!("config error in {}\n\n{}", abs.display(), e))?;

    let config_dir = abs
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| anyhow!("config path {} has no parent", abs.display()))?;

    Ok(Loaded {
        config,
        config_path: abs,
        config_dir,
        source,
    })
}

/// Scaffolds a starter `.baraddur.toml` in `dir`. Refuses if a config already
/// exists at that path. Returns the absolute path that was written.
pub fn init(dir: &Path) -> Result<PathBuf> {
    let path = dir.join(discovery::CONFIG_FILENAME);
    if path.exists() {
        anyhow::bail!("{} already exists", path.display());
    }
    std::fs::write(&path, STARTER_TEMPLATE)
        .with_context(|| format!("writing {}", path.display()))?;
    Ok(path)
}

/// Renders UX state E1. Phrasing is stable — tests assert against it.
fn not_found_error(searched: &[PathBuf], global: Option<&Path>) -> String {
    let mut msg = String::from("no .baraddur.toml found in this directory or any parent");

    match global {
        Some(g) => {
            msg.push_str(",\n          and no ");
            if let Some(home) = dirs::home_dir() {
                if let Ok(rel) = g.strip_prefix(&home) {
                    msg.push('~');
                    msg.push(std::path::MAIN_SEPARATOR);
                    msg.push_str(&rel.to_string_lossy());
                } else {
                    msg.push_str(&g.to_string_lossy());
                }
            } else {
                msg.push_str(&g.to_string_lossy());
            }
            msg.push('.');
        }
        None => msg.push('.'),
    }

    msg.push_str(
        "\n\ncreate a .baraddur.toml in your project root. minimal example:\
         \n\n  [watch]\
         \n  extensions = [\"rs\"]\
         \n\n  [[steps]]\
         \n  name = \"check\"\
         \n  cmd  = \"cargo check\"",
    );

    if !searched.is_empty() {
        msg.push_str("\n\n(searched:");
        for p in searched {
            msg.push_str("\n  ");
            msg.push_str(&p.display().to_string());
        }
        msg.push(')');
    }

    msg
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn init_writes_starter_config() {
        let td = TempDir::new().unwrap();
        let path = init(td.path()).unwrap();
        assert!(path.exists());
        assert_eq!(path.file_name().unwrap(), discovery::CONFIG_FILENAME);

        // The written template must parse as a valid Config.
        let contents = std::fs::read_to_string(&path).unwrap();
        let cfg: Config = toml::from_str(&contents).expect("starter template parses");
        validate(&cfg).expect("starter template validates");
    }

    #[test]
    fn init_refuses_to_overwrite() {
        let td = TempDir::new().unwrap();
        init(td.path()).unwrap();
        let err = init(td.path()).unwrap_err();
        assert!(err.to_string().contains("already exists"));
    }
}
