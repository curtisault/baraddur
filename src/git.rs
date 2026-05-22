//! Git-aware path discovery for `check --staged` / `check --since`.
//!
//! Shells out to `git` rather than linking `git2`; we only need a handful of
//! read-only queries and the binary is always present in dev workflows.

use anyhow::{Context, Result, anyhow};
use std::path::{Path, PathBuf};
use tokio::process::Command;

/// Returns the canonical working-tree root containing `cwd`. Errors with a
/// user-readable message when `cwd` isn't inside a git repository.
pub async fn repo_root(cwd: &Path) -> Result<PathBuf> {
    let out = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .await
        .context("running `git rev-parse --show-toplevel`")?;

    if !out.status.success() {
        return Err(anyhow!(
            "not inside a git working tree (run from within a repository to use --staged / --since)"
        ));
    }

    let s = std::str::from_utf8(&out.stdout)
        .context("git rev-parse output is not UTF-8")?
        .trim();
    PathBuf::from(s)
        .canonicalize()
        .with_context(|| format!("canonicalizing git root {s}"))
}

/// Files staged in the index (`git diff --name-only --cached -z`).
/// Returned paths are relative to `repo`.
pub async fn staged_paths(repo: &Path) -> Result<Vec<PathBuf>> {
    let out = run_git(repo, &["diff", "--name-only", "--cached", "-z"]).await?;
    Ok(parse_nul(&out))
}

/// Files changed between `base` and HEAD using merge-base semantics
/// (`base...HEAD`), plus untracked-but-not-ignored files. Untracked files
/// are appended so the path list matches "what's new since `base`" from the
/// user's working-tree perspective, not just what's already committed.
pub async fn diff_since(repo: &Path, base: &str) -> Result<Vec<PathBuf>> {
    let range = format!("{base}...HEAD");
    let diff_out = run_git(repo, &["diff", "--name-only", "-z", &range]).await?;
    let mut paths = parse_nul(&diff_out);

    let untracked_out =
        run_git(repo, &["ls-files", "--others", "--exclude-standard", "-z"]).await?;
    paths.extend(parse_nul(&untracked_out));

    Ok(paths)
}

async fn run_git(cwd: &Path, args: &[&str]) -> Result<Vec<u8>> {
    let out = Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .await
        .with_context(|| format!("running `git {}`", args.join(" ")))?;

    if !out.status.success() {
        let stderr = String::from_utf8_lossy(&out.stderr);
        return Err(anyhow!(
            "`git {}` failed: {}",
            args.join(" "),
            stderr.trim()
        ));
    }
    Ok(out.stdout)
}

/// Splits NUL-separated git output into path entries. Empty chunks are
/// dropped so a trailing NUL or two concatenated outputs don't produce
/// phantom paths.
pub(crate) fn parse_nul(bytes: &[u8]) -> Vec<PathBuf> {
    bytes
        .split(|&b| b == 0)
        .filter(|chunk| !chunk.is_empty())
        .map(bytes_to_pathbuf)
        .collect()
}

#[cfg(unix)]
fn bytes_to_pathbuf(b: &[u8]) -> PathBuf {
    use std::os::unix::ffi::OsStrExt;
    PathBuf::from(std::ffi::OsStr::from_bytes(b))
}

#[cfg(not(unix))]
fn bytes_to_pathbuf(b: &[u8]) -> PathBuf {
    PathBuf::from(String::from_utf8_lossy(b).into_owned())
}

/// Rebases `git_paths` (relative to `repo`) so they become relative to
/// `app_root`. Both paths must already be canonical. Paths outside `app_root`
/// are dropped — they can't match a step glob anyway.
pub fn rebase_for_app(repo: &Path, app_root: &Path, git_paths: &[PathBuf]) -> Vec<PathBuf> {
    git_paths
        .iter()
        .filter_map(|p| {
            let abs = repo.join(p);
            abs.strip_prefix(app_root).ok().map(Path::to_path_buf)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_nul_basic() {
        let paths = parse_nul(b"a.rs\0b/c.rs\0");
        assert_eq!(paths, vec![PathBuf::from("a.rs"), PathBuf::from("b/c.rs")]);
    }

    #[test]
    fn parse_nul_handles_paths_with_spaces() {
        // NUL is the only separator; embedded spaces stay inside a single entry.
        let paths = parse_nul(b"foo bar.rs\0baz qux.rs\0");
        assert_eq!(
            paths,
            vec![PathBuf::from("foo bar.rs"), PathBuf::from("baz qux.rs")]
        );
    }

    #[test]
    fn parse_nul_drops_empty_chunks() {
        // Two concatenated git outputs can produce a double-NUL.
        let paths = parse_nul(b"a\0\0b\0");
        assert_eq!(paths, vec![PathBuf::from("a"), PathBuf::from("b")]);
    }

    #[test]
    fn parse_nul_no_trailing_nul() {
        let paths = parse_nul(b"a\0b");
        assert_eq!(paths, vec![PathBuf::from("a"), PathBuf::from("b")]);
    }

    #[test]
    fn rebase_drops_paths_outside_app_root() {
        let repo = PathBuf::from("/repo");
        let app = PathBuf::from("/repo/sub");
        let git_paths = vec![PathBuf::from("sub/a.rs"), PathBuf::from("other/b.rs")];
        let rebased = rebase_for_app(&repo, &app, &git_paths);
        assert_eq!(rebased, vec![PathBuf::from("a.rs")]);
    }

    #[test]
    fn rebase_when_app_equals_repo() {
        let repo = PathBuf::from("/repo");
        let app = PathBuf::from("/repo");
        let git_paths = vec![PathBuf::from("a.rs"), PathBuf::from("sub/b.rs")];
        let rebased = rebase_for_app(&repo, &app, &git_paths);
        assert_eq!(
            rebased,
            vec![PathBuf::from("a.rs"), PathBuf::from("sub/b.rs")]
        );
    }
}
