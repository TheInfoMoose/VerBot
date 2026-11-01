use anyhow::{anyhow, Context, Result};
use ratatui::style::{Color, Modifier, Style};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Canonical RGB color tuple with hex formatting and conversion helpers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct RgbColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl RgbColor {
    pub const fn new(r: u8, g: u8, b: u8) -> Self {
        Self { r, g, b }
    }

    pub fn from_hex(hex: &str) -> Result<Self> {
        let trimmed = hex.trim().trim_start_matches('#');
        if trimmed.len() != 6 {
            return Err(anyhow!("Invalid hex color length '{}': expected #RRGGBB", hex));
        }
        let r = u8::from_str_radix(&trimmed[0..2], 16)
            .with_context(|| format!("Invalid red component in '{}'", hex))?;
        let g = u8::from_str_radix(&trimmed[2..4], 16)
            .with_context(|| format!("Invalid green component in '{}'", hex))?;
        let b = u8::from_str_radix(&trimmed[4..6], 16)
            .with_context(|| format!("Invalid blue component in '{}'", hex))?;
        Ok(Self { r, g, b })
    }

    pub fn to_hex(self) -> String {
        format!("#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }

    pub const fn to_ratatui(self) -> Color {
        Color::Rgb(self.r, self.g, self.b)
    }

    /// Blends `self` towards `other` by `factor` (0.0 = self, 1.0 = other).
    pub fn blend(self, other: Self, factor: f32) -> Self {
        let t = factor.clamp(0.0, 1.0);
        let r = (self.r as f32 + (other.r as f32 - self.r as f32) * t).round() as u8;
        let g = (self.g as f32 + (other.g as f32 - self.g as f32) * t).round() as u8;
        let b = (self.b as f32 + (other.b as f32 - self.b as f32) * t).round() as u8;
        Self { r, g, b }
    }
}

/// Official NovOS Cosmic Nova Orange & Deep Obsidian Palette Constants.
pub mod novos_palette {
    use super::RgbColor;

    /// Deep Obsidian Background: `#0B0C10` (`rgb(11, 12, 16)`)
    pub const DEEP_OBSIDIAN_BG: RgbColor = RgbColor::new(0x0B, 0x0C, 0x10);
    /// Deepest Obsidian Backdrop: `#07080B` (`rgb(7, 8, 11)`)
    pub const DEEPEST_BACKDROP: RgbColor = RgbColor::new(0x07, 0x08, 0x0B);
    /// Cosmic Space Surface: `#121521` (`rgb(18, 21, 33)`)
    pub const COSMIC_SPACE_SURFACE: RgbColor = RgbColor::new(0x12, 0x15, 0x21);
    /// Cosmic Secondary Surface: `#0F121C` (`rgb(15, 18, 28)`)
    pub const COSMIC_SURFACE_ALT: RgbColor = RgbColor::new(0x0F, 0x12, 0x1C);
    /// Elevated Border Slate (Step 6 high-contrast inactive border): `#2A314A` (`rgb(42, 49, 74)`)
    pub const ELEVATED_BORDER_SLATE: RgbColor = RgbColor::new(0x2A, 0x31, 0x4A);
    /// Inactive Header Badge Slate: `#9BA8CC` (`rgb(155, 168, 204)`)
    pub const INACTIVE_BADGE_SLATE: RgbColor = RgbColor::new(0x9B, 0xA8, 0xCC);
    /// Primary Accent (COSMIC Nova Orange): `#FF6B1A` (`rgb(255, 107, 26)`)
    pub const COSMIC_NOVA_ORANGE: RgbColor = RgbColor::new(0xFF, 0x6B, 0x1A);
    /// Secondary Accent (Accretion Amber): `#FF9E44` (`rgb(255, 158, 68)`)
    pub const ACCRETION_AMBER: RgbColor = RgbColor::new(0xFF, 0x9E, 0x44);
    /// Tertiary Accent (Warm Flare Amber): `#FFB347` (`rgb(255, 179, 71)`)
    pub const WARM_FLARE_AMBER: RgbColor = RgbColor::new(0xFF, 0xB3, 0x47);
    /// Core Highlight (Starlight Warm White): `#FFF3E0` (`rgb(255, 243, 224)`)
    pub const STARLIGHT_WARM_WHITE: RgbColor = RgbColor::new(0xFF, 0xF3, 0xE0);
    /// Primary Text (Starfield Cool White): `#D8E2FF` (`rgb(216, 226, 255)`)
    pub const STARFIELD_COOL_WHITE: RgbColor = RgbColor::new(0xD8, 0xE2, 0xFF);
    /// Muted Placeholder Slate: `#5A6588` (`rgb(90, 101, 136)`)
    pub const PLACEHOLDER_SLATE_MUTED: RgbColor = RgbColor::new(0x5A, 0x65, 0x88);
}

/// Built-in and user-configurable theme presets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ThemePreset {
    NovosCosmicDark,
    CosmicSystemAuto,
    NovosSolarLight,
    TerminalAnsi,
    Custom,
}

impl ThemePreset {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NovosCosmicDark => "NovOS Cosmic Dark",
            Self::CosmicSystemAuto => "COSMIC System Auto",
            Self::NovosSolarLight => "NovOS Solar Light",
            Self::TerminalAnsi => "Terminal ANSI",
            Self::Custom => "Custom (theme.toml)",
        }
    }

    pub fn from_name(name: &str) -> Self {
        match name.trim().to_ascii_lowercase().as_str() {
            "novos cosmic dark" | "novos-cosmic-dark" | "novos" | "default" | "dark" => {
                Self::NovosCosmicDark
            }
            "cosmic system auto" | "cosmic-system-auto" | "cosmic" | "auto" | "system" => {
                Self::CosmicSystemAuto
            }
            "novos solar light" | "novos-solar-light" | "light" | "solar" => {
                Self::NovosSolarLight
            }
            "terminal ansi" | "terminal-ansi" | "ansi" | "terminal" => Self::TerminalAnsi,
            _ => Self::Custom,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::NovosCosmicDark => Self::CosmicSystemAuto,
            Self::CosmicSystemAuto => Self::NovosSolarLight,
            Self::NovosSolarLight => Self::TerminalAnsi,
            Self::TerminalAnsi => Self::NovosCosmicDark,
            Self::Custom => Self::NovosCosmicDark,
        }
    }
}

/// Resolved Verbocity theme used by the TUI and branding header.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Theme {
    pub preset: ThemePreset,
    pub name: String,
    pub is_dark: bool,
    pub use_ansi: bool,
    pub bg_primary: RgbColor,
    pub bg_deep: RgbColor,
    pub surface_primary: RgbColor,
    pub surface_secondary: RgbColor,
    pub border_slate: RgbColor,
    pub accent_primary: RgbColor,
    pub accent_secondary: RgbColor,
    pub accent_tertiary: RgbColor,
    pub highlight_core: RgbColor,
    pub text_primary: RgbColor,
    pub placeholder_muted: RgbColor,
    pub title_inactive: RgbColor,
    pub badge_inactive: RgbColor,
}

impl Default for Theme {
    fn default() -> Self {
        Self::novos_cosmic_dark()
    }
}

impl Theme {
    /// Default NovOS Cosmic Nova Orange & Deep Obsidian theme.
    pub fn novos_cosmic_dark() -> Self {
        use novos_palette::*;
        Self {
            preset: ThemePreset::NovosCosmicDark,
            name: ThemePreset::NovosCosmicDark.as_str().to_string(),
            is_dark: true,
            use_ansi: false,
            bg_primary: DEEP_OBSIDIAN_BG,
            bg_deep: DEEPEST_BACKDROP,
            surface_primary: COSMIC_SPACE_SURFACE,
            surface_secondary: COSMIC_SURFACE_ALT,
            border_slate: ELEVATED_BORDER_SLATE,
            accent_primary: COSMIC_NOVA_ORANGE,
            accent_secondary: ACCRETION_AMBER,
            accent_tertiary: WARM_FLARE_AMBER,
            highlight_core: STARLIGHT_WARM_WHITE,
            text_primary: STARFIELD_COOL_WHITE,
            placeholder_muted: PLACEHOLDER_SLATE_MUTED,
            title_inactive: ACCRETION_AMBER,
            badge_inactive: INACTIVE_BADGE_SLATE,
        }
    }

    /// Warm daytime companion theme in the NovOS Nova Orange family.
    pub fn novos_solar_light() -> Self {
        Self {
            preset: ThemePreset::NovosSolarLight,
            name: ThemePreset::NovosSolarLight.as_str().to_string(),
            is_dark: false,
            use_ansi: false,
            bg_primary: RgbColor::new(0xFA, 0xF6, 0xF0),
            bg_deep: RgbColor::new(0xF2, 0xEB, 0xE1),
            surface_primary: RgbColor::new(0xFF, 0xFD, 0xFA),
            surface_secondary: RgbColor::new(0xF5, 0xEE, 0xE4),
            border_slate: RgbColor::new(0xD5, 0xC8, 0xB8),
            accent_primary: novos_palette::COSMIC_NOVA_ORANGE,
            accent_secondary: RgbColor::new(0xE0, 0x55, 0x00),
            accent_tertiary: RgbColor::new(0xC8, 0x7A, 0x18),
            highlight_core: RgbColor::new(0x1E, 0x22, 0x33),
            text_primary: RgbColor::new(0x12, 0x15, 0x21),
            placeholder_muted: RgbColor::new(0x7A, 0x70, 0x67),
            title_inactive: RgbColor::new(0xE0, 0x55, 0x00),
            badge_inactive: RgbColor::new(0x5C, 0x53, 0x46),
        }
    }

    /// Terminal ANSI passthrough theme for 16-color or transparent terminal setups.
    pub fn terminal_ansi() -> Self {
        let mut theme = Self::novos_cosmic_dark();
        theme.preset = ThemePreset::TerminalAnsi;
        theme.name = ThemePreset::TerminalAnsi.as_str().to_string();
        theme.use_ansi = true;
        theme
    }

    /// Resolves a theme by preset, inspecting the local COSMIC DE configuration when `CosmicSystemAuto`.
    pub fn from_preset(preset: ThemePreset) -> Self {
        match preset {
            ThemePreset::NovosCosmicDark => Self::novos_cosmic_dark(),
            ThemePreset::CosmicSystemAuto => {
                let cosmic_dir = default_cosmic_config_dir();
                Self::load_cosmic_system_theme(cosmic_dir.as_deref())
            }
            ThemePreset::NovosSolarLight => Self::novos_solar_light(),
            ThemePreset::TerminalAnsi => Self::terminal_ansi(),
            ThemePreset::Custom => Self::load_user_or_default(None),
        }
    }

    /// Loads user theme configuration from `~/.config/verbot/theme.toml` if present,
    /// otherwise returns the default NovOS Cosmic Dark theme.
    pub fn load_user_or_default(custom_config_path: Option<&Path>) -> Self {
        let path = custom_config_path
            .map(PathBuf::from)
            .or_else(default_verbot_theme_path);
        if let Some(ref cfg_path) = path {
            if cfg_path.is_file() {
                if let Ok(theme) = Self::load_from_toml_file(cfg_path) {
                    return theme;
                }
            }
        }
        Self::novos_cosmic_dark()
    }

    /// Loads and applies overrides from a TOML file (`theme.toml`).
    pub fn load_from_toml_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed to read theme config at {}", path.display()))?;
        Self::load_from_toml_str(&raw)
    }

    /// Parses a `ThemeConfig` from a TOML string and overlays any custom hex colors onto the base preset.
    pub fn load_from_toml_str(raw: &str) -> Result<Self> {
        let cfg: ThemeConfig = toml::from_str(raw).context("Failed to parse theme.toml")?;
        let base_preset = cfg
            .preset
            .as_deref()
            .map(ThemePreset::from_name)
            .unwrap_or(ThemePreset::NovosCosmicDark);

        let mut theme = match base_preset {
            ThemePreset::Custom => Self::novos_cosmic_dark(),
            other => Self::from_preset(other),
        };

        let mut has_custom_color = false;
        if let Some(ref hex) = cfg.bg_primary {
            theme.bg_primary = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.bg_deep {
            theme.bg_deep = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.surface_primary {
            theme.surface_primary = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.surface_secondary {
            theme.surface_secondary = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.border_slate {
            theme.border_slate = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.accent_primary {
            theme.accent_primary = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.accent_secondary {
            theme.accent_secondary = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.accent_tertiary {
            theme.accent_tertiary = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.highlight_core {
            theme.highlight_core = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.text_primary {
            theme.text_primary = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.placeholder_muted {
            theme.placeholder_muted = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.title_inactive {
            theme.title_inactive = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(ref hex) = cfg.badge_inactive {
            theme.badge_inactive = RgbColor::from_hex(hex)?;
            has_custom_color = true;
        }
        if let Some(is_dark) = cfg.is_dark {
            theme.is_dark = is_dark;
        }
        if let Some(use_ansi) = cfg.use_ansi {
            theme.use_ansi = use_ansi;
        }
        if let Some(name) = cfg.name {
            theme.name = name;
        } else if has_custom_color && base_preset == ThemePreset::Custom {
            theme.preset = ThemePreset::Custom;
            theme.name = ThemePreset::Custom.as_str().to_string();
        }

        Ok(theme)
    }

    /// Exports the current theme to a `ThemeConfig` serializable as `theme.toml`.
    pub fn to_config(&self) -> ThemeConfig {
        ThemeConfig {
            preset: Some(self.preset.as_str().to_string()),
            name: Some(self.name.clone()),
            is_dark: Some(self.is_dark),
            use_ansi: Some(self.use_ansi),
            bg_primary: Some(self.bg_primary.to_hex()),
            bg_deep: Some(self.bg_deep.to_hex()),
            surface_primary: Some(self.surface_primary.to_hex()),
            surface_secondary: Some(self.surface_secondary.to_hex()),
            border_slate: Some(self.border_slate.to_hex()),
            accent_primary: Some(self.accent_primary.to_hex()),
            accent_secondary: Some(self.accent_secondary.to_hex()),
            accent_tertiary: Some(self.accent_tertiary.to_hex()),
            highlight_core: Some(self.highlight_core.to_hex()),
            text_primary: Some(self.text_primary.to_hex()),
            placeholder_muted: Some(self.placeholder_muted.to_hex()),
            title_inactive: Some(self.title_inactive.to_hex()),
            badge_inactive: Some(self.badge_inactive.to_hex()),
        }
    }

    /// Inspects COSMIC DE RON files in `~/.config/cosmic/` and constructs a synchronized theme,
    /// using the NovOS Cosmic Nova Orange palette as the baseline for any unpopulated fields.
    pub fn load_cosmic_system_theme(cosmic_config_dir: Option<&Path>) -> Self {
        let Some(base_dir) = cosmic_config_dir else {
            let mut fallback = Self::novos_cosmic_dark();
            fallback.preset = ThemePreset::CosmicSystemAuto;
            fallback.name = "COSMIC System Auto (NovOS Default)".to_string();
            return fallback;
        };

        // 1. Read Mode (`com.system76.CosmicTheme.Mode/v1/is_dark` or `v2/is_dark`)
        let is_dark = read_cosmic_file(base_dir, "com.system76.CosmicTheme.Mode", "is_dark")
            .and_then(|s| parse_ron_bool(&s))
            .unwrap_or(true);

        let mut theme = if is_dark {
            Self::novos_cosmic_dark()
        } else {
            Self::novos_solar_light()
        };
        theme.preset = ThemePreset::CosmicSystemAuto;
        theme.is_dark = is_dark;
        theme.name = if is_dark {
            "COSMIC System Auto (Dark)".to_string()
        } else {
            "COSMIC System Auto (Light)".to_string()
        };

        let builder_ns = if is_dark {
            "com.system76.CosmicTheme.Dark.Builder"
        } else {
            "com.system76.CosmicTheme.Light.Builder"
        };
        let active_ns = if is_dark {
            "com.system76.CosmicTheme.Dark"
        } else {
            "com.system76.CosmicTheme.Light"
        };

        // 2. Accent color from Builder or Active theme (`accent`)
        if let Some(accent) = read_cosmic_color(base_dir, builder_ns, "accent")
            .or_else(|| read_cosmic_color(base_dir, active_ns, "accent"))
        {
            theme.accent_primary = accent;
            // Derive cohesive secondary & tertiary amber/highlight tones if custom accent differs from NovOS default
            if accent != novos_palette::COSMIC_NOVA_ORANGE {
                theme.accent_secondary =
                    accent.blend(novos_palette::STARLIGHT_WARM_WHITE, 0.32);
                theme.accent_tertiary =
                    accent.blend(novos_palette::WARM_FLARE_AMBER, 0.45);
            }
        }

        // 3. Optional background / surface / text_tint overrides from COSMIC RON
        if let Some(bg) = read_cosmic_color(base_dir, builder_ns, "bg_color")
            .or_else(|| read_cosmic_color(base_dir, active_ns, "background"))
        {
            theme.bg_primary = bg;
        }
        if let Some(surface) = read_cosmic_color(base_dir, builder_ns, "primary_container_bg")
            .or_else(|| read_cosmic_color(base_dir, active_ns, "primary"))
        {
            theme.surface_primary = surface;
        }
        if let Some(text) = read_cosmic_color(base_dir, builder_ns, "text_tint")
            .or_else(|| read_cosmic_color(base_dir, active_ns, "text_tint"))
        {
            theme.text_primary = text;
        }

        theme
    }

    // --- Ratatui Color Accessors (respecting `use_ansi`) ---

    pub fn bg_color(&self) -> Color {
        if self.use_ansi {
            Color::Reset
        } else {
            self.bg_primary.to_ratatui()
        }
    }

    pub fn surface_color(&self) -> Color {
        if self.use_ansi {
            Color::Reset
        } else {
            self.surface_primary.to_ratatui()
        }
    }

    pub fn border_color(&self) -> Color {
        if self.use_ansi {
            Color::DarkGray
        } else {
            self.border_slate.to_ratatui()
        }
    }

    pub fn accent_color(&self) -> Color {
        if self.use_ansi {
            Color::LightRed
        } else {
            self.accent_primary.to_ratatui()
        }
    }

    pub fn secondary_accent_color(&self) -> Color {
        if self.use_ansi {
            Color::Yellow
        } else {
            self.accent_secondary.to_ratatui()
        }
    }

    pub fn tertiary_accent_color(&self) -> Color {
        if self.use_ansi {
            Color::LightYellow
        } else {
            self.accent_tertiary.to_ratatui()
        }
    }

    pub fn highlight_color(&self) -> Color {
        if self.use_ansi {
            Color::White
        } else {
            self.highlight_core.to_ratatui()
        }
    }

    pub fn text_color(&self) -> Color {
        if self.use_ansi {
            Color::Gray
        } else {
            self.text_primary.to_ratatui()
        }
    }

    pub fn placeholder_color(&self) -> Color {
        if self.use_ansi {
            Color::Gray
        } else {
            self.placeholder_muted.to_ratatui()
        }
    }

    pub fn title_inactive_color(&self) -> Color {
        if self.use_ansi {
            Color::Yellow
        } else {
            self.title_inactive.to_ratatui()
        }
    }

    pub fn badge_inactive_color(&self) -> Color {
        if self.use_ansi {
            Color::Gray
        } else {
            self.badge_inactive.to_ratatui()
        }
    }

    // --- Ready-to-use Ratatui Styles ---

    pub fn base_style(&self) -> Style {
        Style::default().fg(self.text_color()).bg(self.bg_color())
    }

    pub fn surface_style(&self) -> Style {
        Style::default()
            .fg(self.text_color())
            .bg(self.surface_color())
    }

    pub fn border_inactive_style(&self) -> Style {
        Style::default()
            .fg(self.border_color())
            .bg(self.surface_color())
    }

    pub fn border_focused_style(&self) -> Style {
        Style::default()
            .fg(self.accent_color())
            .bg(self.surface_color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn title_focused_style(&self) -> Style {
        Style::default()
            .fg(self.highlight_color())
            .bg(self.surface_color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn title_inactive_style(&self) -> Style {
        Style::default()
            .fg(self.title_inactive_color())
            .bg(self.surface_color())
            .add_modifier(Modifier::BOLD)
    }

    pub fn badge_inactive_style(&self) -> Style {
        Style::default()
            .fg(self.badge_inactive_color())
            .bg(self.surface_color())
    }

    pub fn badge_focused_style(&self) -> Style {
        Style::default()
            .fg(self.tertiary_accent_color())
            .bg(self.surface_color())
            .add_modifier(Modifier::BOLD)
    }
}

/// Serializable TOML structure for `~/.config/verbot/theme.toml`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeConfig {
    pub preset: Option<String>,
    pub name: Option<String>,
    pub is_dark: Option<bool>,
    pub use_ansi: Option<bool>,
    pub bg_primary: Option<String>,
    pub bg_deep: Option<String>,
    pub surface_primary: Option<String>,
    pub surface_secondary: Option<String>,
    pub border_slate: Option<String>,
    pub accent_primary: Option<String>,
    pub accent_secondary: Option<String>,
    pub accent_tertiary: Option<String>,
    pub highlight_core: Option<String>,
    pub text_primary: Option<String>,
    pub placeholder_muted: Option<String>,
    pub title_inactive: Option<String>,
    pub badge_inactive: Option<String>,
}

pub fn default_cosmic_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("cosmic"))
}

pub fn default_verbot_config_dir() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("verbot"))
}

pub fn default_verbot_theme_path() -> Option<PathBuf> {
    default_verbot_config_dir().map(|d| d.join("theme.toml"))
}

fn read_cosmic_file(cosmic_base: &Path, namespace: &str, key: &str) -> Option<String> {
    for version in ["v1", "v2"] {
        let candidate = cosmic_base.join(namespace).join(version).join(key);
        if candidate.is_file() {
            if let Ok(content) = fs::read_to_string(&candidate) {
                let trimmed = content.trim();
                if !trimmed.is_empty() {
                    return Some(trimmed.to_string());
                }
            }
        }
    }
    None
}

fn read_cosmic_color(cosmic_base: &Path, namespace: &str, key: &str) -> Option<RgbColor> {
    let content = read_cosmic_file(cosmic_base, namespace, key)?;
    parse_ron_rgb(&content)
}

/// Parses a boolean value from a COSMIC RON snippet (`true` or `false`).
pub fn parse_ron_bool(input: &str) -> Option<bool> {
    match input.trim().to_ascii_lowercase().as_str() {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    }
}

/// Parses a COSMIC RON color tuple/struct such as:
/// `(red: 1.0, green: 0.41960785, blue: 0.101960786)`
/// or nested `base: (red: ..., green: ..., blue: ...)` into an `RgbColor`.
pub fn parse_ron_rgb(input: &str) -> Option<RgbColor> {
    // If there is a `base:` section inside a larger RON struct (like CosmicTheme), prefer the first `red`/`green`/`blue` triple.
    let r = extract_ron_channel(input, &["red", "r"])?;
    let g = extract_ron_channel(input, &["green", "g"])?;
    let b = extract_ron_channel(input, &["blue", "b"])?;

    Some(RgbColor {
        r: float_or_u8_to_byte(r),
        g: float_or_u8_to_byte(g),
        b: float_or_u8_to_byte(b),
    })
}

fn extract_ron_channel(input: &str, keys: &[&str]) -> Option<f64> {
    for token in input.split(|c: char| c == '(' || c == ')' || c == ',' || c == '\n' || c == '{' || c == '}') {
        let Some((k, v)) = token.split_once(':') else {
            continue;
        };
        let key_clean = k.trim();
        if keys.iter().any(|candidate| key_clean.eq_ignore_ascii_case(candidate)) {
            let val_clean = v.trim();
            if let Ok(parsed) = val_clean.parse::<f64>() {
                return Some(parsed);
            }
        }
    }
    None
}

fn float_or_u8_to_byte(val: f64) -> u8 {
    if val <= 1.0 {
        (val.clamp(0.0, 1.0) * 255.0).round() as u8
    } else {
        val.round().clamp(0.0, 255.0) as u8
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_novos_default_palette_exact_hex_matches() {
        let theme = Theme::novos_cosmic_dark();
        assert_eq!(theme.bg_primary.to_hex(), "#0B0C10");
        assert_eq!(theme.bg_deep.to_hex(), "#07080B");
        assert_eq!(theme.surface_primary.to_hex(), "#121521");
        assert_eq!(theme.surface_secondary.to_hex(), "#0F121C");
        assert_eq!(theme.border_slate.to_hex(), "#2A314A");
        assert_eq!(theme.accent_primary.to_hex(), "#FF6B1A");
        assert_eq!(theme.accent_secondary.to_hex(), "#FF9E44");
        assert_eq!(theme.accent_tertiary.to_hex(), "#FFB347");
        assert_eq!(theme.highlight_core.to_hex(), "#FFF3E0");
        assert_eq!(theme.text_primary.to_hex(), "#D8E2FF");
        assert_eq!(theme.placeholder_muted.to_hex(), "#5A6588");
        assert_eq!(theme.title_inactive.to_hex(), "#FF9E44");
        assert_eq!(theme.badge_inactive.to_hex(), "#9BA8CC");
    }

    #[test]
    fn test_parse_novos_cosmic_ron_accent() {
        let ron = "(red: 1.0, green: 0.41960785, blue: 0.101960786)";
        let parsed = parse_ron_rgb(ron).expect("should parse RON float color");
        assert_eq!(parsed, RgbColor::new(255, 107, 26));
        assert_eq!(parsed.to_hex(), "#FF6B1A");
    }

    #[test]
    fn test_theme_toml_override_loading() {
        let toml_str = r##"
preset = "NovOS Cosmic Dark"
name = "Custom Nova"
accent_primary = "#FF9E44"
border_slate = "#1E2233"
"##;
        let theme = Theme::load_from_toml_str(toml_str).expect("valid theme.toml");
        assert_eq!(theme.name, "Custom Nova");
        assert_eq!(theme.accent_primary.to_hex(), "#FF9E44");
        assert_eq!(theme.bg_primary.to_hex(), "#0B0C10");
    }

    #[test]
    fn test_cosmic_system_theme_dir_ingestion() {
        let tmp_base = std::env::temp_dir().join(format!("verbot_cosmic_test_{}", std::process::id()));
        let mode_dir = tmp_base.join("com.system76.CosmicTheme.Mode/v1");
        let builder_dir = tmp_base.join("com.system76.CosmicTheme.Dark.Builder/v1");
        fs::create_dir_all(&mode_dir).unwrap();
        fs::create_dir_all(&builder_dir).unwrap();

        fs::write(mode_dir.join("is_dark"), "true\n").unwrap();
        fs::write(
            builder_dir.join("accent"),
            "(red: 1.0, green: 0.41960785, blue: 0.101960786)\n",
        )
        .unwrap();

        let loaded = Theme::load_cosmic_system_theme(Some(&tmp_base));
        assert!(loaded.is_dark);
        assert_eq!(loaded.accent_primary.to_hex(), "#FF6B1A");
        assert_eq!(loaded.bg_primary.to_hex(), "#0B0C10");

        let _ = fs::remove_dir_all(&tmp_base);
    }
}
