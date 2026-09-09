//! UI palette for the Tauri frontend.
//!
//! Port of the defaults in `crates/app/src/theme.rs`, serialized as hex
//! strings so the web UI can apply them as CSS variables. Omarchy sync is
//! read-only with safe fallback to the slate defaults, same contract as the
//! egui `ThemeWatcher` (Linux-only, no-op elsewhere).

use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Hex-string palette sent to the frontend (`#rrggbb` per role).
#[derive(Debug, Clone, Serialize)]
pub struct PaletteDto {
    pub bg: String,
    pub card: String,
    pub raised: String,
    pub hover: String,
    pub grid: String,
    pub border_strong: String,
    pub text: String,
    pub text_dim: String,
    pub blue: String,
    pub blue_dark: String,
    pub blue_soft: String,
    pub red: String,
    pub red_dark: String,
    pub green: String,
    pub amber: String,
}

impl Default for PaletteDto {
    fn default() -> Self {
        Self {
            bg: "#0f141c".into(),
            card: "#171d27".into(),
            raised: "#1d2530".into(),
            hover: "#242e3d".into(),
            grid: "#283141".into(),
            border_strong: "#3b4759".into(),
            text: "#d7dee8".into(),
            text_dim: "#8293a8".into(),
            blue: "#4f8dff".into(),
            blue_dark: "#366bd0".into(),
            blue_soft: "#1b2a43".into(),
            red: "#e5534b".into(),
            red_dark: "#b43d38".into(),
            green: "#2fce72".into(),
            amber: "#e0a94b".into(),
        }
    }
}

/// Resolve the currently active palette: Omarchy theme when available.
pub fn current() -> PaletteDto {
    if !cfg!(target_os = "linux") {
        return PaletteDto::default();
    }
    let slug = state_path().and_then(|p| read_slug(&p.join("omarchy/current/theme.name")));
    let colors = slug.as_deref().and_then(colors_path);
    colors
        .as_deref()
        .and_then(PaletteDto::from_omarchy)
        .unwrap_or_default()
}

impl PaletteDto {
    /// Best-effort build from an Omarchy `colors.toml`; `None` when the file
    /// cannot be read or parsed (caller keeps the default palette).
    pub fn from_omarchy(path: &Path) -> Option<PaletteDto> {
        let text = std::fs::read_to_string(path).ok()?;
        let t: OmarchyToml = toml::from_str(&text).ok()?;

        let mut p = PaletteDto::default();
        let accent = pick(&t.accent).or_else(|| pick(&t.blue));
        let selection = pick(&t.selection);
        let muted = pick(&t.muted);
        let bg = pick(&t.background);
        let dark = pick(&t.dark_background);
        let darker = pick(&t.darker_background);
        let lighter = pick(&t.lighter_background);
        let fg = pick(&t.foreground).or_else(|| pick(&t.bright_foreground));
        let dfg = pick(&t.dark_foreground);

        p.bg = darker.or(dark.clone()).or(bg.clone()).unwrap_or(p.bg);
        p.card = dark.or(bg.clone()).unwrap_or(p.card);
        p.raised = bg.clone().or(lighter.clone()).unwrap_or(p.raised);
        p.hover = lighter.clone().or(bg.clone()).unwrap_or(p.hover);
        p.grid = lighter.clone().or(muted.clone()).unwrap_or(p.grid);
        p.border_strong = muted.clone().or(lighter).unwrap_or(p.border_strong);
        p.text = fg.unwrap_or(p.text);
        p.text_dim = dfg.or(muted).unwrap_or(p.text_dim);
        p.blue = accent.clone().unwrap_or(p.blue);
        p.blue_dark = selection.clone().or(accent).unwrap_or(p.blue_dark);
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

fn pick(value: &Option<String>) -> Option<String> {
    value.as_deref().and_then(parse_hex)
}

fn parse_hex(s: &str) -> Option<String> {
    let hex = s.trim().strip_prefix('#').unwrap_or(s.trim());
    if hex.len() != 6 {
        return None;
    }
    u32::from_str_radix(hex, 16).ok()?;
    Some(format!("#{}", hex.to_lowercase()))
}

fn state_path() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home_dir().map(|h| h.join(".local/state")))
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

fn read_slug(path: &Path) -> Option<String> {
    std::fs::read_to_string(path)
        .ok()
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
}

fn colors_path(slug: &str) -> Option<PathBuf> {
    let config_dir = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| home_dir().map(|h| h.join(".config")))
        .unwrap_or_else(|| PathBuf::from("/etc"));
    let user = config_dir.join("themes").join(slug).join("colors.toml");
    if user.is_file() {
        return Some(user);
    }
    let stock = PathBuf::from("/usr/share/omarchy/themes")
        .join(slug)
        .join("colors.toml");
    stock.is_file().then_some(stock)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn defaults_match_egui_slate() {
        let p = PaletteDto::default();
        assert_eq!(p.bg, "#0f141c");
        assert_eq!(p.blue, "#4f8dff");
    }

    #[test]
    fn omarchy_parser_maps_keys() {
        let mut file = tempfile::NamedTempFile::new().unwrap();
        write!(
            file,
            "accent = \"#7aa2f7\"\nbackground = \"#1a1b26\"\nforeground = \"#a9b1d6\"\n"
        )
        .unwrap();
        let p = PaletteDto::from_omarchy(file.path()).expect("parses");
        assert_eq!(p.blue, "#7aa2f7");
        assert_eq!(p.text, "#a9b1d6");
    }

    #[test]
    fn omarchy_missing_file_yields_none() {
        assert!(PaletteDto::from_omarchy(Path::new("/nonexistent/colors.toml")).is_none());
    }
}
