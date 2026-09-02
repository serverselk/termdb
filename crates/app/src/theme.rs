//! "TERMDB" slate UI theme.
//!
//! Ships with a deep-navy default palette and can hot-swap to an active
//! [Omarchy](https://omarchy.org/) theme's colours (`colors.toml`). All reads
//! are best-effort and read-only: on machines without Omarchy (or non-Linux)
//! the default navy palette is used and nothing is ever modified.

use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use egui::{
    style::WidgetVisuals, Color32, CornerRadius, FontId, Shadow, Stroke, TextStyle, Theme, Visuals,
};
use serde::Deserialize;

// Default slate navy palette (also the fallback for missing Omarchy colours).
pub const BG: Color32 = Color32::from_rgb(0x0f, 0x14, 0x1c);
pub const CARD: Color32 = Color32::from_rgb(0x17, 0x1d, 0x27);
pub const RAISED: Color32 = Color32::from_rgb(0x1d, 0x25, 0x30);
pub const HOVER: Color32 = Color32::from_rgb(0x24, 0x2e, 0x3d);
pub const GRID: Color32 = Color32::from_rgb(0x28, 0x31, 0x41);
pub const BORDER_STRONG: Color32 = Color32::from_rgb(0x3b, 0x47, 0x59);
pub const TEXT: Color32 = Color32::from_rgb(0xd7, 0xde, 0xe8);
pub const TEXT_DIM: Color32 = Color32::from_rgb(0x82, 0x93, 0xa8);
pub const BLUE: Color32 = Color32::from_rgb(0x4f, 0x8d, 0xff);
pub const BLUE_DARK: Color32 = Color32::from_rgb(0x36, 0x6b, 0xd0);
pub const BLUE_SOFT: Color32 = Color32::from_rgb(0x1b, 0x2a, 0x43);
pub const RED: Color32 = Color32::from_rgb(0xe5, 0x53, 0x4b);
pub const RED_DARK: Color32 = Color32::from_rgb(0xb4, 0x3d, 0x38);
pub const GREEN: Color32 = Color32::from_rgb(0x2f, 0xce, 0x72);
pub const AMBER: Color32 = Color32::from_rgb(0xe0, 0xa9, 0x4b);

/// The full set of colours the UI derives from. Read via [`palette()`] so UI
/// modules always render with the currently active (possibly Omarchy) theme.
#[derive(Debug, Clone, PartialEq)]
pub struct Palette {
    pub bg: Color32,
    pub card: Color32,
    pub raised: Color32,
    pub hover: Color32,
    pub grid: Color32,
    pub border_strong: Color32,
    pub text: Color32,
    pub text_dim: Color32,
    pub blue: Color32,
    pub blue_dark: Color32,
    pub blue_soft: Color32,
    pub red: Color32,
    pub red_dark: Color32,
    pub green: Color32,
    pub amber: Color32,
}

impl Default for Palette {
    fn default() -> Self {
        Self {
            bg: BG,
            card: CARD,
            raised: RAISED,
            hover: HOVER,
            grid: GRID,
            border_strong: BORDER_STRONG,
            text: TEXT,
            text_dim: TEXT_DIM,
            blue: BLUE,
            blue_dark: BLUE_DARK,
            blue_soft: BLUE_SOFT,
            red: RED,
            red_dark: RED_DARK,
            green: GREEN,
            amber: AMBER,
        }
    }
}

/// Current active palette, shared with every UI module.
static CURRENT: OnceLock<Mutex<Palette>> = OnceLock::new();

/// The palette UI code should render with right now.
pub fn palette() -> Palette {
    CURRENT
        .get()
        .and_then(|m| m.lock().ok())
        .map(|guard| guard.clone())
        .unwrap_or_default()
}

fn set_current(palette: &Palette) {
    let lock = CURRENT.get_or_init(|| Mutex::new(palette.clone()));
    if let Ok(mut guard) = lock.lock() {
        *guard = palette.clone();
    }
}

/// Apply the default navy palette.
pub fn apply(ctx: &egui::Context) {
    apply_palette(ctx, &Palette::default());
}

/// (Re)apply a palette to the whole app. Cheap enough to call per frame.
pub fn apply_palette(ctx: &egui::Context, palette: &Palette) {
    set_current(palette);
    ctx.set_theme(egui::ThemePreference::Dark);
    let style = egui::Style {
        visuals: visuals(palette),
        ..style()
    };
    ctx.set_style_of(Theme::Dark, style);
}

fn visuals(p: &Palette) -> Visuals {
    let mut v = Visuals::dark();

    v.panel_fill = p.bg;
    v.window_fill = p.bg;
    v.extreme_bg_color = p.raised;
    v.faint_bg_color = p.grid;
    v.code_bg_color = p.raised;
    v.override_text_color = Some(p.text);
    v.hyperlink_color = p.blue;
    v.warn_fg_color = p.amber;
    v.error_fg_color = p.red;

    v.window_corner_radius = CornerRadius::ZERO;
    v.window_shadow = Shadow::NONE;
    v.popup_shadow = Shadow::NONE;
    v.window_stroke = Stroke::new(1.0, p.border_strong);
    v.menu_corner_radius = CornerRadius::ZERO;

    v.selection.bg_fill = p.blue_soft;
    v.selection.stroke = Stroke::new(1.0, p.blue);

    widget(&mut v.widgets.noninteractive, p.card, p.grid, p.text_dim);
    widget(&mut v.widgets.inactive, p.card, p.border_strong, p.text);
    widget(&mut v.widgets.hovered, p.hover, p.blue_dark, p.text);
    widget(&mut v.widgets.active, p.blue_soft, p.blue, p.text);
    v.widgets.open = v.widgets.active;

    v
}

fn widget(w: &mut WidgetVisuals, fill: Color32, border: Color32, text: Color32) {
    w.bg_fill = fill;
    w.weak_bg_fill = fill;
    w.bg_stroke = Stroke::new(1.0, border);
    w.fg_stroke = Stroke::new(1.0, text);
    w.corner_radius = CornerRadius::ZERO;
    w.expansion = 0.0;
}

fn style() -> egui::Style {
    egui::Style {
        text_styles: [
            (TextStyle::Heading, FontId::monospace(16.0)),
            (TextStyle::Body, FontId::monospace(14.0)),
            (TextStyle::Button, FontId::monospace(13.0)),
            (TextStyle::Small, FontId::monospace(12.0)),
            (TextStyle::Monospace, FontId::monospace(13.0)),
        ]
        .into(),
        spacing: egui::Spacing {
            item_spacing: egui::vec2(6.0, 4.0),
            button_padding: egui::vec2(10.0, 4.0),
            interact_size: egui::vec2(28.0, 18.0),
            window_margin: egui::Margin::symmetric(8, 8),
            menu_margin: egui::Margin::symmetric(4, 4),
            scroll: egui::style::ScrollStyle {
                floating: true,
                bar_width: 8.0,
                bar_inner_margin: 2.0,
                bar_outer_margin: 0.0,
                handle_min_length: 20.0,
                ..Default::default()
            },
            ..Default::default()
        },
        ..Default::default()
    }
}

impl Palette {
    /// Best-effort build from an Omarchy `colors.toml`.
    ///
    /// Every absent/unknown field falls back to the default navy value and an
    /// unreadable file yields `None` (caller keeps whatever it had before), so
    /// a partial or broken theme can never break the UI.
    pub fn from_omarchy(path: &Path) -> Option<Palette> {
        let text = std::fs::read_to_string(path).ok()?;
        let t: OmarchyToml = toml::from_str(&text).ok()?;

        let mut p = Palette::default();
        let accent = pick(&t.accent).or_else(|| pick(&t.blue));
        let selection = pick(&t.selection);
        let muted = pick(&t.muted);
        let bg = pick(&t.background);
        let dark = pick(&t.dark_background);
        let darker = pick(&t.darker_background);
        let lighter = pick(&t.lighter_background);
        let fg = pick(&t.foreground).or_else(|| pick(&t.bright_foreground));
        let dfg = pick(&t.dark_foreground);

        p.bg = darker.or(dark).or(bg).unwrap_or(p.bg);
        p.card = dark.or(bg).unwrap_or(p.card);
        p.raised = bg.or(lighter).unwrap_or(p.raised);
        p.hover = lighter.or(bg).unwrap_or(p.hover);
        p.grid = lighter.or(muted).unwrap_or(p.grid);
        p.border_strong = muted.or(lighter).unwrap_or(p.border_strong);
        p.text = fg.unwrap_or(p.text);
        p.text_dim = dfg.or(muted).unwrap_or(p.text_dim);
        p.blue = accent.unwrap_or(p.blue);
        p.blue_dark = selection.or(accent).unwrap_or(p.blue_dark);
        p.blue_soft = selection.unwrap_or(p.blue_soft);
        p.red = pick(&t.red).unwrap_or(p.red);
        p.red_dark = pick(&t.bright_red).or(pick(&t.red)).unwrap_or(p.red_dark);
        p.green = pick(&t.green).unwrap_or(p.green);
        p.amber = pick(&t.yellow).unwrap_or(p.amber);
        Some(p)
    }
}

#[derive(Default, Deserialize)]
struct OmarchyToml {
    // Not read today — kept for forward-compat (light themes map the same keys).
    #[allow(dead_code)]
    mode: Option<String>,
    accent: Option<String>,
    selection: Option<String>,
    muted: Option<String>,
    background: Option<String>,
    dark_background: Option<String>,
    darker_background: Option<String>,
    lighter_background: Option<String>,
    foreground: Option<String>,
    dark_foreground: Option<String>,
    bright_foreground: Option<String>,
    red: Option<String>,
    bright_red: Option<String>,
    green: Option<String>,
    yellow: Option<String>,
    blue: Option<String>,
}

fn pick(value: &Option<String>) -> Option<Color32> {
    value.as_deref().and_then(parse_hex)
}

fn parse_hex(s: &str) -> Option<Color32> {
    let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if hex.len() != 6 {
        return None;
    }
    let raw = u32::from_str_radix(hex, 16).ok()?;
    Some(Color32::from_rgb(
        ((raw >> 16) & 0xff) as u8,
        ((raw >> 8) & 0xff) as u8,
        (raw & 0xff) as u8,
    ))
}

/// Live Omarchy theme follower.
///
/// Linux-only by design — Omarchy ships on Linux. It polls nothing but a
/// couple of lightweight stat/reads once per frame, is fully read-only, and
/// degrades to the default navy palette on any error, missing file, broken
/// TOML, or non-Linux platform. On macOS/Windows `new()` returns a disabled
/// watcher and `refresh` is a no-op.
pub struct ThemeWatcher {
    enabled: bool,
    state_path: PathBuf,
    config_dir: PathBuf,
    last: Option<(Option<String>, Option<std::time::SystemTime>)>,
}

impl ThemeWatcher {
    pub fn new() -> Self {
        let enabled = cfg!(target_os = "linux");
        if !enabled {
            return Self {
                enabled: false,
                state_path: PathBuf::new(),
                config_dir: PathBuf::new(),
                last: None,
            };
        }

        let state_dir = std::env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home_dir().map(|h| h.join(".local/state")))
            .unwrap_or_else(std::env::temp_dir);
        let config_dir = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| home_dir().map(|h| h.join(".config")))
            .unwrap_or_else(|| PathBuf::from("/etc"));

        Self {
            enabled,
            state_path: state_dir.join("omarchy/current/theme.name"),
            config_dir: config_dir.join("omarchy"),
            last: None,
        }
    }

    /// Re-resolve the active Omarchy theme and (re)apply it when it changed.
    pub fn refresh(&mut self, ctx: &egui::Context) {
        if !self.enabled {
            return;
        }
        let slug = read_slug(&self.state_path);
        let colors_path = slug.as_deref().and_then(|slug| self.colors_path(slug));
        let modified = colors_path
            .as_ref()
            .and_then(|p| std::fs::metadata(p).ok())
            .and_then(|m| m.modified().ok());
        let stamp = (slug, modified);
        if self.last.as_ref() == Some(&stamp) {
            return;
        }
        self.last = Some(stamp);

        let palette = colors_path
            .as_deref()
            .and_then(Palette::from_omarchy)
            .unwrap_or_default();
        apply_palette(ctx, &palette);
        ctx.request_repaint();
    }

    /// `~/.config/omarchy/themes/<slug>/colors.toml` (user overlay) wins over
    /// the read-only stock theme.
    fn colors_path(&self, slug: &str) -> Option<PathBuf> {
        let user = self
            .config_dir
            .join("themes")
            .join(slug)
            .join("colors.toml");
        if user.is_file() {
            return Some(user);
        }
        let stock = PathBuf::from("/usr/share/omarchy/themes")
            .join(slug)
            .join("colors.toml");
        stock.is_file().then_some(stock)
    }
}

impl Default for ThemeWatcher {
    fn default() -> Self {
        Self::new()
    }
}

fn read_slug(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn slate_theme_is_flat() {
        let v = visuals(&Palette::default());
        for w in [
            &v.widgets.noninteractive,
            &v.widgets.inactive,
            &v.widgets.hovered,
            &v.widgets.active,
        ] {
            assert_eq!(w.corner_radius, CornerRadius::ZERO);
        }
        assert_eq!(v.window_corner_radius, CornerRadius::ZERO);
        assert_eq!(v.menu_corner_radius, CornerRadius::ZERO);
        assert_eq!(v.window_shadow, Shadow::NONE);
        assert_eq!(v.popup_shadow, Shadow::NONE);
    }

    #[test]
    fn omarchy_parser_maps_keys() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(
            file,
            "mode = \"dark\"\n\
             accent = \"#7aa2f7\"\n\
             selection = \"#292e42\"\n\
             muted = \"#414868\"\n\
             background = \"#1a1b26\"\n\
             dark_background = \"#13141c\"\n\
             darker_background = \"#0e0e14\"\n\
             lighter_background = \"#24283b\"\n\
             foreground = \"#a9b1d6\"\n\
             dark_foreground = \"#565f89\"\n\
             red = \"#f7768e\"\n\
             green = \"#9ece6a\"\n\
             yellow = \"#e0af68\"\n"
        )
        .unwrap();
        let p = Palette::from_omarchy(file.path()).expect("parses");
        assert_eq!(p.bg, Color32::from_rgb(0x0e, 0x0e, 0x14));
        assert_eq!(p.card, Color32::from_rgb(0x13, 0x14, 0x1c));
        assert_eq!(p.raised, Color32::from_rgb(0x1a, 0x1b, 0x26));
        assert_eq!(p.hover, Color32::from_rgb(0x24, 0x28, 0x3b));
        assert_eq!(p.blue, Color32::from_rgb(0x7a, 0xa2, 0xf7));
        assert_eq!(p.blue_soft, Color32::from_rgb(0x29, 0x2e, 0x42));
        assert_eq!(p.text, Color32::from_rgb(0xa9, 0xb1, 0xd6));
        assert_eq!(p.green, Color32::from_rgb(0x9e, 0xce, 0x6a));
        assert_eq!(p.amber, Color32::from_rgb(0xe0, 0xaf, 0x68));
    }

    #[test]
    fn omarchy_missing_fields_fall_back_to_defaults() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(file, "mode = \"dark\"\nforeground = \"#ffffff\"\n").unwrap();
        let p = Palette::from_omarchy(file.path()).expect("parses");
        let d = Palette::default();
        assert_eq!(p.text, Color32::from_rgb(0xff, 0xff, 0xff));
        assert_eq!(p.bg, d.bg);
        assert_eq!(p.blue, d.blue);
    }

    #[test]
    fn omarchy_missing_file_yields_none() {
        let p = Palette::from_omarchy(Path::new("/nonexistent/colors.toml"));
        assert!(p.is_none());
    }
}
