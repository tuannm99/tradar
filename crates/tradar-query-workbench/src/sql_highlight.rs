//! Tree-sitter-based SQL syntax highlighting for `QueryEditorComponent`,
//! using the grammar's own bundled `highlights.scm` (via
//! `tree_sitter_sequel::HIGHLIGHTS_QUERY`) through `tree-sitter-highlight`
//! -- the standard way editors turn a tree-sitter parse into colored spans.
//! Only SQL (Postgres/SQLite) is wired up for now; Mongo/Elasticsearch/
//! Redis use their own hand-rolled query shapes with no tree-sitter grammar
//! to match, so `QueryEditorComponent::set_dialect` leaves them as
//! `Dialect::PlainText`.
//!
//! Known limitation: `tree-sitter-sequel`'s bundled `highlights.scm` was
//! written for Neovim's Lua-based highlighter, whose `#match?` predicates
//! use Lua patterns (`%d+`) rather than the regex `tree-sitter-highlight`
//! (this crate) actually understands. The `@number`/`@float` predicates
//! never match as a result, so numeric literals fall back to `@string`'s
//! color instead of their own -- cosmetic, not a correctness issue, and not
//! worth forking the query file over.

use std::sync::OnceLock;

use ratatui::style::Color;
use tree_sitter_highlight::{Highlight, HighlightConfiguration, HighlightEvent, Highlighter};

/// Must stay in the same order passed to `HighlightConfiguration::configure`
/// -- a `HighlightEvent::HighlightStart` carries an index into this list.
const HIGHLIGHT_NAMES: &[&str] = &[
    "attribute",
    "boolean",
    "comment",
    "conditional",
    "field",
    "float",
    "function.call",
    "keyword",
    "keyword.operator",
    "number",
    "operator",
    "parameter",
    "punctuation.bracket",
    "punctuation.delimiter",
    "spell",
    "storageclass",
    "string",
    "type",
    "type.builtin",
    "type.qualifier",
    "variable",
];

fn sql_config() -> &'static HighlightConfiguration {
    static CONFIG: OnceLock<HighlightConfiguration> = OnceLock::new();
    CONFIG.get_or_init(|| {
        let mut config = HighlightConfiguration::new(
            tree_sitter_sequel::LANGUAGE.into(),
            "sql",
            tree_sitter_sequel::HIGHLIGHTS_QUERY,
            "",
            "",
        )
        .expect("tree-sitter-sequel's bundled highlights.scm must be a valid query");
        config.configure(HIGHLIGHT_NAMES);
        config
    })
}

fn color_for(highlight_name: &str) -> Color {
    match highlight_name {
        "keyword" | "keyword.operator" | "conditional" => Color::Magenta,
        "string" => Color::Green,
        "number" | "float" | "boolean" => Color::Yellow,
        // The grammar tags comments with both `@comment` and `@spell`
        // (`(comment) @comment @spell`) -- `@spell` is meant as metadata
        // for a spell-checker, not a distinct visual category, but
        // whichever of the two capture actually fires isn't guaranteed, so
        // both map to the same color rather than risk `@spell` shadowing
        // `@comment` (or vice versa) with something else.
        "comment" | "spell" => Color::DarkGray,
        "type" | "type.builtin" | "type.qualifier" | "storageclass" => Color::Cyan,
        "function.call" => Color::Blue,
        "field" | "variable" | "parameter" => Color::White,
        "punctuation.bracket" | "punctuation.delimiter" | "operator" | "attribute" => Color::Gray,
        _ => Color::Reset,
    }
}

/// One `Color` per byte offset in `text`. `None` if parsing/highlighting
/// failed outright (malformed UTF-8 handling aside, tree-sitter tolerates
/// syntax errors by producing partial highlights, so this should be rare in
/// practice -- callers fall back to unstyled text either way).
fn byte_colors(text: &str) -> Option<Vec<Color>> {
    let mut highlighter = Highlighter::new();
    let events = highlighter
        .highlight(sql_config(), text.as_bytes(), None, |_| None)
        .ok()?;

    let mut colors = vec![Color::Reset; text.len()];
    let mut stack: Vec<Color> = Vec::new();
    for event in events {
        match event.ok()? {
            HighlightEvent::HighlightStart(Highlight(idx)) => {
                stack.push(color_for(HIGHLIGHT_NAMES[idx]));
            }
            HighlightEvent::HighlightEnd => {
                stack.pop();
            }
            HighlightEvent::Source { start, end } => {
                if let Some(&color) = stack.last() {
                    colors[start..end].fill(color);
                }
            }
        }
    }
    Some(colors)
}

/// One `Color` per **character** in `text` (not byte) -- callers index this
/// with `char` positions, matching `QueryEditorComponent`'s `Vec<Vec<char>>`
/// buffer.
pub fn char_colors(text: &str) -> Option<Vec<Color>> {
    let byte_colors = byte_colors(text)?;
    Some(text.char_indices().map(|(b, _)| byte_colors[b]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_and_a_string_literal_get_different_colors() {
        let colors = char_colors("select 'x' from users").unwrap();

        let select_color = colors[0];
        let string_color = colors[7]; // the `'` opening the string literal
        assert_ne!(select_color, Color::Reset);
        assert_ne!(string_color, Color::Reset);
        assert_ne!(select_color, string_color);
    }

    #[test]
    fn output_length_matches_the_character_count_not_the_byte_count() {
        let text = "select 'é'"; // é is 2 bytes, 1 char
        let colors = char_colors(text).unwrap();

        assert_eq!(colors.len(), text.chars().count());
    }
}
