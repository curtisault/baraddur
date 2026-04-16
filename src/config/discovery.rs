use std::path::{Path, PathBuf};

pub const CONFIG_FILENAME: &str = ".baraddur.toml";
pub const GLOBAL_RELATIVE: &str = ".config/baraddur/config.toml";

/// Walks upward from `start`, returning the first `.baraddur.toml` found.
/// Also returns every directory that was searched, for error reporting.
pub fn walk_up(start: &Path) -> WalkResult {
    let mut searched = Vec::new();
    let mut cur = Some(start);

    while let Some(dir) = cur {
        searched.push(dir.to_path_buf());
        let candidate = dir.join(CONFIG_FILENAME);
        if candidate.is_file() {
            return WalkResult {
                found: Some(candidate),
                searched,
            };
        }
        cur = dir.parent();
    }

    WalkResult {
        found: None,
        searched,
    }
}

pub struct WalkResult {
    pub found: Option<PathBuf>,
    pub searched: Vec<PathBuf>,
}

/// Returns `~/.config/baraddur/config.toml`, or `None` if `$HOME` is
/// unresolvable. Does not check whether the file exists.
pub fn global_path() -> Option<PathBuf> {
    dirs::home_dir().map(|h| h.join(GLOBAL_RELATIVE))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn finds_config_in_start_dir() {
        let td = TempDir::new().unwrap();
        let cfg_path = td.path().join(CONFIG_FILENAME);
        fs::write(&cfg_path, "").unwrap();

        let result = walk_up(td.path());
        // Canonicalize both sides to handle macOS /var → /private/var symlinks.
        let found = result.found.unwrap().canonicalize().unwrap();
        let expected = cfg_path.canonicalize().unwrap();
        assert_eq!(found, expected);
    }

    #[test]
    fn finds_config_in_ancestor() {
        let td = TempDir::new().unwrap();
        let deep = td.path().join("a/b/c");
        fs::create_dir_all(&deep).unwrap();
        let cfg_path = td.path().join(CONFIG_FILENAME);
        fs::write(&cfg_path, "").unwrap();

        let deep = deep.canonicalize().unwrap();
        let cfg_path = cfg_path.canonicalize().unwrap();

        let result = walk_up(&deep);
        let found = result.found.unwrap().canonicalize().unwrap();
        assert_eq!(found, cfg_path);
        // Searched at least: c, b, a, tempdir root.
        assert!(result.searched.len() >= 4);
    }

    #[test]
    fn returns_none_when_not_in_tempdir_tree() {
        let td = TempDir::new().unwrap();
        // Don't create any .baraddur.toml inside the tempdir.
        let result = walk_up(td.path());
        // The found path, if any, must be outside the tempdir.
        if let Some(p) = &result.found {
            let p = p.canonicalize().unwrap();
            let td_canon = td.path().canonicalize().unwrap();
            assert!(!p.starts_with(td_canon));
        }
    }

    #[test]
    fn global_path_points_inside_home() {
        let p = global_path().expect("HOME should resolve in test environment");
        assert!(
            p.to_string_lossy().contains(".config/baraddur/config.toml"),
            "unexpected global path: {}",
            p.display()
        );
    }
}
