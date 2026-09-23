//! The app's semantic color palette. Every component asks this module for
//! colors by *role* (`border_focused`, `error`, `syntax_keyword`, ...)
//! rather than naming a `Color` inline, so the whole TUI can be recolored
//! from one place -- and so a user's `config.toml` can override any single
//! role without the components knowing config exists.
//!
//! Loaded once at startup by `crate::config::init`; every read goes through
//! `theme()`, which falls back to the built-in dark palette when nothing was
//! loaded (tests, or a missing/broken config file).

use std::collections::HashMap;
use std::str::FromStr;
use std::sync::OnceLock;

use ratatui::style::Color;

static THEME: OnceLock<Theme> = OnceLock::new();

/// The active palette -- the built-in default until `set_theme` runs.
pub fn theme() -> &'static Theme {
    THEME.get_or_init(Theme::default)
}

/// Installs `theme` as the process-wide palette. Only the first call wins
/// (the app calls this once, before the first frame); later calls are
/// ignored rather than racing a half-drawn screen into a new palette.
pub fn set_theme(theme: Theme) {
    let _ = THEME.set(theme);
}

/// Colors by role. Names describe *what is being drawn*, never the color
/// itself -- `border_focused`, not `bright_blue`.
#[derive(Debug, Clone, PartialEq)]
pub struct Theme {
    /// Panel borders and their titles, dim when unfocused.
    pub border: Color,
    pub border_focused: Color,
    pub title: Color,
    pub title_focused: Color,

    /// Body text, and the dimmer variant for secondary/hint text.
    pub text: Color,
    pub text_dim: Color,

    /// The selected row in any list (connection picker, schema sidebar,
    /// results, history).
    pub selection_bg: Color,
    pub selection_fg: Color,

    pub error: Color,
    pub warning: Color,
    pub accent: Color,

    /// The bottom status/hint bar, and the key names highlighted in it.
    pub status_bar_bg: Color,
    pub status_bar_fg: Color,
    pub status_key: Color,

    /// The tab bar, shown only when more than one tab is open.
    pub tab_active_bg: Color,
    pub tab_active_fg: Color,
    pub tab_inactive: Color,

    /// SQL syntax highlighting (see `tradar-query-workbench`'s
    /// `sql_highlight`), which maps tree-sitter capture names onto these.
    pub syntax_keyword: Color,
    pub syntax_string: Color,
    pub syntax_number: Color,
    pub syntax_comment: Color,
    pub syntax_type: Color,
    pub syntax_function: Color,
    pub syntax_variable: Color,
    pub syntax_punctuation: Color,
}

impl Default for Theme {
    /// A dark palette built from the 256-color cube rather than truecolor,
    /// so it degrades gracefully on terminals without 24-bit color.
    fn default() -> Self {
        Self {
            border: Color::Indexed(240),
            border_focused: Color::Indexed(75),
            title: Color::Indexed(245),
            title_focused: Color::Indexed(81),

            text: Color::Indexed(253),
            text_dim: Color::Indexed(244),

            selection_bg: Color::Indexed(238),
            selection_fg: Color::Indexed(231),

            error: Color::Indexed(203),
            warning: Color::Indexed(215),
            accent: Color::Indexed(114),

            status_bar_bg: Color::Indexed(236),
            status_bar_fg: Color::Indexed(250),
            status_key: Color::Indexed(180),

            tab_active_bg: Color::Indexed(75),
            tab_active_fg: Color::Indexed(232),
            tab_inactive: Color::Indexed(244),

            syntax_keyword: Color::Indexed(176),
            syntax_string: Color::Indexed(114),
            syntax_number: Color::Indexed(215),
            syntax_comment: Color::Indexed(243),
            syntax_type: Color::Indexed(80),
            syntax_function: Color::Indexed(75),
            syntax_variable: Color::Indexed(253),
            syntax_punctuation: Color::Indexed(245),
        }
    }
}

impl Theme {
    /// The "Dracula" preset (<https://draculatheme.com>), true 24-bit hex
    /// straight from its published palette -- unlike `default()`'s indexed
    /// colors (chosen for terminals without truecolor support), a *named*
    /// preset is picked specifically to look like the real thing, and
    /// truecolor is common enough now that that trade favors fidelity.
    pub fn dracula() -> Self {
        let background = Color::Rgb(40, 42, 54);
        let current_line = Color::Rgb(68, 71, 90);
        let foreground = Color::Rgb(248, 248, 242);
        let comment = Color::Rgb(98, 114, 164);
        let cyan = Color::Rgb(139, 233, 253);
        let green = Color::Rgb(80, 250, 123);
        let orange = Color::Rgb(255, 184, 108);
        let pink = Color::Rgb(255, 121, 198);
        let purple = Color::Rgb(189, 147, 249);
        let red = Color::Rgb(255, 85, 85);
        let yellow = Color::Rgb(241, 250, 140);

        Self {
            border: comment,
            border_focused: purple,
            title: comment,
            title_focused: cyan,

            text: foreground,
            text_dim: comment,

            selection_bg: current_line,
            selection_fg: foreground,

            error: red,
            warning: orange,
            accent: green,

            status_bar_bg: current_line,
            status_bar_fg: foreground,
            status_key: yellow,

            tab_active_bg: purple,
            tab_active_fg: background,
            tab_inactive: comment,

            syntax_keyword: pink,
            syntax_string: green,
            syntax_number: orange,
            syntax_comment: comment,
            syntax_type: cyan,
            syntax_function: purple,
            syntax_variable: foreground,
            syntax_punctuation: foreground,
        }
    }

    /// The "Nord" preset (<https://www.nordtheme.com>), same true-hex
    /// reasoning as `dracula()` -- role choices follow the palette's own
    /// Polar Night/Snow Storm/Frost/Aurora groupings (see the project's
    /// "Colors and Palettes" docs) rather than reusing `default()`'s
    /// indexed values.
    pub fn nord() -> Self {
        let polar0 = Color::Rgb(46, 52, 64);
        let polar1 = Color::Rgb(59, 66, 82);
        let polar3 = Color::Rgb(76, 86, 106);
        let snow4 = Color::Rgb(216, 222, 233);
        let snow6 = Color::Rgb(236, 239, 244);
        let frost7 = Color::Rgb(143, 188, 187);
        let frost8 = Color::Rgb(136, 192, 208);
        let frost9 = Color::Rgb(129, 161, 193);
        let aurora_red = Color::Rgb(191, 97, 106);
        let aurora_yellow = Color::Rgb(235, 203, 139);
        let aurora_green = Color::Rgb(163, 190, 140);
        let aurora_purple = Color::Rgb(180, 142, 173);

        Self {
            border: polar3,
            border_focused: frost8,
            title: polar3,
            title_focused: frost9,

            text: snow6,
            text_dim: snow4,

            selection_bg: Color::Rgb(67, 76, 94),
            selection_fg: snow6,

            error: aurora_red,
            warning: aurora_yellow,
            accent: aurora_green,

            status_bar_bg: polar1,
            status_bar_fg: snow4,
            status_key: aurora_yellow,

            tab_active_bg: frost8,
            tab_active_fg: polar0,
            tab_inactive: polar3,

            syntax_keyword: frost9,
            syntax_string: aurora_green,
            syntax_number: aurora_purple,
            syntax_comment: polar3,
            syntax_type: frost7,
            syntax_function: frost8,
            syntax_variable: snow4,
            syntax_punctuation: snow4,
        }
    }

    /// Overrides individual roles from `[theme]` in `config.toml`. Keys are
    /// the field names above in kebab-case (`border-focused`,
    /// `syntax-keyword`, ...); values are anything `ratatui`'s `Color`
    /// parses -- a name (`"red"`, `"bright-blue"`), `"#rrggbb"`, or a
    /// 256-color index (`"75"`).
    ///
    /// An unknown key or unparseable color is an error rather than a silent
    /// no-op: a typo'd role name would otherwise look like "my config
    /// didn't work" with nothing to explain why.
    pub fn apply_overrides(&mut self, overrides: &HashMap<String, String>) -> anyhow::Result<()> {
        for (key, value) in overrides {
            let color = Color::from_str(value)
                .map_err(|_| anyhow::anyhow!("theme.{key}: '{value}' is not a valid color"))?;
            let slot = self
                .slot_mut(key)
                .ok_or_else(|| anyhow::anyhow!("theme.{key}: unknown theme key"))?;
            *slot = color;
        }
        Ok(())
    }

    fn slot_mut(&mut self, key: &str) -> Option<&mut Color> {
        Some(match key {
            "border" => &mut self.border,
            "border-focused" => &mut self.border_focused,
            "title" => &mut self.title,
            "title-focused" => &mut self.title_focused,
            "text" => &mut self.text,
            "text-dim" => &mut self.text_dim,
            "selection-bg" => &mut self.selection_bg,
            "selection-fg" => &mut self.selection_fg,
            "error" => &mut self.error,
            "warning" => &mut self.warning,
            "accent" => &mut self.accent,
            "status-bar-bg" => &mut self.status_bar_bg,
            "status-bar-fg" => &mut self.status_bar_fg,
            "status-key" => &mut self.status_key,
            "tab-active-bg" => &mut self.tab_active_bg,
            "tab-active-fg" => &mut self.tab_active_fg,
            "tab-inactive" => &mut self.tab_inactive,
            "syntax-keyword" => &mut self.syntax_keyword,
            "syntax-string" => &mut self.syntax_string,
            "syntax-number" => &mut self.syntax_number,
            "syntax-comment" => &mut self.syntax_comment,
            "syntax-type" => &mut self.syntax_type,
            "syntax-function" => &mut self.syntax_function,
            "syntax-variable" => &mut self.syntax_variable,
            "syntax-punctuation" => &mut self.syntax_punctuation,
            _ => return None,
        })
    }

    /// The preset named `name` in `[theme]`'s `preset` key, or `None` for an
    /// unrecognized name -- kept next to `dracula()`/`nord()` so the list of
    /// valid names lives in exactly one place.
    pub fn by_preset_name(name: &str) -> Option<Self> {
        match name {
            "dracula" => Some(Self::dracula()),
            "nord" => Some(Self::nord()),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn overrides(pairs: &[(&str, &str)]) -> HashMap<String, String> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn an_override_replaces_only_the_named_role() {
        let mut theme = Theme::default();
        let default_border = theme.border;

        theme
            .apply_overrides(&overrides(&[("error", "#ff0000")]))
            .unwrap();

        assert_eq!(theme.error, Color::Rgb(255, 0, 0));
        assert_eq!(
            theme.border, default_border,
            "other roles must be untouched"
        );
    }

    #[test]
    fn colors_can_be_named_hex_or_indexed() {
        let mut theme = Theme::default();

        theme
            .apply_overrides(&overrides(&[
                ("error", "red"),
                ("warning", "#00ff00"),
                ("accent", "42"),
            ]))
            .unwrap();

        assert_eq!(theme.error, Color::Red);
        assert_eq!(theme.warning, Color::Rgb(0, 255, 0));
        assert_eq!(theme.accent, Color::Indexed(42));
    }

    #[test]
    fn an_unknown_key_is_an_error_rather_than_a_silent_no_op() {
        let mut theme = Theme::default();

        let err = theme
            .apply_overrides(&overrides(&[("bordr-focused", "red")]))
            .unwrap_err();

        assert!(err.to_string().contains("unknown theme key"), "{err}");
    }

    #[test]
    fn an_unparseable_color_is_an_error() {
        let mut theme = Theme::default();

        let err = theme
            .apply_overrides(&overrides(&[("error", "chartreuse-ish")]))
            .unwrap_err();

        assert!(err.to_string().contains("not a valid color"), "{err}");
    }

    #[test]
    fn dracula_and_nord_differ_from_the_default_and_each_other() {
        let default = Theme::default();
        let dracula = Theme::dracula();
        let nord = Theme::nord();

        assert_ne!(dracula, default);
        assert_ne!(nord, default);
        assert_ne!(dracula, nord);
    }

    #[test]
    fn by_preset_name_finds_dracula_and_nord_and_rejects_an_unknown_name() {
        assert_eq!(Theme::by_preset_name("dracula"), Some(Theme::dracula()));
        assert_eq!(Theme::by_preset_name("nord"), Some(Theme::nord()));
        assert_eq!(Theme::by_preset_name("solarized"), None);
    }

    #[test]
    fn an_override_still_applies_on_top_of_a_preset() {
        let mut theme = Theme::dracula();

        theme
            .apply_overrides(&overrides(&[("error", "#ff0000")]))
            .unwrap();

        assert_eq!(theme.error, Color::Rgb(255, 0, 0));
        assert_eq!(
            theme.accent,
            Theme::dracula().accent,
            "roles not named in the override must keep the preset's own color"
        );
    }
}
