use baraddur::config::{self, ConfigSource};
use std::fs;
use tempfile::TempDir;

const MINIMAL_CONFIG: &str = r#"
[watch]
extensions = ["rs"]

[[steps]]
name = "check"
cmd = "cargo check"
"#;

// ── Explicit path tests (no cwd mutation, safe under parallel test runs) ────

#[test]
fn explicit_path_loads_config() {
    let td = TempDir::new().unwrap();
    let path = td.path().join(".baraddur.toml");
    fs::write(&path, MINIMAL_CONFIG).unwrap();

    let loaded = config::load(Some(&path)).unwrap();
    assert!(matches!(loaded.source, ConfigSource::CliOverride));
    assert_eq!(loaded.config.steps.len(), 1);
    assert_eq!(loaded.config.steps[0].name, "check");
}

#[test]
fn config_dir_is_parent_of_config_path() {
    let td = TempDir::new().unwrap();
    let path = td.path().join(".baraddur.toml");
    fs::write(&path, MINIMAL_CONFIG).unwrap();

    let loaded = config::load(Some(&path)).unwrap();
    let canon_td = td.path().canonicalize().unwrap();
    assert_eq!(loaded.config_dir, canon_td);
}

#[test]
fn explicit_missing_config_is_hard_error() {
    let td = TempDir::new().unwrap();
    let missing = td.path().join("does-not-exist.toml");
    let err = config::load(Some(&missing)).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("not found"), "unexpected error: {msg}");
}

// ── Parse error presentation ─────────────────────────────────────────────────

#[test]
fn parse_error_wrapped_with_file_path() {
    let td = TempDir::new().unwrap();
    let bad = td.path().join(".baraddur.toml");
    fs::write(&bad, "this_is_not_toml = = garbage\n").unwrap();

    let err = config::load(Some(&bad)).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("config error in"), "unexpected error: {msg}");
    assert!(msg.contains(".baraddur.toml"), "unexpected error: {msg}");
}

#[test]
fn unknown_field_error_names_the_field() {
    let td = TempDir::new().unwrap();
    let bad = td.path().join(".baraddur.toml");
    fs::write(
        &bad,
        r#"
[watch]
extensions = ["rs"]

[[steps]]
name = "c"
cmd = "true"
parralel = false
"#,
    )
    .unwrap();

    let err = config::load(Some(&bad)).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("parralel"), "unexpected error: {msg}");
}

// ── Validation error presentation ────────────────────────────────────────────

#[test]
fn validation_error_wrapped_with_file_path() {
    let td = TempDir::new().unwrap();
    let bad = td.path().join(".baraddur.toml");
    fs::write(
        &bad,
        r#"
[watch]
extensions = ["rs"]

[[steps]]
name = ""
cmd = ""
"#,
    )
    .unwrap();

    let err = config::load(Some(&bad)).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("config error in"), "unexpected error: {msg}");
    assert!(msg.contains("empty `name`"), "unexpected error: {msg}");
    assert!(msg.contains("empty `cmd`"), "unexpected error: {msg}");
}

#[test]
fn empty_steps_validation_error() {
    let td = TempDir::new().unwrap();
    let bad = td.path().join(".baraddur.toml");
    fs::write(
        &bad,
        r#"
[watch]
extensions = ["rs"]
"#,
    )
    .unwrap();

    let err = config::load(Some(&bad)).unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("no steps"), "unexpected error: {msg}");
}

// ── Forward compatibility: [summarize] table ─────────────────────────────────

#[test]
fn summarize_table_is_accepted() {
    let td = TempDir::new().unwrap();
    let path = td.path().join(".baraddur.toml");
    fs::write(
        &path,
        r#"
[watch]
extensions = ["rs"]

[summarize]
enabled = true
cmd = "claude -p"
timeout_secs = 30

[[steps]]
name = "check"
cmd = "cargo check"
"#,
    )
    .unwrap();

    let loaded = config::load(Some(&path)).unwrap();
    assert!(loaded.config.summarize.enabled);
    assert_eq!(loaded.config.summarize.timeout_secs, 30);
}

// ── Discovery: walk-up via pure function (avoids cwd mutation) ───────────────

#[test]
fn walk_up_finds_config_from_subdir() {
    use baraddur::config::discovery;

    let td = TempDir::new().unwrap();
    let root = td.path().canonicalize().unwrap();
    fs::write(root.join(".baraddur.toml"), "").unwrap();

    let deep = root.join("a/b/c");
    fs::create_dir_all(&deep).unwrap();

    let result = discovery::walk_up(&deep);
    let found = result.found.unwrap().canonicalize().unwrap();
    assert_eq!(found, root.join(".baraddur.toml").canonicalize().unwrap());
}

#[test]
fn walk_up_returns_none_inside_empty_tempdir() {
    use baraddur::config::discovery;

    let td = TempDir::new().unwrap();
    let result = discovery::walk_up(td.path());
    // The found path, if any, must be outside the tempdir.
    if let Some(p) = &result.found {
        let p = p.canonicalize().unwrap();
        let td_canon = td.path().canonicalize().unwrap();
        assert!(!p.starts_with(td_canon));
    }
}
