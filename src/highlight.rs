use ratatui::style::Color;
use std::sync::OnceLock;
use syntect::easy::HighlightLines;
use syntect::highlighting::{Theme, ThemeSet};
use syntect::parsing::SyntaxSet;

pub struct Highlighter {
    syntaxes: SyntaxSet,
    theme: Theme,
}

impl Highlighter {
    pub fn new() -> Self {
        let syntaxes = SyntaxSet::load_defaults_newlines();
        let theme = ThemeSet::load_defaults().themes["base16-ocean.dark"].clone();
        Highlighter { syntaxes, theme }
    }

    /// Highlight one line into (foreground color, text) spans. Unknown `lang` (a file
    /// extension) or a syntect error falls back to a single plain span — never panics.
    /// ponytail: each line highlighted independently; multi-line constructs (block
    /// comments/strings) aren't carried across lines.
    pub fn line(&self, lang: &str, text: &str) -> Vec<(Color, String)> {
        let syntax = self
            .syntaxes
            .find_syntax_by_extension(lang)
            .unwrap_or_else(|| self.syntaxes.find_syntax_plain_text());
        let mut h = HighlightLines::new(syntax, &self.theme);
        match h.highlight_line(text, &self.syntaxes) {
            Ok(ranges) => ranges
                .into_iter()
                .map(|(s, t)| {
                    let c = s.foreground;
                    (Color::Rgb(c.r, c.g, c.b), t.to_string())
                })
                .collect(),
            Err(_) => vec![(Color::Reset, text.to_string())],
        }
    }
}

impl Default for Highlighter {
    fn default() -> Self {
        Self::new()
    }
}

/// A process-global highlighter, built once (syntect default-load is not free).
pub fn shared() -> &'static Highlighter {
    static H: OnceLock<Highlighter> = OnceLock::new();
    H.get_or_init(Highlighter::new)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_language_tokenizes_into_spans() {
        let h = Highlighter::new();
        let spans = h.line("rs", "let x = 5;");
        assert!(spans.len() > 1, "rust should tokenize into multiple spans");
        let joined: String = spans.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(joined.trim_end(), "let x = 5;");
    }

    #[test]
    fn unknown_language_round_trips_as_plain() {
        let h = Highlighter::new();
        let spans = h.line("zzzz", "plain text");
        let joined: String = spans.iter().map(|(_, t)| t.as_str()).collect();
        assert_eq!(joined.trim_end(), "plain text");
    }

    #[test]
    fn shared_is_reused() {
        let a = shared() as *const Highlighter;
        let b = shared() as *const Highlighter;
        assert_eq!(a, b);
    }
}
