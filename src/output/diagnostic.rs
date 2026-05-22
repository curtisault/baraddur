//! Extracts `(path, line, col?)` triples from step output so browse mode can
//! offer "jump to file:line" navigation.
//!
//! Supported producers:
//! - `rustc` / `clippy`: `--> src/foo.rs:42:5` (also `::: …` for context spans)
//! - Test panics: `panicked at src/foo.rs:42:5:` (2021+) or `…panicked at 'msg', src/foo.rs:42:5`
//! - Generic: `src/foo.ts:42:5: error: …` and `src/foo.py:42: …` at the start of a line
//!
//! The parser is intentionally permissive about the surrounding line — it
//! locates a `path:line(:col)?` substring after a known marker and trusts
//! that it's a diagnostic. The path heuristic requires at least one of `/`,
//! `.`, `\` so plain identifiers like `Caused by: foo:42` don't get picked up.

use std::collections::HashSet;
use std::path::PathBuf;

/// One `(file, line, col?)` triple suitable for opening in `$EDITOR`.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Diagnostic {
    pub path: PathBuf,
    pub line: u32,
    pub col: Option<u32>,
}

/// Walks `output` line-by-line, returning every diagnostic in declared
/// order. Duplicates by `(path, line, col)` are dropped so clippy notes
/// that point at the same source location only appear once.
pub fn parse(output: &str) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut seen: HashSet<(PathBuf, u32, Option<u32>)> = HashSet::new();

    for line in output.lines() {
        if let Some(d) = extract_line(line) {
            let key = (d.path.clone(), d.line, d.col);
            if seen.insert(key) {
                out.push(d);
            }
        }
    }
    out
}

/// Pattern dispatch for a single line of output. Returns the first hit.
/// Exposed so the display can tag individual rendered lines as navigable.
pub fn extract_line(line: &str) -> Option<Diagnostic> {
    // rustc / clippy primary span: `  --> path:line:col`
    if let Some(idx) = line.find("--> ") {
        if let Some(d) = parse_path_line_col(&line[idx + 4..]) {
            return Some(d);
        }
    }
    // rustc context span: `  ::: path:line:col`
    if let Some(idx) = line.find("::: ") {
        if let Some(d) = parse_path_line_col(&line[idx + 4..]) {
            return Some(d);
        }
    }
    // Rust 2021+ panic: `thread 'main' panicked at path:line:col:`
    // Rust legacy panic:  `thread 'main' panicked at 'msg', path:line:col`
    if let Some(idx) = line.find("panicked at ") {
        let rest = &line[idx + "panicked at ".len()..];
        // Legacy form has a `, ` separator between the message and the location.
        if let Some(comma) = rest.find(", ") {
            if let Some(d) = parse_path_line_col(&rest[comma + 2..]) {
                return Some(d);
            }
        }
        if let Some(d) = parse_path_line_col(rest) {
            return Some(d);
        }
    }
    // Generic eslint/mypy/gofmt style: `path:line[:col]: …` at the start of
    // the (possibly indented) line.
    parse_path_line_col(line.trim_start())
}

/// Parses `<path>:<line>(:<col>)?` from the start of `s`. The path heuristic
/// rejects bare identifiers — there must be at least one path-ish character.
fn parse_path_line_col(s: &str) -> Option<Diagnostic> {
    let bytes = s.as_bytes();
    let mut i = 0;

    // Locate the colon that separates the path from the line number. The
    // path is anything up to the first `:DIGIT`; whitespace ends the scan
    // (paths embed colons on Windows, but we don't try to be cute about it).
    let path_end = loop {
        if i >= bytes.len() {
            return None;
        }
        let c = bytes[i];
        if c.is_ascii_whitespace() {
            return None;
        }
        if c == b':' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            break i;
        }
        i += 1;
    };

    if path_end == 0 {
        return None;
    }
    let path_str = &s[..path_end];
    // Path-ish heuristic: must contain at least one separator or dot. Filters
    // out bare identifiers that happen to be followed by `:DIGIT`.
    if !path_str.contains('/') && !path_str.contains('.') && !path_str.contains('\\') {
        return None;
    }

    // Line digits.
    let line_start = path_end + 1;
    let mut line_end = line_start;
    while line_end < bytes.len() && bytes[line_end].is_ascii_digit() {
        line_end += 1;
    }
    let line: u32 = s[line_start..line_end].parse().ok()?;

    // Optional :col.
    let col = if line_end < bytes.len()
        && bytes[line_end] == b':'
        && bytes
            .get(line_end + 1)
            .map(|b| b.is_ascii_digit())
            .unwrap_or(false)
    {
        let col_start = line_end + 1;
        let mut col_end = col_start;
        while col_end < bytes.len() && bytes[col_end].is_ascii_digit() {
            col_end += 1;
        }
        s[col_start..col_end].parse().ok()
    } else {
        None
    };

    Some(Diagnostic {
        path: PathBuf::from(path_str),
        line,
        col,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn diag(path: &str, line: u32, col: Option<u32>) -> Diagnostic {
        Diagnostic {
            path: PathBuf::from(path),
            line,
            col,
        }
    }

    #[test]
    fn parses_rustc_primary_span() {
        let input = "error[E0277]: trait bound\n  --> src/foo.rs:42:5\n   |\n42 |     ...";
        let diags = parse(input);
        assert_eq!(diags, vec![diag("src/foo.rs", 42, Some(5))]);
    }

    #[test]
    fn parses_clippy_warning() {
        let input = "warning: unused variable: `x`\n  --> src/bar.rs:10:9\n";
        assert_eq!(parse(input), vec![diag("src/bar.rs", 10, Some(9))]);
    }

    #[test]
    fn parses_rustc_context_span() {
        // clippy/rustc use `:::` for cross-file context. Both should be navigable.
        let input = "  --> a.rs:1:1\n  ::: b.rs:2:2\n";
        assert_eq!(
            parse(input),
            vec![diag("a.rs", 1, Some(1)), diag("b.rs", 2, Some(2))]
        );
    }

    #[test]
    fn parses_panic_modern() {
        let input = "thread 'main' panicked at src/foo.rs:42:5:\nassertion failed: false";
        assert_eq!(parse(input), vec![diag("src/foo.rs", 42, Some(5))]);
    }

    #[test]
    fn parses_panic_legacy() {
        let input = "thread 'main' panicked at 'assertion failed', src/foo.rs:42:5";
        assert_eq!(parse(input), vec![diag("src/foo.rs", 42, Some(5))]);
    }

    #[test]
    fn parses_generic_filename_line_col() {
        let input = "src/foo.ts:42:5: error: blah\nsrc/bar.py:10: error: blah";
        assert_eq!(
            parse(input),
            vec![
                diag("src/foo.ts", 42, Some(5)),
                diag("src/bar.py", 10, None),
            ]
        );
    }

    #[test]
    fn rejects_lines_without_path_separator() {
        // `foo:42` looks like a diag but has no `/`, `.`, or `\` — filtered.
        let input = "Caused by: foo:42 stuff\n";
        assert!(parse(input).is_empty());
    }

    #[test]
    fn dedups_same_location() {
        let input = "  --> src/foo.rs:42:5\n  --> src/foo.rs:42:5\n";
        assert_eq!(parse(input).len(), 1);
    }

    #[test]
    fn keeps_distinct_locations_in_same_file() {
        let input = "  --> src/foo.rs:42:5\n  --> src/foo.rs:50:1\n";
        assert_eq!(parse(input).len(), 2);
    }

    #[test]
    fn handles_absolute_paths() {
        let input = "  --> /home/me/foo.rs:1:1\n";
        assert_eq!(parse(input), vec![diag("/home/me/foo.rs", 1, Some(1))]);
    }

    #[test]
    fn empty_input_returns_empty() {
        assert!(parse("").is_empty());
    }

    #[test]
    fn no_colon_digit_returns_none() {
        assert!(parse("just text with no diags\n").is_empty());
    }
}
