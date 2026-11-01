use crate::models::{EffortLevel, ModelFamily};
use crate::theme::Theme;
use serde::{Deserialize, Serialize};

/// The 4 navigable tabs of the full-screen Settings Page (`Alt+S`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SettingsTab {
    /// Tab 1: Theme & Contrast (`theme.toml` & live hex overrides)
    ThemeAndContrast,
    /// Tab 2: Models & TOML Ingestion (`models.toml`, `.toml`/`.yaml` file ingest, `agy models` sync, custom sub-model form)
    ModelsAndToml,
    /// Tab 3: Templates & Defaults (`config.toml` startup defaults & template ingestion)
    TemplatesAndDefaults,
    /// Tab 4: Execution, Shell Formatting & Workspace Storage (`agy` CLI flags, shell formatting policy, XDG vs `.verbot/`)
    ExecutionAndWorkspace,
}

impl Default for SettingsTab {
    fn default() -> Self {
        Self::ThemeAndContrast
    }
}

impl SettingsTab {
    pub const ALL: [SettingsTab; 4] = [
        Self::ThemeAndContrast,
        Self::ModelsAndToml,
        Self::TemplatesAndDefaults,
        Self::ExecutionAndWorkspace,
    ];

    pub fn index(self) -> usize {
        match self {
            Self::ThemeAndContrast => 0,
            Self::ModelsAndToml => 1,
            Self::TemplatesAndDefaults => 2,
            Self::ExecutionAndWorkspace => 3,
        }
    }

    pub fn from_index(idx: usize) -> Self {
        match idx % 4 {
            0 => Self::ThemeAndContrast,
            1 => Self::ModelsAndToml,
            2 => Self::TemplatesAndDefaults,
            _ => Self::ExecutionAndWorkspace,
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::ThemeAndContrast => "1: Theme & Contrast",
            Self::ModelsAndToml => "2: Models & TOML Ingestion",
            Self::TemplatesAndDefaults => "3: Templates & Defaults",
            Self::ExecutionAndWorkspace => "4: Execution & Workspace",
        }
    }

    pub fn short_title(self) -> &'static str {
        match self {
            Self::ThemeAndContrast => "1:Theme",
            Self::ModelsAndToml => "2:Models",
            Self::TemplatesAndDefaults => "3:Templates",
            Self::ExecutionAndWorkspace => "4:Exec/WS",
        }
    }

    pub fn next(self) -> Self {
        Self::from_index(self.index() + 1)
    }

    pub fn prev(self) -> Self {
        Self::from_index((self.index() + 3) % 4)
    }

    /// Number of interactive setting rows inside this tab.
    pub fn row_count(self) -> usize {
        match self {
            Self::ThemeAndContrast => 8,
            Self::ModelsAndToml => 9,
            Self::TemplatesAndDefaults => 8,
            Self::ExecutionAndWorkspace => 9,
        }
    }
}

/// Editable color token targets in Tab 1 (`Theme & Contrast`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThemeColorField {
    AccentPrimary,
    AccentSecondary,
    TextPrimary,
    TitleInactive,
    BorderSlate,
    PlaceholderMuted,
}

impl ThemeColorField {
    pub const ALL: [ThemeColorField; 6] = [
        Self::AccentPrimary,
        Self::AccentSecondary,
        Self::TextPrimary,
        Self::TitleInactive,
        Self::BorderSlate,
        Self::PlaceholderMuted,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AccentPrimary => "Primary Accent (accent_primary)",
            Self::AccentSecondary => "Secondary Accent (accent_secondary)",
            Self::TextPrimary => "Primary Text (text_primary)",
            Self::TitleInactive => "Unfocused Header Title (title_inactive)",
            Self::BorderSlate => "Inactive Border Slate (border_slate)",
            Self::PlaceholderMuted => "Muted Placeholder (placeholder_muted)",
        }
    }
}

/// Complete state for the full-screen Settings Page (`Alt+S`).
#[derive(Debug, Clone)]
pub struct SettingsPageState {
    pub is_open: bool,
    pub active_tab: SettingsTab,
    /// Selected setting row within the active tab (`0..active_tab.row_count()`).
    pub selected_row: usize,
    pub status_note: String,

    // --- Tab 1: Theme & Contrast hex override buffers ---
    pub hex_accent_primary: String,
    pub hex_accent_secondary: String,
    pub hex_text_primary: String,
    pub hex_title_inactive: String,
    pub hex_border_slate: String,
    pub hex_placeholder_muted: String,

    // --- Tab 2: Models & TOML Ingestion buffers ---
    pub model_ingest_path: String,
    pub custom_family: ModelFamily,
    pub custom_short_name: String,
    pub custom_display_name: String,
    pub custom_agy_base_id: String,
    pub custom_available_tiers: String,
    pub custom_default_effort: EffortLevel,

    // --- Tab 3: Templates & Defaults buffers ---
    pub template_ingest_path: String,
    pub template_ingest_local: bool,
    pub active_template_idx: usize,

    // --- Tab 4: Execution, Shell Formatting & Workspace buffers ---
    pub add_dirs_input: String,
    pub default_plan_path_input: String,
    pub pinned_constraints_input: String,
}

impl Default for SettingsPageState {
    fn default() -> Self {
        let default_theme = Theme::novos_cosmic_dark();
        Self {
            is_open: false,
            active_tab: SettingsTab::ThemeAndContrast,
            selected_row: 0,
            status_note:
                "1..4: Switch Tab | Up/Down/Tab: Navigate | Left/Right/Enter: Edit/Apply | Ctrl+S: Save | Esc/Alt+S: Return to Prompt"
                    .to_string(),
            hex_accent_primary: default_theme.accent_primary.to_hex(),
            hex_accent_secondary: default_theme.accent_secondary.to_hex(),
            hex_text_primary: default_theme.text_primary.to_hex(),
            hex_title_inactive: default_theme.title_inactive.to_hex(),
            hex_border_slate: default_theme.border_slate.to_hex(),
            hex_placeholder_muted: default_theme.placeholder_muted.to_hex(),
            model_ingest_path: String::new(),
            custom_family: ModelFamily::Gemini,
            custom_short_name: String::new(),
            custom_display_name: String::new(),
            custom_agy_base_id: String::new(),
            custom_available_tiers: "low, medium, high".to_string(),
            custom_default_effort: EffortLevel::Medium,
            template_ingest_path: String::new(),
            template_ingest_local: false,
            active_template_idx: 0,
            add_dirs_input: String::new(),
            default_plan_path_input: String::new(),
            pinned_constraints_input: String::new(),
        }
    }
}

impl SettingsPageState {
    /// Synchronizes the hex color edit buffers from the currently active [`Theme`].
    pub fn sync_from_theme(&mut self, theme: &Theme) {
        self.hex_accent_primary = theme.accent_primary.to_hex();
        self.hex_accent_secondary = theme.accent_secondary.to_hex();
        self.hex_text_primary = theme.text_primary.to_hex();
        self.hex_title_inactive = theme.title_inactive.to_hex();
        self.hex_border_slate = theme.border_slate.to_hex();
        self.hex_placeholder_muted = theme.placeholder_muted.to_hex();
    }

    pub fn switch_tab(&mut self, tab: SettingsTab) {
        self.active_tab = tab;
        self.selected_row = 0;
    }

    pub fn next_row(&mut self) {
        let count = self.active_tab.row_count();
        if count > 0 {
            self.selected_row = (self.selected_row + 1) % count;
        }
    }

    pub fn prev_row(&mut self) {
        let count = self.active_tab.row_count();
        if count > 0 {
            self.selected_row = (self.selected_row + count - 1) % count;
        }
    }

    /// Returns `true` if the currently focused row in the active tab accepts free-form text input.
    pub fn is_text_input_row_focused(&self) -> bool {
        match self.active_tab {
            SettingsTab::ThemeAndContrast => (1..=6).contains(&self.selected_row),
            SettingsTab::ModelsAndToml => {
                matches!(self.selected_row, 0 | 3 | 4 | 5 | 6)
            }
            SettingsTab::TemplatesAndDefaults => self.selected_row == 1,
            SettingsTab::ExecutionAndWorkspace => matches!(self.selected_row, 4 | 7 | 8),
        }
    }
}
