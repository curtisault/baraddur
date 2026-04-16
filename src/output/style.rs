use crossterm::style::{StyledContent, Stylize};
use std::fmt;

// ── Color detection ─────────────────────────────────────────────────────────

/// Returns true if color output should be enabled.
/// Respects the `NO_COLOR` environment variable (no-color.org).
pub fn should_color(is_tty: bool) -> bool {
    is_tty && std::env::var_os("NO_COLOR").is_none()
}

// ── Conditional styling ─────────────────────────────────────────────────────

/// A piece of text that is either styled (with ANSI escapes) or plain.
/// Implements `Display` so it can be used in `format!()` / `write!()`.
pub enum Styled<'a> {
    Plain(&'a str),
    Colored(StyledContent<&'a str>),
}

impl fmt::Display for Styled<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Styled::Plain(s) => f.write_str(s),
            Styled::Colored(sc) => write!(f, "{sc}"),
        }
    }
}

/// Resolved once at startup. Passed into display constructors.
/// All styling decisions go through Theme so NO_COLOR is respected everywhere.
pub struct Theme {
    color: bool,
}

impl Theme {
    pub fn new(color: bool) -> Self {
        Self { color }
    }

    pub fn color_enabled(&self) -> bool {
        self.color
    }

    pub fn style<'a>(
        &self,
        text: &'a str,
        apply: fn(&'a str) -> StyledContent<&'a str>,
    ) -> Styled<'a> {
        if self.color {
            Styled::Colored(apply(text))
        } else {
            Styled::Plain(text)
        }
    }

    pub fn pass_glyph(&self) -> Styled<'static> {
        self.style("✓", |s| s.green().bold())
    }

    pub fn fail_glyph(&self) -> Styled<'static> {
        self.style("✗", |s| s.red().bold())
    }

    pub fn skip_glyph(&self) -> Styled<'static> {
        self.style("⊘", |s| s.dark_grey())
    }

    pub fn queued_glyph(&self) -> Styled<'static> {
        self.style("·", |s| s.dark_grey())
    }

    pub fn dim<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.dim())
    }

    pub fn cyan<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.cyan())
    }

    pub fn red<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.red())
    }

    pub fn green<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.green())
    }

    pub fn yellow<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.yellow())
    }

    pub fn selected<'a>(&self, text: &'a str) -> Styled<'a> {
        self.style(text, |s| s.reverse())
    }
}

// ── ANSI-aware string measurement ───────────────────────────────────────────

/// Returns the visible character count of a string, stripping ANSI SGR
/// escape sequences (ESC [ ... m).
pub fn visible_len(s: &str) -> usize {
    let mut len = 0;
    let mut in_escape = false;
    for c in s.chars() {
        if c == '\x1b' {
            in_escape = true;
        } else if in_escape {
            if c == 'm' {
                in_escape = false;
            }
        } else {
            len += 1;
        }
    }
    len
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossterm::style::Stylize;

    #[test]
    fn visible_len_plain() {
        assert_eq!(visible_len("hello"), 5);
    }

    #[test]
    fn visible_len_with_ansi() {
        let styled = format!("{}", "✓".green().bold());
        assert_eq!(visible_len(&styled), 1);
    }

    #[test]
    fn visible_len_empty() {
        assert_eq!(visible_len(""), 0);
    }

    #[test]
    fn styled_plain_has_no_escapes() {
        let theme = Theme::new(false);
        let s = format!("{}", theme.pass_glyph());
        assert_eq!(s, "✓");
        assert!(!s.contains('\x1b'));
    }

    #[test]
    fn styled_colored_has_escapes() {
        let theme = Theme::new(true);
        let s = format!("{}", theme.pass_glyph());
        assert!(s.contains('\x1b'), "expected ANSI escapes in: {s:?}");
    }
}
