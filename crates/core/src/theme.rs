//! Semantic colour tokens for shroud's chrome. The only file allowed to hold colour
//! values; pane content is never themed. Add a token with the change that first draws it.

use crate::render::cell::Style;
use crate::render::color::Color;

#[derive(Clone, Debug, PartialEq, Hash)]
pub struct Theme {
    pub border_active: Color,
    pub border_inactive: Color,
    /// Borders between panes that all get typed keys, while broadcast is on.
    pub border_broadcast: Color,
    pub title_active: Color,
    pub title_inactive: Color,
    pub status_fg: Color,
    pub status_bg: Color,
    pub status_mode: Color,
    pub status_session: Color,
    pub status_tab: Color,
    pub status_tab_active: Color,
    pub status_message: Color,
    pub switcher_border: Color,
    pub switcher_match: Color,
    pub switcher_selected: Color,
    pub badge_error: Color,
    pub badge_done: Color,
    pub overlay_border: Color,
    pub overlay_title: Color,
}

impl Default for Theme {
    // ANSI 0–15 only: Ghostty then applies the user's theme with no colour queries.
    fn default() -> Theme {
        Theme {
            border_active: Color::Palette(4),
            border_inactive: Color::Palette(8),
            border_broadcast: Color::Palette(1),
            title_active: Color::Palette(4),
            title_inactive: Color::Palette(8),
            status_fg: Color::Palette(8),
            status_bg: Color::Default,
            status_mode: Color::Palette(3),
            status_session: Color::Palette(4),
            status_tab: Color::Palette(8),
            status_tab_active: Color::Palette(4),
            status_message: Color::Palette(3),
            switcher_border: Color::Palette(8),
            switcher_match: Color::Palette(3),
            switcher_selected: Color::Palette(4),
            badge_error: Color::Palette(1),
            badge_done: Color::Palette(2),
            overlay_border: Color::Palette(8),
            overlay_title: Color::Palette(4),
        }
    }
}

impl Theme {
    pub fn border(&self, active: bool) -> Style {
        Style::fg(if active { self.border_active } else { self.border_inactive })
    }

    /// The theme a broadcasting tab is drawn with: every pane gets the keys, so every border
    /// but the focused pane's says so. Focus keeps its own colour because the cursor, visual
    /// mode and the mouse still follow it.
    pub fn broadcasting(&self) -> Theme {
        Theme { border_inactive: self.border_broadcast, ..self.clone() }
    }

    pub fn title(&self, active: bool) -> Style {
        Style { bold: active, ..Style::fg(if active { self.title_active } else { self.title_inactive }) }
    }

    pub fn status(&self) -> Style {
        Style { fg: self.status_fg, bg: self.status_bg, ..Style::PLAIN }
    }

    pub fn status_with(&self, fg: Color) -> Style {
        Style { fg, ..self.status() }
    }

    /// The mode tag is reversed so it reads as a state, not as text.
    pub fn status_mode(&self) -> Style {
        Style { fg: self.status_mode, bg: self.status_bg, inverse: true, bold: true, ..Style::PLAIN }
    }

    /// A token name or a colour value, for plugins that bring their own palette.
    pub fn resolve(&self, name: &str) -> Option<Color> {
        self.token(name).or_else(|| Theme::parse_color(name).ok())
    }

    pub fn token(&self, key: &str) -> Option<Color> {
        self.clone().token_mut(key).copied()
    }

    pub fn token_mut(&mut self, key: &str) -> Option<&mut Color> {
        Some(match key {
            "border.active" => &mut self.border_active,
            "border.inactive" => &mut self.border_inactive,
            "border.broadcast" => &mut self.border_broadcast,
            "title.active" => &mut self.title_active,
            "title.inactive" => &mut self.title_inactive,
            "status.fg" => &mut self.status_fg,
            "status.bg" => &mut self.status_bg,
            "status.mode" => &mut self.status_mode,
            "status.session" => &mut self.status_session,
            "status.tab" => &mut self.status_tab,
            "status.tab_active" => &mut self.status_tab_active,
            "status.message" => &mut self.status_message,
            "switcher.border" => &mut self.switcher_border,
            "switcher.match" => &mut self.switcher_match,
            "switcher.selected" => &mut self.switcher_selected,
            "badge.error" => &mut self.badge_error,
            "badge.done" => &mut self.badge_done,
            "overlay.border" => &mut self.overlay_border,
            "overlay.title" => &mut self.overlay_title,
            _ => return None,
        })
    }

    pub fn parse_color(value: &str) -> Result<Color, String> {
        if value == "default" {
            return Ok(Color::Default);
        }
        if let Some(hex) = value.strip_prefix('#')
            && hex.len() == 6
            && let Ok(rgb) = u32::from_str_radix(hex, 16)
        {
            return Ok(Color::Rgb((rgb >> 16) as u8, (rgb >> 8) as u8, rgb as u8));
        }
        match value.parse::<i64>() {
            Ok(n) => Theme::palette(n),
            Err(_) => Err(format!("`{value}` is not \"default\", a palette index or \"#rrggbb\"")),
        }
    }

    pub fn palette(index: i64) -> Result<Color, String> {
        u8::try_from(index).map(Color::Palette).map_err(|_| format!("palette index {index} is outside 0-255"))
    }

    pub const TOKENS: &[&str] = &[
        "border.active",
        "border.inactive",
        "border.broadcast",
        "title.active",
        "title.inactive",
        "status.fg",
        "status.bg",
        "status.mode",
        "status.session",
        "status.tab",
        "status.tab_active",
        "status.message",
        "switcher.border",
        "switcher.match",
        "switcher.selected",
        "badge.error",
        "badge.done",
        "overlay.border",
        "overlay.title",
    ];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_theme_uses_only_default_and_16_colours() {
        let mut t = Theme::default();
        for key in Theme::TOKENS {
            let c = *t.token_mut(key).expect("every listed token resolves");
            assert!(matches!(c, Color::Default | Color::Palette(0..=15)), "{key} = {c:?}");
        }
    }

    #[test]
    fn token_lookup_by_config_key() {
        let mut t = Theme::default();
        *t.token_mut("border.active").unwrap() = Color::Rgb(1, 2, 3);
        assert_eq!(t.border_active, Color::Rgb(1, 2, 3));
        assert!(t.token_mut("border.nope").is_none());
    }

    #[test]
    fn broadcasting_recolours_only_the_unfocused_borders() {
        let t = Theme::default();
        let b = t.broadcasting();
        assert_eq!(b.border(false), Style::fg(t.border_broadcast));
        assert_eq!(b.border(true), t.border(true));
        assert_eq!(Theme { border_inactive: t.border_inactive, ..b }, t);
    }

    #[test]
    fn resolve_takes_tokens_then_colour_values() {
        let t = Theme::default();
        assert_eq!(t.resolve("badge.error"), Some(t.badge_error));
        assert_eq!(t.resolve("#1a1a1a"), Some(Color::Rgb(0x1a, 0x1a, 0x1a)));
        assert_eq!(t.resolve("4"), Some(Color::Palette(4)));
        assert_eq!(t.resolve("nope"), None);
    }

    #[test]
    fn colour_values_from_config() {
        assert_eq!(Theme::parse_color("default"), Ok(Color::Default));
        assert_eq!(Theme::parse_color("#7aa2f7"), Ok(Color::Rgb(0x7a, 0xa2, 0xf7)));
        assert_eq!(Theme::parse_color("12"), Ok(Color::Palette(12)));
        assert!(Theme::parse_color("#7aa2f").is_err());
        assert!(Theme::parse_color("blue").is_err());
        assert!(Theme::palette(256).is_err());
    }
}
