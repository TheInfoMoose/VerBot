use crate::files::{ScannedFile, WorkspaceFileIndex};
use crate::formatting::{
    build_agy_footer_preview_with_flags, build_shell_safe_agy_invocation,
    inspect_prompt_values_safety, inspect_shell_safety, ShellFormattingPolicy, ShellSafetyReport,
};
use crate::ingest::{discover_ingested_templates, ingest_template, IngestOptions};
use crate::models::{
    estimate_prompt_metrics, AgyLaunchMode, EffortLevel, ModelCatalog, ModelFamily, PromptMetrics,
    SubModelProfile,
};
use crate::settings::{SettingsPageState, SettingsTab};
use crate::state::{save_theme_config, GlobalConfig, StateStorageLocation, WorkspaceState};
use crate::template::{PromptTemplate, PromptValues, TemplateFieldSpec};
use crate::theme::{RgbColor, Theme, ThemePreset};
use anyhow::{Context, Result};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders};
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tui_textarea::TextArea;

/// Selectable items in the top control bar (`Family`, `Sub-Model`, `Effort`, `Command`, `Settings`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlItem {
    ModelFamily,
    SubModel,
    Effort,
    SlashCommand,
    Settings,
}

impl ControlItem {
    pub const ALL: [ControlItem; 5] = [
        Self::ModelFamily,
        Self::SubModel,
        Self::Effort,
        Self::SlashCommand,
        Self::Settings,
    ];

    pub fn next(self) -> Self {
        match self {
            Self::ModelFamily => Self::SubModel,
            Self::SubModel => Self::Effort,
            Self::Effort => Self::SlashCommand,
            Self::SlashCommand => Self::Settings,
            Self::Settings => Self::ModelFamily,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::ModelFamily => Self::Settings,
            Self::SubModel => Self::ModelFamily,
            Self::Effort => Self::SubModel,
            Self::SlashCommand => Self::Effort,
            Self::Settings => Self::SlashCommand,
        }
    }
}

/// Active focus pane in the multi-pane TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusPane {
    ControlBar(ControlItem),
    FieldEditor(usize),
    ActionBar,
    TerminalOutput,
}

/// Scrollable in-app Terminal Execution Output Pane (`Ctrl+O` or opened automatically when running
/// `2: Run agy -p`, `3: Output to Stdout / Clipboard`, or `4: Save to .md File`).
#[derive(Debug, Clone)]
pub struct TerminalOutputDrawer {
    pub is_open: bool,
    pub lines: Vec<String>,
    pub scroll_offset: usize,
    pub last_command: String,
    pub last_exit_code: Option<i32>,
    pub is_running: bool,
}

impl Default for TerminalOutputDrawer {
    fn default() -> Self {
        Self {
            is_open: false,
            lines: vec![
                "Verbocity Terminal Execution Output Pane (Ctrl+O to toggle, Up/Down/PgUp/PgDn to scroll, Ctrl+L to clear)".to_string(),
                "Ready — execute an action (Alt+1..4 or Enter on Action Bar) to stream output here without closing Verbocity.".to_string(),
            ],
            scroll_offset: 0,
            last_command: String::new(),
            last_exit_code: None,
            is_running: false,
        }
    }
}

impl TerminalOutputDrawer {
    pub fn clear(&mut self) {
        self.lines.clear();
        self.scroll_offset = 0;
        self.last_exit_code = None;
    }

    pub fn push_line(&mut self, line: impl Into<String>) {
        self.lines.push(line.into());
        // Auto-scroll to bottom when new lines arrive
        self.scroll_to_bottom();
    }

    pub fn append_multiline(&mut self, text: &str) {
        for line in text.lines() {
            self.lines.push(line.to_string());
        }
        self.scroll_to_bottom();
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_offset = self.scroll_offset.saturating_sub(amount);
    }

    pub fn scroll_down(&mut self, amount: usize) {
        let max_offset = self.lines.len().saturating_sub(1);
        self.scroll_offset = (self.scroll_offset + amount).min(max_offset);
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_offset = self.lines.len().saturating_sub(1);
    }
}

/// Selectable dispatch actions in the bottom Action Bar.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum DispatchAction {
    /// `1. Run agy -i (Interactive)`
    AgyInteractive,
    /// `2. Run agy -p (Print)`
    AgyPrint,
    /// `3. Output to Stdout / Clipboard`
    StdoutClipboard,
    /// `4. Save to .md File`
    SaveMarkdown,
}

impl DispatchAction {
    pub const ALL: [DispatchAction; 4] = [
        Self::AgyInteractive,
        Self::AgyPrint,
        Self::StdoutClipboard,
        Self::SaveMarkdown,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::AgyInteractive => "1: Run agy -i (Interactive)",
            Self::AgyPrint => "2: Run agy -p (Print)",
            Self::StdoutClipboard => "3: Output to Stdout / Clipboard",
            Self::SaveMarkdown => "4: Save to .md File",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::AgyInteractive => "agy-interactive",
            Self::AgyPrint => "agy-print",
            Self::StdoutClipboard => "stdout",
            Self::SaveMarkdown => "save-md",
        }
    }

    pub fn from_slug(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "agy-interactive" | "interactive" | "agy-i" | "-i" | "1" => Some(Self::AgyInteractive),
            "agy-print" | "print" | "agy-p" | "-p" | "2" => Some(Self::AgyPrint),
            "stdout" | "clipboard" | "copy" | "3" => Some(Self::StdoutClipboard),
            "save-md" | "save" | "md" | "file" | "4" => Some(Self::SaveMarkdown),
            _ => None,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::AgyInteractive => Self::AgyPrint,
            Self::AgyPrint => Self::StdoutClipboard,
            Self::StdoutClipboard => Self::SaveMarkdown,
            Self::SaveMarkdown => Self::AgyInteractive,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::AgyInteractive => Self::SaveMarkdown,
            Self::AgyPrint => Self::AgyInteractive,
            Self::StdoutClipboard => Self::AgyPrint,
            Self::SaveMarkdown => Self::StdoutClipboard,
        }
    }

    pub fn launch_mode(self) -> AgyLaunchMode {
        match self {
            Self::AgyPrint => AgyLaunchMode::Print,
            _ => AgyLaunchMode::Interactive,
        }
    }
}

/// Structured dispatch manifest emitted when the user confirms an action in `verbot-tui`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DispatchManifest {
    pub action: String,
    pub model_id: String,
    pub model_display_name: String,
    pub effort: String,
    pub slash_command: Option<String>,
    pub rendered_prompt: String,
    pub agy_command_preview: String,
    #[serde(default)]
    pub shell_safe_command: String,
    #[serde(default)]
    pub agy_argv: Vec<String>,
    #[serde(default)]
    pub agy_extra_flags: Vec<String>,
    #[serde(default)]
    pub shell_formatting_policy: String,
    pub output_md_path: Option<String>,
    pub cwd: String,
}

impl DispatchManifest {
    pub fn write_to_path(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create parent directory for dispatch manifest {}",
                        parent.display()
                    )
                })?;
            }
        }
        let json = serde_json::to_string_pretty(self)
            .context("Failed to serialize DispatchManifest to JSON")?;
        fs::write(path, json)
            .with_context(|| format!("Failed to write dispatch manifest to {}", path.display()))
    }
}

/// Destination for a confirmed selection in the fuzzy file picker modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FilePickerTarget {
    FieldEditor(usize),
    SettingsModelIngest,
    SettingsTemplateIngest,
}

/// State for the `@` / `Ctrl+F` CWD fuzzy file picker modal.
#[derive(Debug, Clone)]
pub struct FilePickerModal {
    pub is_open: bool,
    pub query: String,
    pub selected_index: usize,
    pub markdown_only: bool,
    pub target_field_index: usize,
    pub target: FilePickerTarget,
    /// Optional token in the target field to replace (e.g. `"<file>.md"` when triggered by `Update <file>.md plan`).
    pub replace_token: Option<String>,
}

impl Default for FilePickerModal {
    fn default() -> Self {
        Self {
            is_open: false,
            query: String::new(),
            selected_index: 0,
            markdown_only: false,
            target_field_index: 0,
            target: FilePickerTarget::FieldEditor(0),
            replace_token: None,
        }
    }
}

impl FilePickerModal {
    pub fn open(
        &mut self,
        target_field_index: usize,
        markdown_only: bool,
        replace_token: Option<String>,
    ) {
        self.is_open = true;
        self.query.clear();
        self.selected_index = 0;
        self.markdown_only = markdown_only;
        self.target_field_index = target_field_index;
        self.target = FilePickerTarget::FieldEditor(target_field_index);
        self.replace_token = replace_token;
    }

    pub fn open_for_target(&mut self, target: FilePickerTarget, markdown_only: bool) {
        self.is_open = true;
        self.query.clear();
        self.selected_index = 0;
        self.markdown_only = markdown_only;
        self.target = target;
        self.replace_token = None;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.query.clear();
        self.selected_index = 0;
        self.replace_token = None;
    }

    pub fn matches<'a>(&self, index: &'a WorkspaceFileIndex, limit: usize) -> Vec<&'a ScannedFile> {
        index.fuzzy_search(&self.query, self.markdown_only, limit)
    }
}

/// Pending confirmation state before opening the `.md` plan file picker when `Update <file>.md plan` is chosen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingPlanPickerApproval {
    pub field_index: usize,
    pub replace_token: String,
}

/// Confirmation modal displayed when `ShellFormattingPolicy::PromptOnSpecialChars` is active
/// and shell metacharacters or multi-line breaks are detected in the prompt.
#[derive(Debug, Clone, Default)]
pub struct ShellQuoteModal {
    pub is_open: bool,
    pub pending_action: Option<DispatchAction>,
    pub safety_report: ShellSafetyReport,
    pub shell_safe_preview: String,
}

impl ShellQuoteModal {
    pub fn open(
        &mut self,
        action: DispatchAction,
        report: ShellSafetyReport,
        shell_safe_preview: String,
    ) {
        self.is_open = true;
        self.pending_action = Some(action);
        self.safety_report = report;
        self.shell_safe_preview = shell_safe_preview;
    }

    pub fn close(&mut self) {
        self.is_open = false;
        self.pending_action = None;
    }
}

/// Interactive fields inside the `Alt+U` Model Catalog Sync & Add Modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelModalField {
    Family,
    ShortName,
    DisplayName,
    AgyBaseId,
    DefaultEffort,
}

impl ModelModalField {
    pub const ALL: [ModelModalField; 5] = [
        Self::Family,
        Self::ShortName,
        Self::DisplayName,
        Self::AgyBaseId,
        Self::DefaultEffort,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Family => "Model Family (Left/Right)",
            Self::ShortName => "Short Name (e.g. Nebula)",
            Self::DisplayName => "Display Name (e.g. Gemini 4 Nebula)",
            Self::AgyBaseId => "Base agy ID (e.g. gemini-4-nebula)",
            Self::DefaultEffort => "Default Effort (Left/Right)",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Family => Self::ShortName,
            Self::ShortName => Self::DisplayName,
            Self::DisplayName => Self::AgyBaseId,
            Self::AgyBaseId => Self::DefaultEffort,
            Self::DefaultEffort => Self::Family,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Family => Self::DefaultEffort,
            Self::ShortName => Self::Family,
            Self::DisplayName => Self::ShortName,
            Self::AgyBaseId => Self::DisplayName,
            Self::DefaultEffort => Self::AgyBaseId,
        }
    }
}

/// State for the `Alt+U` Model Catalog Sync & Custom Sub-Model Add/Update Modal.
#[derive(Debug, Clone)]
pub struct ModelManagerModal {
    pub is_open: bool,
    pub focused_field: ModelModalField,
    pub family: ModelFamily,
    pub short_name: String,
    pub display_name: String,
    pub agy_base_id: String,
    pub default_effort: EffortLevel,
    pub status_note: String,
}

impl Default for ModelManagerModal {
    fn default() -> Self {
        Self {
            is_open: false,
            focused_field: ModelModalField::ShortName,
            family: ModelFamily::Gemini,
            short_name: String::new(),
            display_name: String::new(),
            agy_base_id: String::new(),
            default_effort: EffortLevel::Medium,
            status_note:
                "Press Ctrl+R / Alt+R to sync from `agy models`, or fill fields and press Enter to add/update."
                    .to_string(),
        }
    }
}

impl ModelManagerModal {
    pub fn open(&mut self, current_family: ModelFamily, current_effort: EffortLevel) {
        self.is_open = true;
        self.focused_field = ModelModalField::ShortName;
        self.family = current_family;
        self.short_name.clear();
        self.display_name.clear();
        self.agy_base_id.clear();
        self.default_effort = current_effort;
        self.status_note =
            "Ctrl+R / Alt+R: Sync from `agy models` | Tab/Up/Down: Field | Enter: Save Model | Esc: Cancel"
                .to_string();
    }

    pub fn close(&mut self) {
        self.is_open = false;
    }
}

/// Wrapper around `tui_textarea::TextArea` for a single template field pane.
#[derive(Debug, Clone)]
pub struct FieldEditor {
    pub spec: TemplateFieldSpec,
    pub textarea: TextArea<'static>,
    pub active_choice_idx: Option<usize>,
}

impl FieldEditor {
    pub fn new(spec: TemplateFieldSpec, initial_text: &str) -> Self {
        let mut textarea = if initial_text.is_empty() {
            TextArea::default()
        } else {
            let lines: Vec<String> = initial_text.lines().map(ToString::to_string).collect();
            TextArea::new(lines)
        };
        textarea.set_placeholder_text(format!("<{}>", spec.placeholder));
        textarea.set_cursor_line_style(Style::default());
        Self {
            spec,
            textarea,
            active_choice_idx: None,
        }
    }

    pub fn text(&self) -> String {
        self.textarea.lines().join("\n")
    }

    pub fn set_text(&mut self, content: &str) {
        let lines: Vec<String> = if content.is_empty() {
            vec![String::new()]
        } else {
            content.lines().map(ToString::to_string).collect()
        };
        self.textarea = TextArea::new(lines);
        self.textarea
            .set_placeholder_text(format!("<{}>", self.spec.placeholder));
        self.textarea.set_cursor_line_style(Style::default());
        self.textarea.move_cursor(tui_textarea::CursorMove::Bottom);
        self.textarea.move_cursor(tui_textarea::CursorMove::End);
    }

    pub fn insert_str(&mut self, text: &str) {
        self.textarea.insert_str(text);
    }

    /// Applies the active NovOS / COSMIC theme styling to the `TextArea` widget block and cursor.
    pub fn apply_theme(&mut self, theme: &Theme, focused: bool, pinned_badge: Option<&str>) {
        let surface_style = theme.surface_style();
        let border_style = if focused {
            theme.border_focused_style()
        } else {
            theme.border_inactive_style()
        };
        let title_style = if focused {
            theme.title_focused_style()
        } else {
            theme.title_inactive_style()
        };
        let badge_style = if focused {
            theme.badge_focused_style()
        } else {
            theme.badge_inactive_style()
        };

        let mut title_spans = vec![Span::styled(format!(" {} ", self.spec.label), title_style)];
        if !self.spec.choices.is_empty() {
            title_spans.push(Span::styled(
                format!("[Choices (Alt+C): {}] ", self.spec.choices.join(" | ")),
                badge_style,
            ));
        }
        if let Some(badge) = pinned_badge {
            if !badge.is_empty() {
                title_spans.push(Span::styled(
                    format!("[Pinned (Ctrl+K): {}] ", badge),
                    badge_style,
                ));
            }
        }
        if focused {
            title_spans.push(Span::styled("(@/Ctrl+F: Files) ", badge_style));
        }

        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border_style)
            .title(Line::from(title_spans))
            .style(surface_style);

        self.textarea.set_block(block);
        self.textarea.set_style(surface_style);
        self.textarea.set_placeholder_style(
            Style::default()
                .fg(theme.placeholder_color())
                .bg(theme.surface_color()),
        );
        self.textarea.set_cursor_line_style(surface_style);
        self.textarea.set_cursor_style(
            Style::default()
                .fg(theme.bg_color())
                .bg(theme.highlight_color())
                .add_modifier(Modifier::BOLD),
        );
    }
}

/// Complete application state for `verbot-tui`.
pub struct App {
    pub cwd: PathBuf,
    pub force_local_state: bool,
    pub custom_xdg_data_dir: Option<PathBuf>,
    pub custom_theme_toml_path: Option<PathBuf>,
    pub custom_models_toml_path: Option<PathBuf>,
    pub custom_config_toml_path: Option<PathBuf>,
    pub custom_global_templates_dir: Option<PathBuf>,
    pub custom_agy_bin: Option<PathBuf>,
    pub global_config: GlobalConfig,
    pub workspace_state: WorkspaceState,
    pub state_location: StateStorageLocation,
    pub theme: Theme,
    pub template: PromptTemplate,
    pub available_slash_commands: Vec<String>,
    pub catalog: ModelCatalog,
    pub selected_family: ModelFamily,
    pub selected_profile_idx: usize,
    pub selected_effort: EffortLevel,
    pub values: PromptValues,
    pub editors: Vec<FieldEditor>,
    pub focus: FocusPane,
    pub selected_action: DispatchAction,
    pub show_preview: bool,
    pub file_index: WorkspaceFileIndex,
    pub file_picker: FilePickerModal,
    pub pending_plan_picker_approval: Option<PendingPlanPickerApproval>,
    pub model_modal: ModelManagerModal,
    pub settings_page: SettingsPageState,
    pub shell_quote_modal: ShellQuoteModal,
    pub terminal_output: TerminalOutputDrawer,
    pub pending_interactive_handoff: Option<DispatchManifest>,
    pub history_cursor: Option<usize>,
    pub status_message: String,
    pub should_quit: bool,
    pub confirmed_manifest: Option<DispatchManifest>,
    pub saved_markdown_path: Option<PathBuf>,
    pub explicit_agy_model_id: Option<String>,
}

impl App {
    /// Initializes the TUI application state for `cwd`.
    pub fn new(
        cwd: PathBuf,
        template: PromptTemplate,
        catalog: ModelCatalog,
        workspace_state: WorkspaceState,
        state_location: StateStorageLocation,
        theme: Theme,
        file_index: WorkspaceFileIndex,
        force_local_state: bool,
    ) -> Self {
        let global_config = GlobalConfig::load_or_default(None);

        // Resolve model profile and effort from workspace_state
        let mut selected_profile_idx = 0;
        for (idx, p) in catalog.profiles.iter().enumerate() {
            if p.short_name
                .eq_ignore_ascii_case(&workspace_state.selected_sub_model)
                || p.display_name
                    .eq_ignore_ascii_case(&workspace_state.selected_sub_model)
            {
                selected_profile_idx = idx;
                break;
            }
        }
        let selected_family = catalog.profiles[selected_profile_idx].family;
        let selected_effort = EffortLevel::from_str_loose(&workspace_state.selected_effort);

        // Initialize canonical values
        let mut values = PromptValues::new();
        values.slash_command = workspace_state
            .selected_slash_command
            .clone()
            .or_else(|| template.default_command.clone());

        if !workspace_state.pinned_constraints.is_empty() {
            values.constraints = workspace_state.formatted_pinned_constraints();
        }

        // Build field editors from template specifications
        let mut editors = Vec::with_capacity(template.fields.len());
        for spec in &template.fields {
            let init_val = values.get(&spec.id).to_string();
            editors.push(FieldEditor::new(spec.clone(), &init_val));
        }

        let initial_focus = if editors.is_empty() {
            FocusPane::ControlBar(ControlItem::ModelFamily)
        } else {
            FocusPane::FieldEditor(0)
        };

        let available_slash_commands = template.all_slash_commands(Some(&cwd), None);
        let mut settings_page = SettingsPageState::default();
        settings_page.sync_from_theme(&theme);
        settings_page.add_dirs_input = global_config.agy_flags.formatted_add_dirs();
        settings_page.default_plan_path_input = workspace_state
            .default_plan_path
            .clone()
            .unwrap_or_default();
        settings_page.pinned_constraints_input = workspace_state.formatted_pinned_constraints();

        let show_preview = global_config.show_preview_on_startup;

        let mut app = Self {
            cwd,
            force_local_state,
            custom_xdg_data_dir: None,
            custom_theme_toml_path: None,
            custom_models_toml_path: None,
            custom_config_toml_path: None,
            custom_global_templates_dir: None,
            custom_agy_bin: None,
            global_config,
            workspace_state,
            state_location,
            theme,
            template,
            available_slash_commands,
            catalog,
            selected_family,
            selected_profile_idx,
            selected_effort,
            values,
            editors,
            focus: initial_focus,
            selected_action: DispatchAction::AgyInteractive,
            show_preview,
            file_index,
            file_picker: FilePickerModal::default(),
            pending_plan_picker_approval: None,
            model_modal: ModelManagerModal::default(),
            settings_page,
            shell_quote_modal: ShellQuoteModal::default(),
            terminal_output: TerminalOutputDrawer::default(),
            pending_interactive_handoff: None,
            history_cursor: None,
            status_message:
                "Tab/Shift+Tab: Navigate | Alt+S: Settings | Alt+M: Model | Ctrl+O: Output | Ctrl+P: Preview | Ctrl+Q: Quit"
                    .to_string(),
            should_quit: false,
            confirmed_manifest: None,
            saved_markdown_path: None,
            explicit_agy_model_id: None,
        };
        app.sync_values_from_editors();
        app.refresh_editor_themes();
        app
    }

    /// Pre-populates the `Context/Logs` field with captured stdin or initial context text.
    pub fn set_initial_context_logs(&mut self, context_text: &str) {
        let trimmed = context_text.trim();
        if trimmed.is_empty() {
            return;
        }
        if let Some(editor) = self
            .editors
            .iter_mut()
            .find(|e| e.spec.id == "context_logs")
        {
            let existing = editor.text();
            let merged = if existing.trim().is_empty() {
                trimmed.to_string()
            } else {
                format!("{}\n{}", existing.trim(), trimmed)
            };
            editor.set_text(&merged);
        } else {
            self.values.context_logs = trimmed.to_string();
        }
        self.sync_values_from_editors();
    }

    /// Returns the currently active [`SubModelProfile`].
    pub fn active_profile(&self) -> &SubModelProfile {
        &self.catalog.profiles[self.selected_profile_idx]
    }

    /// Returns the exact `agy` model ID for the active sub-model and effort level.
    pub fn active_agy_model_id(&self) -> String {
        if let Some(ref explicit) = self.explicit_agy_model_id {
            return explicit.clone();
        }
        self.active_profile()
            .resolve_agy_model_id(self.selected_effort)
    }

    /// Synchronizes canonical [`PromptValues`] from the current [`FieldEditor`] contents.
    pub fn sync_values_from_editors(&mut self) {
        for editor in &self.editors {
            let text = editor.text();
            self.values.set(&editor.spec.id, text);
        }
    }

    /// Copies canonical [`PromptValues`] into the [`FieldEditor`] widgets (e.g., when loading history).
    pub fn sync_editors_from_values(&mut self) {
        for editor in &mut self.editors {
            let val = self.values.get(&editor.spec.id).to_string();
            editor.set_text(&val);
        }
        self.refresh_editor_themes();
    }

    /// Re-applies active theme colors and focus borders to all field editors.
    pub fn refresh_editor_themes(&mut self) {
        let pinned_summary = if self.workspace_state.pinned_constraints.is_empty() {
            None
        } else {
            Some(self.workspace_state.pinned_constraints.join(", "))
        };

        for (idx, editor) in self.editors.iter_mut().enumerate() {
            let is_focused = self.focus == FocusPane::FieldEditor(idx)
                && !self.file_picker.is_open
                && !self.model_modal.is_open
                && !self.settings_page.is_open
                && !self.shell_quote_modal.is_open;
            let badge = if editor.spec.id == "constraints" {
                pinned_summary.as_deref()
            } else {
                None
            };
            editor.apply_theme(&self.theme, is_focused, badge);
        }
    }

    /// Dynamically renders the prompt in the active model's format without mutating `self.values`.
    pub fn rendered_prompt(&self) -> String {
        self.active_profile().render_prompt(&self.values)
    }

    /// Computes live word/character/token metrics for the rendered prompt.
    pub fn prompt_metrics(&self) -> PromptMetrics {
        estimate_prompt_metrics(&self.rendered_prompt())
    }

    /// Builds the live `agy` CLI invocation preview string for the footer (including any configured
    /// extra `agy` flags from Settings).
    pub fn agy_command_preview(&self) -> String {
        let model_id = self.active_agy_model_id();
        let rendered = self.rendered_prompt();
        build_agy_footer_preview_with_flags(
            self.selected_action.launch_mode(),
            &model_id,
            self.selected_effort,
            &self.global_config.agy_flags,
            &rendered,
        )
    }

    /// Builds the raw `argv` vector and full POSIX-quoted `shell_safe_command` string for a given [`DispatchAction`].
    pub fn build_shell_safe_agy_for_action(&self, action: DispatchAction) -> (Vec<String>, String) {
        let model_id = self.active_agy_model_id();
        let rendered = self.rendered_prompt();
        build_shell_safe_agy_invocation(
            action.launch_mode(),
            &model_id,
            self.selected_effort,
            &self.global_config.agy_flags,
            &rendered,
            self.global_config.shell_formatting_policy,
        )
    }

    /// Inspects current user input across all prompt fields for shell-sensitive metacharacters and newlines.
    pub fn inspect_prompt_shell_safety(&self) -> ShellSafetyReport {
        let mut report = inspect_prompt_values_safety(&self.values);
        if !report.needs_escaping_or_quoting() {
            let rendered_rep = inspect_shell_safety(&self.rendered_prompt());
            if rendered_rep.needs_escaping_or_quoting() {
                report = rendered_rep;
            }
        }
        report
    }

    /// Opens the dedicated full-screen Settings Page (`Alt+S`), preserving all field text.
    pub fn open_settings_page(&mut self) {
        self.sync_values_from_editors();
        self.pending_plan_picker_approval = None;
        self.settings_page.sync_from_theme(&self.theme);
        self.settings_page.custom_family = self.selected_family;
        self.settings_page.custom_default_effort = self.selected_effort;
        self.settings_page.add_dirs_input = self.global_config.agy_flags.formatted_add_dirs();
        self.settings_page.default_plan_path_input = self
            .workspace_state
            .default_plan_path
            .clone()
            .unwrap_or_default();
        self.settings_page.pinned_constraints_input =
            self.workspace_state.formatted_pinned_constraints();
        self.settings_page.is_open = true;
        self.refresh_editor_themes();
        self.status_message =
            "Settings Page (Alt+S): 1..4 Switch Tab | Up/Down/Tab Navigate | Left/Right/Enter Edit | Esc Return"
                .to_string();
    }

    /// Closes the Settings Page (`Esc` or `Alt+S`) and returns to the Main Page without losing any prompt text.
    pub fn close_settings_page(&mut self) {
        self.settings_page.is_open = false;
        self.refresh_editor_themes();
        self.status_message =
            "Returned from Settings Page (all prompt field text preserved)".to_string();
    }

    /// Persists `self.global_config` to `~/.config/verbot/config.toml` (or `self.custom_config_toml_path`).
    pub fn persist_global_config(&self) {
        let target = self
            .custom_config_toml_path
            .clone()
            .or_else(GlobalConfig::default_path);
        if let Some(path) = target {
            let _ = self.global_config.save_to_file(&path);
        }
    }

    /// Persists `self.theme` to `~/.config/verbot/theme.toml` (or `self.custom_theme_toml_path`).
    pub fn persist_theme_toml(&self) {
        let _ = save_theme_config(&self.theme, self.custom_theme_toml_path.as_deref());
    }

    /// Applies any valid `#RRGGBB` hex color overrides from `self.settings_page` onto `self.theme`
    /// and saves `theme.toml`.
    pub fn apply_settings_theme_hex_overrides(&mut self) -> Result<()> {
        let mut updated = self.theme.clone();
        let mut changed = false;

        if let Ok(c) = RgbColor::from_hex(&self.settings_page.hex_accent_primary) {
            if updated.accent_primary != c {
                updated.accent_primary = c;
                changed = true;
            }
        }
        if let Ok(c) = RgbColor::from_hex(&self.settings_page.hex_accent_secondary) {
            if updated.accent_secondary != c {
                updated.accent_secondary = c;
                changed = true;
            }
        }
        if let Ok(c) = RgbColor::from_hex(&self.settings_page.hex_text_primary) {
            if updated.text_primary != c {
                updated.text_primary = c;
                changed = true;
            }
        }
        if let Ok(c) = RgbColor::from_hex(&self.settings_page.hex_title_inactive) {
            if updated.title_inactive != c {
                updated.title_inactive = c;
                changed = true;
            }
        }
        if let Ok(c) = RgbColor::from_hex(&self.settings_page.hex_border_slate) {
            if updated.border_slate != c {
                updated.border_slate = c;
                changed = true;
            }
        }
        if let Ok(c) = RgbColor::from_hex(&self.settings_page.hex_placeholder_muted) {
            if updated.placeholder_muted != c {
                updated.placeholder_muted = c;
                changed = true;
            }
        }

        if changed {
            updated.preset = ThemePreset::Custom;
            updated.name = ThemePreset::Custom.as_str().to_string();
            updated.use_ansi = false;
            self.theme = updated;
            self.workspace_state.active_theme_preset = self.theme.preset.as_str().to_string();
            self.global_config.default_theme_preset = self.theme.preset.as_str().to_string();
            self.refresh_editor_themes();
        }
        self.persist_theme_toml();
        self.persist_global_config();
        self.persist_workspace_state();
        let msg = format!(
            "Saved theme '{}' (accent={}, title_inactive={}, border={})",
            self.theme.name,
            self.theme.accent_primary.to_hex(),
            self.theme.title_inactive.to_hex(),
            self.theme.border_slate.to_hex()
        );
        self.settings_page.status_note = msg.clone();
        self.status_message = msg;
        Ok(())
    }

    /// Ingests an external model `.toml` or `.yaml` file from the Settings Page, merges its profiles
    /// into `self.catalog`, selects the first ingested profile, and saves `models.toml`.
    pub fn ingest_model_file_from_settings(&mut self, path: &Path) -> Result<Vec<SubModelProfile>> {
        self.sync_values_from_editors();
        let resolved_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        let ingested = self
            .catalog
            .ingest_model_file(&resolved_path, self.custom_models_toml_path.as_deref())?;
        if let Some(first) = ingested.first() {
            self.select_model_by_query(&first.short_name);
        }
        let names: Vec<String> = ingested
            .iter()
            .map(|p| format!("{} ({})", p.display_name, p.agy_base_id))
            .collect();
        let msg = format!(
            "Ingested {} model profile(s) from {}: {}",
            ingested.len(),
            resolved_path.display(),
            names.join(", ")
        );
        self.settings_page.status_note = msg.clone();
        self.status_message = msg;
        Ok(ingested)
    }

    /// Submits the custom sub-model form on Tab 2 of the Settings Page (`Models & TOML Ingestion`).
    pub fn submit_custom_model_from_settings(&mut self) -> Result<usize> {
        self.sync_values_from_editors();
        let family = self.settings_page.custom_family;
        let effort = self.settings_page.custom_default_effort;
        let raw_short = self.settings_page.custom_short_name.trim().to_string();
        let raw_display = self.settings_page.custom_display_name.trim().to_string();
        let raw_base = self.settings_page.custom_agy_base_id.trim().to_string();
        let raw_tiers = self.settings_page.custom_available_tiers.trim().to_string();

        if raw_short.is_empty() && raw_base.is_empty() && raw_display.is_empty() {
            let note =
                "Enter at least a Short Name or Base agy ID to add/update a custom sub-model."
                    .to_string();
            self.settings_page.status_note = note.clone();
            self.status_message = note;
            return Ok(self.selected_profile_idx);
        }

        let short_name = if !raw_short.is_empty() {
            raw_short
        } else if !raw_display.is_empty() {
            raw_display.clone()
        } else {
            raw_base.clone()
        };

        let display_name = if !raw_display.is_empty() {
            raw_display
        } else {
            format!("{} {}", family.as_str(), short_name)
        };

        let slug: String = short_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string();

        let agy_base_id = if !raw_base.is_empty() {
            raw_base
                .trim_end_matches("-low")
                .trim_end_matches("-medium")
                .trim_end_matches("-high")
                .to_ascii_lowercase()
        } else {
            match family {
                ModelFamily::Gemini => format!("gemini-{}", slug),
                ModelFamily::Claude => format!("claude-{}", slug),
                ModelFamily::Gpt => format!("gpt-{}", slug),
            }
        };

        let available_tiers: Vec<String> = raw_tiers
            .split(|c: char| c == ',' || c.is_whitespace())
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| s.to_ascii_lowercase())
            .collect();

        let profile = SubModelProfile {
            family,
            short_name: short_name.clone(),
            display_name: display_name.clone(),
            agy_base_id,
            available_tiers: if available_tiers.is_empty() {
                vec![
                    "low".to_string(),
                    "medium".to_string(),
                    "high".to_string(),
                ]
            } else {
                available_tiers
            },
            default_effort: effort,
            format_style: family.default_format_style(),
        };

        let idx = self.catalog.upsert_profile(profile);
        self.selected_profile_idx = idx;
        self.selected_family = family;
        self.selected_effort = effort;
        self.explicit_agy_model_id = None;
        self.workspace_state.selected_model_family = family.as_str().to_string();
        self.workspace_state.selected_sub_model = short_name;
        self.workspace_state.selected_effort = effort.as_str().to_string();

        let _ = self
            .catalog
            .save_to_toml_file(self.custom_models_toml_path.as_deref());
        self.persist_workspace_state();

        let msg = format!(
            "Saved custom sub-model '{}' ({}) to models.toml",
            display_name,
            self.active_agy_model_id()
        );
        self.settings_page.status_note = msg.clone();
        self.status_message = msg;
        Ok(idx)
    }

    /// Returns all available templates (`(label, PromptTemplate)`) including the built-in template
    /// and any ingested templates from `~/.config/verbot/templates/` or `./.verbot/templates/`.
    pub fn available_templates(&self) -> Vec<(String, PromptTemplate)> {
        let mut list = vec![(
            "Built-in: Gemini Agent Prompt Template".to_string(),
            PromptTemplate::default(),
        )];
        for (path, tmpl) in discover_ingested_templates(
            &self.cwd,
            self.custom_global_templates_dir.as_deref(),
        ) {
            let label = format!(
                "{} ({})",
                tmpl.name,
                path.file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("template")
            );
            if !list.iter().any(|(_, existing)| existing.name == tmpl.name) {
                list.push((label, tmpl));
            }
        }
        list
    }

    /// Cycles the active template on Tab 3 of the Settings Page while preserving canonical field text.
    pub fn cycle_active_template(&mut self, forward: bool) {
        self.sync_values_from_editors();
        let templates = self.available_templates();
        if templates.is_empty() {
            return;
        }
        let len = templates.len();
        self.settings_page.active_template_idx = if forward {
            (self.settings_page.active_template_idx + 1) % len
        } else {
            (self.settings_page.active_template_idx + len - 1) % len
        };
        let (_, chosen_tmpl) = &templates[self.settings_page.active_template_idx];
        self.template = chosen_tmpl.clone();
        self.global_config.default_template = Some(self.template.name.clone());
        self.editors = self
            .template
            .fields
            .iter()
            .map(|spec| {
                let val = self.values.get(&spec.id).to_string();
                FieldEditor::new(spec.clone(), &val)
            })
            .collect();
        self.refresh_slash_commands(None);
        self.refresh_editor_themes();
        self.persist_global_config();
        let msg = format!("Active template switched to '{}'", self.template.name);
        self.settings_page.status_note = msg.clone();
        self.status_message = msg;
    }

    /// Ingests a template `.md` / `.toml` / `.yaml` file from Tab 3 of the Settings Page.
    pub fn ingest_template_from_settings(&mut self, path: &Path) -> Result<PathBuf> {
        self.sync_values_from_editors();
        let resolved_path = if path.is_absolute() {
            path.to_path_buf()
        } else {
            self.cwd.join(path)
        };
        let opts = IngestOptions {
            source_path: resolved_path,
            local: self.settings_page.template_ingest_local,
            cwd: Some(self.cwd.clone()),
            custom_global_templates_dir: self.custom_global_templates_dir.clone(),
            custom_name: None,
            dry_run: false,
        };
        let report = ingest_template(&opts)?;
        self.template = report.parsed_template.clone();
        self.global_config.default_template = Some(self.template.name.clone());
        self.editors = self
            .template
            .fields
            .iter()
            .map(|spec| {
                let val = self.values.get(&spec.id).to_string();
                FieldEditor::new(spec.clone(), &val)
            })
            .collect();
        self.refresh_slash_commands(None);
        self.refresh_editor_themes();
        self.persist_global_config();
        let msg = format!(
            "Ingested template '{}' -> {}",
            report.parsed_template.name,
            report.primary_dest_path.display()
        );
        self.settings_page.status_note = msg.clone();
        self.status_message = msg;
        Ok(report.primary_dest_path)
    }

    /// Cycles to the next [`ThemePreset`] (`Ctrl+T`), preserving all field text and updating styles.
    pub fn cycle_theme(&mut self) {
        let next_preset = self.theme.preset.next();
        self.set_theme_preset(next_preset);
    }

    /// Sets a specific [`ThemePreset`] and updates workspace state and editor styles.
    pub fn set_theme_preset(&mut self, preset: ThemePreset) {
        let _ = self
            .workspace_state
            .set_theme_preset(preset, self.custom_theme_toml_path.as_deref());
        self.theme = Theme::from_preset(preset);
        self.global_config.default_theme_preset = preset.as_str().to_string();
        self.settings_page.sync_from_theme(&self.theme);
        self.refresh_editor_themes();
        let msg = format!(
            "Theme switched to '{}' ({})",
            self.theme.name,
            self.theme.accent_primary.to_hex()
        );
        self.settings_page.status_note = msg.clone();
        self.status_message = msg;
        self.persist_global_config();
        self.persist_workspace_state();
    }

    /// Selects a model profile by query string (e.g. `"Argon"`, `"Opus"`, `"GPT-OSS 120B"`),
    /// preserving all canonical field values and editor text.
    pub fn select_model_by_query(&mut self, query: &str) -> bool {
        let q = query.trim().to_ascii_lowercase();
        for (idx, profile) in self.catalog.profiles.iter().enumerate() {
            if profile.short_name.to_ascii_lowercase() == q
                || profile.display_name.to_ascii_lowercase() == q
                || profile.agy_base_id.to_ascii_lowercase() == q
            {
                self.selected_profile_idx = idx;
                self.selected_family = profile.family;
                self.explicit_agy_model_id = None;
                self.workspace_state.selected_model_family =
                    self.selected_family.as_str().to_string();
                self.workspace_state.selected_sub_model = profile.short_name.clone();
                self.status_message = format!(
                    "Selected model: {} ({})",
                    profile.display_name,
                    self.active_agy_model_id()
                );
                self.persist_workspace_state();
                return true;
            }
            for tier in &profile.available_tiers {
                let full_id = format!("{}-{}", profile.agy_base_id, tier);
                if full_id == q {
                    self.selected_profile_idx = idx;
                    self.selected_family = profile.family;
                    self.selected_effort = EffortLevel::from_str_loose(tier);
                    self.explicit_agy_model_id = Some(full_id);
                    self.workspace_state.selected_model_family =
                        self.selected_family.as_str().to_string();
                    self.workspace_state.selected_sub_model = profile.short_name.clone();
                    self.workspace_state.selected_effort =
                        self.selected_effort.as_str().to_string();
                    self.persist_workspace_state();
                    return true;
                }
            }
        }
        false
    }

    /// Cycles the active Model Family (`Gemini` -> `Claude` -> `GPT`), preserving all field text.
    pub fn cycle_model_family(&mut self, forward: bool) {
        let target_family = if forward {
            self.selected_family.next()
        } else {
            self.selected_family.prev()
        };
        if let Some((idx, profile)) = self
            .catalog
            .profiles
            .iter()
            .enumerate()
            .find(|(_, p)| p.family == target_family)
        {
            self.selected_family = target_family;
            self.selected_profile_idx = idx;
            self.explicit_agy_model_id = None;
            self.workspace_state.selected_model_family = target_family.as_str().to_string();
            self.workspace_state.selected_sub_model = profile.short_name.clone();
            self.status_message = format!(
                "Model family: {} -> {} ({})",
                target_family.as_str(),
                profile.display_name,
                self.active_agy_model_id()
            );
            self.persist_workspace_state();
        }
    }

    /// Cycles the active Sub-Model across the catalog (`Alt+M` or arrow keys on Sub-Model selector),
    /// preserving all field text.
    pub fn cycle_sub_model(&mut self, forward: bool) {
        if self.catalog.profiles.is_empty() {
            return;
        }
        let len = self.catalog.profiles.len();
        self.selected_profile_idx = if forward {
            (self.selected_profile_idx + 1) % len
        } else {
            (self.selected_profile_idx + len - 1) % len
        };
        let profile = &self.catalog.profiles[self.selected_profile_idx];
        self.selected_family = profile.family;
        self.explicit_agy_model_id = None;
        self.workspace_state.selected_model_family = self.selected_family.as_str().to_string();
        self.workspace_state.selected_sub_model = profile.short_name.clone();
        self.status_message = format!(
            "Sub-model: {} ({})",
            profile.display_name,
            self.active_agy_model_id()
        );
        self.persist_workspace_state();
    }

    /// Cycles the active Thinking / Effort level (`low`, `medium`, `high`, `xhigh`, `max`).
    pub fn cycle_effort(&mut self, forward: bool) {
        self.explicit_agy_model_id = None;
        self.selected_effort = if forward {
            self.selected_effort.next()
        } else {
            self.selected_effort.prev()
        };
        self.workspace_state.selected_effort = self.selected_effort.as_str().to_string();
        self.status_message = format!(
            "Thinking effort: {} (model ID: {})",
            self.selected_effort.as_str(),
            self.active_agy_model_id()
        );
        self.persist_workspace_state();
    }

    /// Re-scans built-in and filesystem `agy` slash commands (skills and workflows).
    pub fn refresh_slash_commands(&mut self, extra_roots: Option<&[PathBuf]>) {
        self.available_slash_commands = self
            .template
            .all_slash_commands(Some(&self.cwd), extra_roots);
    }

    /// Returns all selectable slash command options (`Some(cmd)` for each discovered command, plus `None` for `(none)`).
    pub fn slash_command_options(&self) -> Vec<Option<String>> {
        let mut options: Vec<Option<String>> = self
            .available_slash_commands
            .iter()
            .cloned()
            .map(Some)
            .collect();
        options.push(None);
        options
    }

    /// Cycles the active slash command across all built-in and discovered `agy` slash commands plus `(none)`.
    pub fn cycle_slash_command(&mut self, forward: bool) {
        let options = self.slash_command_options();

        let current_idx = options
            .iter()
            .position(|opt| opt == &self.values.slash_command)
            .unwrap_or(0);
        let next_idx = if forward {
            (current_idx + 1) % options.len()
        } else {
            (current_idx + options.len() - 1) % options.len()
        };
        self.values.slash_command = options[next_idx].clone();
        self.workspace_state.selected_slash_command = self.values.slash_command.clone();
        let display_cmd = self
            .values
            .formatted_slash_command()
            .unwrap_or_else(|| "(none)".to_string());
        self.status_message = format!("Slash command: {}", display_cmd);
        self.persist_workspace_state();
    }

    /// Cycles quick-choice options (`Alt+C`) for the currently focused field (e.g., `Output` choices:
    /// `Diagnose only | Update <file>.md plan | Execute Step X`).
    /// When `Update <file>.md plan` is chosen, stages a pending approval gate requiring explicit
    /// confirmation (`Enter` or `Alt+Y`) before opening the `.md` CWD file picker modal.
    pub fn cycle_focused_field_choice(&mut self) {
        let FocusPane::FieldEditor(field_idx) = self.focus else {
            return;
        };
        let Some(editor) = self.editors.get_mut(field_idx) else {
            return;
        };
        if editor.spec.choices.is_empty() {
            self.status_message = format!("Field '{}' has no quick choices", editor.spec.label);
            return;
        }
        let next_idx = match editor.active_choice_idx {
            Some(i) => (i + 1) % editor.spec.choices.len(),
            None => 0,
        };
        editor.active_choice_idx = Some(next_idx);
        let chosen = editor.spec.choices[next_idx].clone();
        let label = editor.spec.label.clone();

        // If the choice contains `<file>.md`, stage a pending approval prompt instead of immediately popping open the modal
        if chosen.contains("<file>.md") {
            editor.set_text(&chosen);
            self.sync_values_from_editors();
            self.pending_plan_picker_approval = Some(PendingPlanPickerApproval {
                field_index: field_idx,
                replace_token: "<file>.md".to_string(),
            });
            self.refresh_editor_themes();
            self.status_message =
                "Approve opening .md file picker? [Enter/Alt+Y: Open Picker | Esc/Alt+N: Keep literal]"
                    .to_string();
        } else {
            self.pending_plan_picker_approval = None;
            editor.set_text(&chosen);
            self.sync_values_from_editors();
            self.status_message = format!("{}: selected '{}'", label, chosen);
        }
    }

    /// Approves the staged `.md` plan file picker request (`Enter` / `Alt+Y`), opening `FilePickerModal`.
    pub fn approve_pending_plan_picker(&mut self) -> bool {
        let Some(pending) = self.pending_plan_picker_approval.take() else {
            return false;
        };
        self.file_picker
            .open(pending.field_index, true, Some(pending.replace_token));
        self.refresh_editor_themes();
        self.status_message =
            "Select a .md plan file from current directory to populate <file>.md".to_string();
        true
    }

    /// Declines the staged `.md` plan file picker request (`Esc` / `Alt+N`), keeping `<file>.md` literal.
    pub fn decline_pending_plan_picker(&mut self) -> bool {
        if self.pending_plan_picker_approval.take().is_some() {
            self.refresh_editor_themes();
            self.status_message =
                "Kept literal 'Update <file>.md plan' without opening .md file picker".to_string();
            true
        } else {
            false
        }
    }

    /// Opens the `Alt+U` Model Catalog Sync & Custom Sub-Model Add/Update Modal.
    pub fn open_model_manager_modal(&mut self) {
        self.pending_plan_picker_approval = None;
        self.model_modal
            .open(self.selected_family, self.selected_effort);
        self.refresh_editor_themes();
        self.status_message =
            "Model Catalog Modal (Alt+U): Ctrl+R to sync from `agy models`, or fill fields and press Enter to add/update"
                .to_string();
    }

    /// Runs `agy models`, merges discovered profiles into `self.catalog`, and persists `models.toml`.
    pub fn sync_models_from_agy(&mut self) -> Result<usize> {
        self.sync_values_from_editors();
        let count = self
            .catalog
            .sync_from_agy_cli(self.custom_agy_bin.as_deref())?;
        let saved_path = self
            .catalog
            .save_to_toml_file(self.custom_models_toml_path.as_deref())?;
        let msg = format!(
            "Synced {} model profile(s) from `agy models` -> {}",
            count,
            saved_path.display()
        );
        self.model_modal.status_note = msg.clone();
        self.settings_page.status_note = msg.clone();
        self.status_message = msg;
        Ok(count)
    }

    /// Submits the custom sub-model entered in `ModelManagerModal`, adding or updating it in `self.catalog`,
    /// selecting it immediately while preserving all canonical prompt field values, and persisting `models.toml`.
    pub fn submit_custom_model_from_modal(&mut self) -> Result<usize> {
        self.sync_values_from_editors();
        let family = self.model_modal.family;
        let effort = self.model_modal.default_effort;
        let raw_short = self.model_modal.short_name.trim().to_string();
        let raw_display = self.model_modal.display_name.trim().to_string();
        let raw_base = self.model_modal.agy_base_id.trim().to_string();

        if raw_short.is_empty() && raw_base.is_empty() && raw_display.is_empty() {
            let note = "Enter at least a Short Name or Base agy ID (or press Ctrl+R to sync from `agy models`).".to_string();
            self.model_modal.status_note = note.clone();
            self.status_message = note;
            return Ok(self.selected_profile_idx);
        }

        let short_name = if !raw_short.is_empty() {
            raw_short
        } else if !raw_display.is_empty() {
            raw_display.clone()
        } else {
            raw_base.clone()
        };

        let display_name = if !raw_display.is_empty() {
            raw_display
        } else {
            format!("{} {}", family.as_str(), short_name)
        };

        let slug: String = short_name
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() {
                    c.to_ascii_lowercase()
                } else {
                    '-'
                }
            })
            .collect::<String>()
            .trim_matches('-')
            .to_string();

        let agy_base_id = if !raw_base.is_empty() {
            raw_base
                .trim_end_matches("-low")
                .trim_end_matches("-medium")
                .trim_end_matches("-high")
                .to_ascii_lowercase()
        } else {
            match family {
                ModelFamily::Gemini => format!("gemini-{}", slug),
                ModelFamily::Claude => format!("claude-{}", slug),
                ModelFamily::Gpt => format!("gpt-{}", slug),
            }
        };

        let profile = SubModelProfile {
            family,
            short_name: short_name.clone(),
            display_name: display_name.clone(),
            agy_base_id: agy_base_id.clone(),
            available_tiers: vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
            ],
            default_effort: effort,
            format_style: family.default_format_style(),
        };

        let idx = self.catalog.upsert_profile(profile);
        self.selected_profile_idx = idx;
        self.selected_family = family;
        self.selected_effort = effort;
        self.explicit_agy_model_id = None;
        self.workspace_state.selected_model_family = family.as_str().to_string();
        self.workspace_state.selected_sub_model = short_name;
        self.workspace_state.selected_effort = effort.as_str().to_string();

        let _ = self
            .catalog
            .save_to_toml_file(self.custom_models_toml_path.as_deref());
        self.persist_workspace_state();
        self.model_modal.close();
        self.refresh_editor_themes();
        self.status_message = format!(
            "Added/updated model '{}' ({})",
            display_name,
            self.active_agy_model_id()
        );
        Ok(idx)
    }

    /// Toggles pinning of the current `Constraints` field value in `WorkspaceState` (`Ctrl+K`).
    pub fn toggle_pin_current_constraint(&mut self) {
        self.sync_values_from_editors();
        let current = self.values.constraints.trim().to_string();
        if current.is_empty() {
            if !self.workspace_state.pinned_constraints.is_empty() {
                let applied = self.workspace_state.formatted_pinned_constraints();
                self.values.constraints = applied.clone();
                self.sync_editors_from_values();
                self.status_message = format!("Restored pinned constraints: {}", applied);
            } else {
                self.status_message =
                    "Constraints field is empty; enter a constraint (e.g. 'No QEMU') then press Ctrl+K to pin".to_string();
            }
            return;
        }

        // Toggle each semicolon-separated token or the whole constraint line
        let pinned_now = self.workspace_state.toggle_pinned_constraint(&current);
        self.settings_page.pinned_constraints_input =
            self.workspace_state.formatted_pinned_constraints();
        self.persist_workspace_state();
        self.refresh_editor_themes();
        if pinned_now {
            self.status_message = format!(
                "Pinned constraint '{}' for workspace ({})",
                current,
                self.state_location.path().display()
            );
        } else {
            self.status_message = format!("Unpinned constraint '{}'", current);
        }
    }

    /// Cycles through recent prompts in the per-directory `WorkspaceState` ring buffer (`Ctrl+H`).
    pub fn cycle_prompt_history(&mut self) {
        let len = self.workspace_state.prompt_history.len();
        if len == 0 {
            self.status_message = "No prompt history recorded for this directory yet".to_string();
            return;
        }
        let next_idx = match self.history_cursor {
            Some(0) => len - 1,
            Some(i) => i - 1,
            None => len - 1,
        };
        self.history_cursor = Some(next_idx);
        if let Some(entry) = self.workspace_state.prompt_history.get(next_idx).cloned() {
            self.values = entry.values;
            self.sync_editors_from_values();
            self.status_message = format!(
                "Loaded history [{}/{}] from {} (model: {})",
                next_idx + 1,
                len,
                entry.timestamp_utc,
                entry.model_id
            );
        }
    }

    /// Opens the `@` / `Ctrl+F` CWD fuzzy file picker modal for the active field editor.
    pub fn open_file_picker_for_active_field(
        &mut self,
        markdown_only: bool,
        replace_token: Option<String>,
    ) {
        let target_idx = match self.focus {
            FocusPane::FieldEditor(idx) => idx,
            _ => self
                .editors
                .iter()
                .position(|e| e.spec.id == "context_logs")
                .unwrap_or(0),
        };
        self.focus = FocusPane::FieldEditor(target_idx);
        self.file_picker
            .open(target_idx, markdown_only, replace_token);
        self.refresh_editor_themes();
        self.status_message =
            "CWD File Picker: Type to fuzzy-filter files, Up/Down to select, Enter to insert, Esc to cancel"
                .to_string();
    }

    /// Confirms the currently highlighted file in the fuzzy file picker modal and inserts/processes its relative path.
    pub fn confirm_file_picker_selection(&mut self) -> Option<String> {
        if !self.file_picker.is_open {
            return None;
        }
        let matches = self.file_picker.matches(&self.file_index, 50);
        let chosen_rel = matches
            .get(self.file_picker.selected_index)
            .map(|f| f.relative_path.clone())?;

        let target = self.file_picker.target;
        let replace_token = self.file_picker.replace_token.clone();
        let was_markdown_plan = self.file_picker.markdown_only;
        self.file_picker.close();

        match target {
            FilePickerTarget::SettingsModelIngest => {
                self.settings_page.model_ingest_path = chosen_rel.clone();
                let path = PathBuf::from(&chosen_rel);
                if let Err(err) = self.ingest_model_file_from_settings(&path) {
                    let msg = format!("Model TOML ingest error: {}", err);
                    self.settings_page.status_note = msg.clone();
                    self.status_message = msg;
                }
                return Some(chosen_rel);
            }
            FilePickerTarget::SettingsTemplateIngest => {
                self.settings_page.template_ingest_path = chosen_rel.clone();
                let path = PathBuf::from(&chosen_rel);
                if let Err(err) = self.ingest_template_from_settings(&path) {
                    let msg = format!("Template ingest error: {}", err);
                    self.settings_page.status_note = msg.clone();
                    self.status_message = msg;
                }
                return Some(chosen_rel);
            }
            FilePickerTarget::FieldEditor(target_idx) => {
                if let Some(editor) = self.editors.get_mut(target_idx) {
                    if let Some(token) = replace_token {
                        let current_text = editor.text();
                        if current_text.contains(&token) {
                            let updated = current_text.replace(&token, &chosen_rel);
                            editor.set_text(&updated);
                        } else {
                            editor.insert_str(&chosen_rel);
                        }
                    } else {
                        let current_text = editor.text();
                        let needs_leading_space = !current_text.is_empty()
                            && !current_text.ends_with(|c: char| {
                                c.is_whitespace() || c == ',' || c == '@'
                            });
                        if needs_leading_space {
                            editor.insert_str(&format!(", {}", chosen_rel));
                        } else {
                            editor.insert_str(&chosen_rel);
                        }
                    }
                }

                if was_markdown_plan {
                    self.workspace_state.default_plan_path = Some(chosen_rel.clone());
                    self.settings_page.default_plan_path_input = chosen_rel.clone();
                    self.persist_workspace_state();
                }

                self.sync_values_from_editors();
                self.refresh_editor_themes();
                self.status_message = format!("Inserted '{}' into field", chosen_rel);
                Some(chosen_rel)
            }
        }
    }

    /// Saves the currently rendered prompt to a Markdown file in the working directory (`Ctrl+S` or Action 4).
    pub fn save_rendered_prompt_to_md(&mut self, custom_path: Option<&Path>) -> Result<PathBuf> {
        self.sync_values_from_editors();
        let rendered = self.rendered_prompt();
        let target_path = custom_path
            .map(PathBuf::from)
            .unwrap_or_else(|| self.cwd.join("verbot-prompt.md"));
        if let Some(parent) = target_path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent)?;
            }
        }
        fs::write(&target_path, format!("{}\n", rendered)).with_context(|| {
            format!(
                "Failed to save rendered prompt to {}",
                target_path.display()
            )
        })?;
        self.saved_markdown_path = Some(target_path.clone());
        self.status_message = format!("Saved rendered prompt to {}", target_path.display());
        Ok(target_path)
    }

    /// Interactive dispatch entrypoint triggered by `Enter` on `ActionBar`, `Ctrl+Enter`, or `Alt+1..4`.
    /// When `ShellFormattingPolicy::PromptOnSpecialChars` is active and shell metacharacters or
    /// newlines are present, opens the confirmation modal (`ShellQuoteModal`) before dispatching.
    pub fn request_dispatch(&mut self, action: DispatchAction) -> Result<Option<DispatchManifest>> {
        self.selected_action = action;
        self.sync_values_from_editors();

        let report = self.inspect_prompt_shell_safety();
        if self.global_config.shell_formatting_policy == ShellFormattingPolicy::PromptOnSpecialChars
            && report.needs_escaping_or_quoting()
        {
            let (_, shell_safe_cmd) = self.build_shell_safe_agy_for_action(action);
            self.shell_quote_modal.open(action, report, shell_safe_cmd);
            self.refresh_editor_themes();
            self.status_message =
                "Shell Metacharacters Detected: [Enter/Alt+E: Auto-Escape & Quote | Alt+A: Always Auto-Escape | Alt+R/Esc: Edit Manually]"
                    .to_string();
            return Ok(None);
        }

        let manifest = self.confirm_dispatch(action)?;
        Ok(Some(manifest))
    }

    /// Finalizes the user's selected [`DispatchAction`], records the prompt in workspace history,
    /// saves workspace state, populates `self.confirmed_manifest`, and executes or stages the action
    /// inside Verbocity without closing the application (`self.should_quit` remains `false`).
    pub fn confirm_dispatch(&mut self, action: DispatchAction) -> Result<DispatchManifest> {
        self.selected_action = action;
        self.sync_values_from_editors();

        let model_id = self.active_agy_model_id();
        let rendered = self.rendered_prompt();
        let agy_preview = self.agy_command_preview();
        let (agy_argv, shell_safe_command) = self.build_shell_safe_agy_for_action(action);
        let agy_extra_flags = self.global_config.agy_flags.to_cli_args();

        let output_md_path = if action == DispatchAction::SaveMarkdown {
            let custom_md = self.saved_markdown_path.clone();
            let p = self.save_rendered_prompt_to_md(custom_md.as_deref())?;
            Some(p.to_string_lossy().to_string())
        } else {
            self.saved_markdown_path
                .as_ref()
                .map(|p| p.to_string_lossy().to_string())
        };

        self.workspace_state.record_prompt(
            model_id.clone(),
            self.selected_effort,
            self.values.clone(),
            rendered.clone(),
        );
        self.persist_workspace_state();

        let manifest = DispatchManifest {
            action: action.slug().to_string(),
            model_id,
            model_display_name: self.active_profile().display_name.clone(),
            effort: self.selected_effort.as_str().to_string(),
            slash_command: self.values.formatted_slash_command(),
            rendered_prompt: rendered,
            agy_command_preview: agy_preview,
            shell_safe_command,
            agy_argv,
            agy_extra_flags,
            shell_formatting_policy: self
                .global_config
                .shell_formatting_policy
                .as_str()
                .to_string(),
            output_md_path,
            cwd: self.cwd.to_string_lossy().to_string(),
        };

        self.confirmed_manifest = Some(manifest.clone());
        // Do NOT set `self.should_quit = true` here — Verbocity remains open after executing actions.
        self.execute_in_app_dispatch(&manifest)?;
        Ok(manifest)
    }

    /// Resolves the `agy` binary path from `self.custom_agy_bin`, `VERBOT_AGY_BIN`, `PATH`, or `~/.local/bin/agy`.
    pub fn resolve_agy_executable(&self) -> PathBuf {
        if let Some(ref custom) = self.custom_agy_bin {
            return custom.clone();
        }
        if let Ok(env_bin) = std::env::var("VERBOT_AGY_BIN") {
            if !env_bin.trim().is_empty() {
                return PathBuf::from(env_bin);
            }
        }
        if let Some(home_agy) = dirs::home_dir().map(|h| h.join(".local/bin/agy")) {
            if home_agy.is_file() {
                return home_agy;
            }
        }
        PathBuf::from("agy")
    }

    /// Executes non-interactive actions (`agy -p`, `Stdout / Clipboard`, `Save .md`) directly into
    /// the in-app [`TerminalOutputDrawer`] (`FocusPane::TerminalOutput`) and stages `agy -i` for
    /// suspend/resume terminal handoff while keeping `self.should_quit == false`.
    pub fn execute_in_app_dispatch(&mut self, manifest: &DispatchManifest) -> Result<()> {
        let action = DispatchAction::from_slug(&manifest.action)
            .unwrap_or(DispatchAction::StdoutClipboard);

        match action {
            DispatchAction::AgyPrint => {
                self.terminal_output.is_open = true;
                self.terminal_output.is_running = true;
                self.terminal_output.last_command = manifest.shell_safe_command.clone();
                self.terminal_output.push_line("────────────────────────────────────────────────────────────────────────");
                self.terminal_output.push_line(format!(
                    "$ {}",
                    manifest.agy_command_preview
                ));

                let agy_bin = self.resolve_agy_executable();
                let dry_run = std::env::var("VERBOT_AGY_DRY_RUN")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);

                if dry_run {
                    let extra_joined = if manifest.agy_extra_flags.is_empty() {
                        String::new()
                    } else {
                        format!(" {}", manifest.agy_extra_flags.join(" "))
                    };
                    self.terminal_output.push_line(format!(
                        "[Verbocity Dry-Run] {} --model {} --effort {}{} -p",
                        agy_bin.display(),
                        manifest.model_id,
                        manifest.effort,
                        extra_joined
                    ));
                    self.terminal_output
                        .append_multiline(&manifest.rendered_prompt);
                    self.terminal_output.last_exit_code = Some(0);
                    self.terminal_output.is_running = false;
                    self.focus = FocusPane::TerminalOutput;
                    self.refresh_editor_themes();
                    self.status_message =
                        "Executed agy -p (dry-run) in Terminal Output Pane [Ctrl+O to toggle, Tab to return to fields]"
                            .to_string();
                    return Ok(());
                }

                let mut cmd = Command::new(&agy_bin);
                cmd.current_dir(&self.cwd)
                    .arg("--model")
                    .arg(&manifest.model_id)
                    .arg("--effort")
                    .arg(&manifest.effort)
                    .args(&manifest.agy_extra_flags)
                    .arg("-p")
                    .arg(&manifest.rendered_prompt)
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped());

                match cmd.spawn() {
                    Ok(mut child) => {
                        if let Some(stdout) = child.stdout.take() {
                            let reader = BufReader::new(stdout);
                            for line in reader.lines().map_while(Result::ok) {
                                self.terminal_output.push_line(line);
                            }
                        }
                        if let Some(stderr) = child.stderr.take() {
                            let reader = BufReader::new(stderr);
                            for line in reader.lines().map_while(Result::ok) {
                                self.terminal_output.push_line(format!("[stderr] {}", line));
                            }
                        }
                        let code = child.wait().ok().and_then(|s| s.code()).unwrap_or(1);
                        self.terminal_output.last_exit_code = Some(code);
                        self.terminal_output
                            .push_line(format!("==> [agy -p exited with status {}]", code));
                        self.status_message = format!(
                            "agy -p completed (exit {}) — output streamed to Terminal Output Pane (Ctrl+O)",
                            code
                        );
                    }
                    Err(err) => {
                        self.terminal_output.last_exit_code = Some(127);
                        self.terminal_output.push_line(format!(
                            "[Error spawning '{}': {}]",
                            agy_bin.display(),
                            err
                        ));
                        self.terminal_output.push_line("--- Rendered Prompt ---");
                        self.terminal_output
                            .append_multiline(&manifest.rendered_prompt);
                        self.status_message = format!(
                            "Could not spawn '{}' ({}) — prompt displayed in Terminal Output Pane",
                            agy_bin.display(),
                            err
                        );
                    }
                }
                self.terminal_output.is_running = false;
                self.focus = FocusPane::TerminalOutput;
                self.refresh_editor_themes();
            }
            DispatchAction::StdoutClipboard => {
                self.terminal_output.is_open = true;
                self.terminal_output.last_command =
                    "Output to Stdout / System Clipboard".to_string();
                self.terminal_output.push_line("────────────────────────────────────────────────────────────────────────");
                self.terminal_output
                    .push_line("==> [Action 3: Rendered Prompt Output / Clipboard]");
                self.terminal_output
                    .append_multiline(&manifest.rendered_prompt);
                let copied = try_copy_to_system_clipboard(&manifest.rendered_prompt);
                self.terminal_output.last_exit_code = Some(0);
                if copied {
                    self.terminal_output
                        .push_line("==> [Copied rendered prompt to system clipboard]");
                    self.status_message =
                        "Rendered prompt copied to clipboard and displayed in Terminal Output Pane (Ctrl+O)"
                            .to_string();
                } else {
                    self.status_message =
                        "Rendered prompt displayed in Terminal Output Pane (Ctrl+O to toggle)"
                            .to_string();
                }
                self.focus = FocusPane::TerminalOutput;
                self.refresh_editor_themes();
            }
            DispatchAction::SaveMarkdown => {
                self.terminal_output.is_open = true;
                let saved_path = manifest
                    .output_md_path
                    .as_deref()
                    .unwrap_or("verbot-prompt.md");
                self.terminal_output.last_command = format!("Save to {}", saved_path);
                self.terminal_output.push_line("────────────────────────────────────────────────────────────────────────");
                self.terminal_output
                    .push_line(format!("==> [Action 4: Saved Rendered Prompt to {}]", saved_path));
                self.terminal_output
                    .append_multiline(&manifest.rendered_prompt);
                self.terminal_output.last_exit_code = Some(0);
                self.status_message = format!(
                    "Saved rendered prompt to {} (Verbocity remains open; Ctrl+O toggles Output Pane)",
                    saved_path
                );
                self.focus = FocusPane::TerminalOutput;
                self.refresh_editor_themes();
            }
            DispatchAction::AgyInteractive => {
                self.terminal_output.is_open = true;
                self.terminal_output.last_command = manifest.shell_safe_command.clone();
                self.terminal_output.push_line("────────────────────────────────────────────────────────────────────────");
                self.terminal_output.push_line(format!(
                    "$ {}",
                    manifest.agy_command_preview
                ));

                // Stage interactive handoff for `run_interactive_tui` (or execute immediately if
                // running in a non-TUI test harness where custom_agy_bin or dry-run is active).
                self.pending_interactive_handoff = Some(manifest.clone());

                let dry_run = std::env::var("VERBOT_AGY_DRY_RUN")
                    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                if dry_run || self.custom_agy_bin.is_some() {
                    let _ = self.run_interactive_handoff_child(manifest);
                    self.pending_interactive_handoff = None;
                } else {
                    self.status_message =
                        "Staging agy -i interactive session handoff (Verbocity will resume when agy exits)..."
                            .to_string();
                }
            }
        }
        Ok(())
    }

    /// Runs the child `agy -i` interactive session and records its exit status in [`TerminalOutputDrawer`]
    /// so Verbocity resumes cleanly after `agy -i` exits.
    pub fn run_interactive_handoff_child(&mut self, manifest: &DispatchManifest) -> Result<i32> {
        let agy_bin = self.resolve_agy_executable();
        let dry_run = std::env::var("VERBOT_AGY_DRY_RUN")
            .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
            .unwrap_or(false);

        if dry_run {
            let extra_joined = if manifest.agy_extra_flags.is_empty() {
                String::new()
            } else {
                format!(" {}", manifest.agy_extra_flags.join(" "))
            };
            self.terminal_output.push_line(format!(
                "[Verbocity Dry-Run] Interactive handoff: {} --model {} --effort {}{} -i",
                agy_bin.display(),
                manifest.model_id,
                manifest.effort,
                extra_joined
            ));
            self.terminal_output.last_exit_code = Some(0);
            self.status_message =
                "Resumed Verbocity after agy -i (dry-run) session (all prompt fields preserved)"
                    .to_string();
            return Ok(0);
        }

        let mut cmd = Command::new(&agy_bin);
        cmd.current_dir(&self.cwd)
            .arg("--model")
            .arg(&manifest.model_id)
            .arg("--effort")
            .arg(&manifest.effort)
            .args(&manifest.agy_extra_flags)
            .arg("-i")
            .arg(&manifest.rendered_prompt);

        // When a custom mock `agy` binary is used in headless self-tests, capture its output into the drawer
        if self.custom_agy_bin.is_some() {
            match cmd.output() {
                Ok(out) => {
                    let stdout_s = String::from_utf8_lossy(&out.stdout);
                    if !stdout_s.trim().is_empty() {
                        self.terminal_output.append_multiline(stdout_s.trim());
                    }
                    let stderr_s = String::from_utf8_lossy(&out.stderr);
                    if !stderr_s.trim().is_empty() {
                        self.terminal_output.append_multiline(stderr_s.trim());
                    }
                    let code = out.status.code().unwrap_or(0);
                    self.terminal_output.last_exit_code = Some(code);
                    self.terminal_output.push_line(format!(
                        "==> [Resumed Verbocity: agy -i session exited with status {}]",
                        code
                    ));
                    self.status_message = format!(
                        "Resumed Verbocity after agy -i (exit {}) — all prompt fields intact",
                        code
                    );
                    return Ok(code);
                }
                Err(err) => {
                    self.terminal_output.last_exit_code = Some(127);
                    self.terminal_output.push_line(format!(
                        "[Error launching '{}': {}]",
                        agy_bin.display(),
                        err
                    ));
                    return Ok(127);
                }
            }
        }

        match cmd.status() {
            Ok(status) => {
                let code = status.code().unwrap_or(0);
                self.terminal_output.last_exit_code = Some(code);
                self.terminal_output.push_line(format!(
                    "==> [Resumed Verbocity: agy -i session exited with status {}]",
                    code
                ));
                self.status_message = format!(
                    "Resumed Verbocity after agy -i session (exit {}) — all prompt fields preserved",
                    code
                );
                Ok(code)
            }
            Err(err) => {
                self.terminal_output.last_exit_code = Some(127);
                self.terminal_output.push_line(format!(
                    "[Error launching '{} -i': {}]",
                    agy_bin.display(),
                    err
                ));
                self.status_message = format!(
                    "Failed to launch '{}' ({}) — resumed Verbocity with all fields preserved",
                    agy_bin.display(),
                    err
                );
                Ok(127)
            }
        }
    }

    pub fn persist_workspace_state(&mut self) {
        let use_local = self.force_local_state || self.global_config.workspace_storage_local;
        if let Ok(loc) = self.workspace_state.save_for_dir(
            &self.cwd,
            use_local,
            self.custom_xdg_data_dir.as_deref(),
        ) {
            self.state_location = loc;
        }
    }

    /// Moves focus forward (`Tab`) across ControlBar, FieldEditors, ActionBar, and TerminalOutput (when open).
    pub fn focus_next(&mut self) {
        self.focus = match self.focus {
            FocusPane::ControlBar(item) => {
                if item == ControlItem::Settings {
                    if self.editors.is_empty() {
                        FocusPane::ActionBar
                    } else {
                        FocusPane::FieldEditor(0)
                    }
                } else {
                    FocusPane::ControlBar(item.next())
                }
            }
            FocusPane::FieldEditor(idx) => {
                if idx + 1 < self.editors.len() {
                    FocusPane::FieldEditor(idx + 1)
                } else {
                    FocusPane::ActionBar
                }
            }
            FocusPane::ActionBar => {
                if self.terminal_output.is_open {
                    FocusPane::TerminalOutput
                } else {
                    FocusPane::ControlBar(ControlItem::ModelFamily)
                }
            }
            FocusPane::TerminalOutput => FocusPane::ControlBar(ControlItem::ModelFamily),
        };
        self.refresh_editor_themes();
    }

    /// Moves focus backward (`Shift+Tab` / `BackTab`) across TerminalOutput, ActionBar, FieldEditors, and ControlBar.
    pub fn focus_prev(&mut self) {
        self.focus = match self.focus {
            FocusPane::ControlBar(item) => {
                if item == ControlItem::ModelFamily {
                    if self.terminal_output.is_open {
                        FocusPane::TerminalOutput
                    } else {
                        FocusPane::ActionBar
                    }
                } else {
                    FocusPane::ControlBar(item.prev())
                }
            }
            FocusPane::FieldEditor(idx) => {
                if idx > 0 {
                    FocusPane::FieldEditor(idx - 1)
                } else {
                    FocusPane::ControlBar(ControlItem::Settings)
                }
            }
            FocusPane::ActionBar => {
                if self.editors.is_empty() {
                    FocusPane::ControlBar(ControlItem::Settings)
                } else {
                    FocusPane::FieldEditor(self.editors.len() - 1)
                }
            }
            FocusPane::TerminalOutput => FocusPane::ActionBar,
        };
        self.refresh_editor_themes();
    }

    /// Toggles the in-app Terminal Execution Output Pane (`Ctrl+O`).
    pub fn toggle_terminal_output(&mut self) {
        self.terminal_output.is_open = !self.terminal_output.is_open;
        if self.terminal_output.is_open {
            self.focus = FocusPane::TerminalOutput;
            self.status_message =
                "Terminal Execution Output Pane opened [Up/Down/PgUp/PgDn: Scroll | Ctrl+L: Clear | Ctrl+O/Esc: Close]"
                    .to_string();
        } else {
            if self.focus == FocusPane::TerminalOutput {
                self.focus = if self.editors.is_empty() {
                    FocusPane::ActionBar
                } else {
                    FocusPane::FieldEditor(0)
                };
            }
            self.status_message = "Terminal Execution Output Pane collapsed".to_string();
        }
        self.refresh_editor_themes();
    }

    /// Handles key events while the full-screen Settings Page (`Alt+S`) is active.
    fn handle_settings_key_event(&mut self, key: KeyEvent) -> Result<()> {
        // Global exit from Settings Page (`Esc` or `Alt+S`) preserves all prompt field text
        if key.code == KeyCode::Esc
            || (key.modifiers.contains(KeyModifiers::ALT)
                && matches!(key.code, KeyCode::Char('s') | KeyCode::Char('S')))
        {
            self.close_settings_page();
            return Ok(());
        }

        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    self.should_quit = true;
                    return Ok(());
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.save_current_settings_tab()?;
                    return Ok(());
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    if let Err(err) = self.sync_models_from_agy() {
                        let msg = format!("agy models sync error: {}", err);
                        self.settings_page.status_note = msg.clone();
                        self.status_message = msg;
                    }
                    return Ok(());
                }
                KeyCode::Char('i') | KeyCode::Char('I') => {
                    self.settings_page.switch_tab(SettingsTab::ModelsAndToml);
                    self.file_picker
                        .open_for_target(FilePickerTarget::SettingsModelIngest, false);
                    return Ok(());
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    if self.settings_page.active_tab == SettingsTab::TemplatesAndDefaults {
                        self.file_picker
                            .open_for_target(FilePickerTarget::SettingsTemplateIngest, false);
                    } else {
                        self.cycle_theme();
                    }
                    return Ok(());
                }
                KeyCode::Left => {
                    let prev_tab = self.settings_page.active_tab.prev();
                    self.settings_page.switch_tab(prev_tab);
                    return Ok(());
                }
                KeyCode::Right => {
                    let next_tab = self.settings_page.active_tab.next();
                    self.settings_page.switch_tab(next_tab);
                    return Ok(());
                }
                _ => {}
            }
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('1') => {
                    self.settings_page.switch_tab(SettingsTab::ThemeAndContrast);
                    return Ok(());
                }
                KeyCode::Char('2') => {
                    self.settings_page.switch_tab(SettingsTab::ModelsAndToml);
                    return Ok(());
                }
                KeyCode::Char('3') => {
                    self.settings_page
                        .switch_tab(SettingsTab::TemplatesAndDefaults);
                    return Ok(());
                }
                KeyCode::Char('4') => {
                    self.settings_page
                        .switch_tab(SettingsTab::ExecutionAndWorkspace);
                    return Ok(());
                }
                KeyCode::Left => {
                    let prev_tab = self.settings_page.active_tab.prev();
                    self.settings_page.switch_tab(prev_tab);
                    return Ok(());
                }
                KeyCode::Right => {
                    let next_tab = self.settings_page.active_tab.next();
                    self.settings_page.switch_tab(next_tab);
                    return Ok(());
                }
                KeyCode::Char('r') | KeyCode::Char('R') => {
                    let _ = self.sync_models_from_agy();
                    return Ok(());
                }
                _ => {}
            }
        }

        // `1..4` (or `F1..F4` / `[` / `]`) switches tabs when not actively typing into a text input row
        if !self.settings_page.is_text_input_row_focused() {
            match key.code {
                KeyCode::Char('1') | KeyCode::F(1) => {
                    self.settings_page.switch_tab(SettingsTab::ThemeAndContrast);
                    return Ok(());
                }
                KeyCode::Char('2') | KeyCode::F(2) => {
                    self.settings_page.switch_tab(SettingsTab::ModelsAndToml);
                    return Ok(());
                }
                KeyCode::Char('3') | KeyCode::F(3) => {
                    self.settings_page
                        .switch_tab(SettingsTab::TemplatesAndDefaults);
                    return Ok(());
                }
                KeyCode::Char('4') | KeyCode::F(4) => {
                    self.settings_page
                        .switch_tab(SettingsTab::ExecutionAndWorkspace);
                    return Ok(());
                }
                KeyCode::Char('[') => {
                    let prev_tab = self.settings_page.active_tab.prev();
                    self.settings_page.switch_tab(prev_tab);
                    return Ok(());
                }
                KeyCode::Char(']') => {
                    let next_tab = self.settings_page.active_tab.next();
                    self.settings_page.switch_tab(next_tab);
                    return Ok(());
                }
                _ => {}
            }
        } else {
            match key.code {
                KeyCode::F(1) => {
                    self.settings_page.switch_tab(SettingsTab::ThemeAndContrast);
                    return Ok(());
                }
                KeyCode::F(2) => {
                    self.settings_page.switch_tab(SettingsTab::ModelsAndToml);
                    return Ok(());
                }
                KeyCode::F(3) => {
                    self.settings_page
                        .switch_tab(SettingsTab::TemplatesAndDefaults);
                    return Ok(());
                }
                KeyCode::F(4) => {
                    self.settings_page
                        .switch_tab(SettingsTab::ExecutionAndWorkspace);
                    return Ok(());
                }
                _ => {}
            }
        }

        // Row navigation (`Up` / `Down` / `Tab` / `BackTab`)
        match key.code {
            KeyCode::Down | KeyCode::Tab => {
                self.settings_page.next_row();
                return Ok(());
            }
            KeyCode::Up | KeyCode::BackTab => {
                self.settings_page.prev_row();
                return Ok(());
            }
            _ => {}
        }

        // Tab-specific row editing
        match self.settings_page.active_tab {
            SettingsTab::ThemeAndContrast => self.handle_settings_tab_theme(key)?,
            SettingsTab::ModelsAndToml => self.handle_settings_tab_models(key)?,
            SettingsTab::TemplatesAndDefaults => self.handle_settings_tab_templates(key)?,
            SettingsTab::ExecutionAndWorkspace => self.handle_settings_tab_execution(key)?,
        }

        Ok(())
    }

    fn handle_settings_tab_theme(&mut self, key: KeyEvent) -> Result<()> {
        let row = self.settings_page.selected_row;
        match row {
            0 => {
                // Theme Preset selector
                match key.code {
                    KeyCode::Left => {
                        let prev = match self.theme.preset {
                            ThemePreset::NovosCosmicDark => ThemePreset::TerminalAnsi,
                            ThemePreset::CosmicSystemAuto => ThemePreset::NovosCosmicDark,
                            ThemePreset::NovosSolarLight => ThemePreset::CosmicSystemAuto,
                            ThemePreset::TerminalAnsi => ThemePreset::NovosSolarLight,
                            ThemePreset::Custom => ThemePreset::NovosCosmicDark,
                        };
                        self.set_theme_preset(prev);
                    }
                    KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                        self.cycle_theme();
                    }
                    _ => {}
                }
            }
            1..=6 => {
                let buf = match row {
                    1 => &mut self.settings_page.hex_accent_primary,
                    2 => &mut self.settings_page.hex_accent_secondary,
                    3 => &mut self.settings_page.hex_text_primary,
                    4 => &mut self.settings_page.hex_title_inactive,
                    5 => &mut self.settings_page.hex_border_slate,
                    _ => &mut self.settings_page.hex_placeholder_muted,
                };
                match key.code {
                    KeyCode::Backspace => {
                        buf.pop();
                    }
                    KeyCode::Enter => {
                        self.apply_settings_theme_hex_overrides()?;
                    }
                    KeyCode::Char(c)
                        if c == '#' || c.is_ascii_hexdigit() =>
                    {
                        if buf.len() < 7 {
                            buf.push(c.to_ascii_uppercase());
                        }
                        if buf.len() == 7 && buf.starts_with('#') {
                            let _ = self.apply_settings_theme_hex_overrides();
                        }
                    }
                    _ => {}
                }
            }
            7 => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
                    self.apply_settings_theme_hex_overrides()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_settings_tab_models(&mut self, key: KeyEvent) -> Result<()> {
        let row = self.settings_page.selected_row;
        match row {
            0 => match key.code {
                KeyCode::Backspace => {
                    self.settings_page.model_ingest_path.pop();
                }
                KeyCode::Enter => {
                    let raw_path = self.settings_page.model_ingest_path.trim().to_string();
                    if raw_path.is_empty() {
                        self.file_picker
                            .open_for_target(FilePickerTarget::SettingsModelIngest, false);
                    } else {
                        let path = PathBuf::from(&raw_path);
                        if let Err(err) = self.ingest_model_file_from_settings(&path) {
                            let msg = format!("Model TOML ingest error: {}", err);
                            self.settings_page.status_note = msg.clone();
                            self.status_message = msg;
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.settings_page.model_ingest_path.push(c);
                }
                _ => {}
            },
            1 => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
                    if let Err(err) = self.sync_models_from_agy() {
                        let msg = format!("agy models sync error: {}", err);
                        self.settings_page.status_note = msg.clone();
                        self.status_message = msg;
                    }
                }
            }
            2 => match key.code {
                KeyCode::Left => {
                    self.settings_page.custom_family = self.settings_page.custom_family.prev();
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    self.settings_page.custom_family = self.settings_page.custom_family.next();
                }
                _ => {}
            },
            3..=6 => {
                let target_buf = match row {
                    3 => &mut self.settings_page.custom_short_name,
                    4 => &mut self.settings_page.custom_display_name,
                    5 => &mut self.settings_page.custom_agy_base_id,
                    _ => &mut self.settings_page.custom_available_tiers,
                };
                match key.code {
                    KeyCode::Backspace => {
                        target_buf.pop();
                    }
                    KeyCode::Enter => {
                        self.submit_custom_model_from_settings()?;
                    }
                    KeyCode::Char(c)
                        if !key.modifiers.contains(KeyModifiers::CONTROL)
                            && !key.modifiers.contains(KeyModifiers::ALT) =>
                    {
                        target_buf.push(c);
                    }
                    _ => {}
                }
            }
            7 => match key.code {
                KeyCode::Left => {
                    self.settings_page.custom_default_effort =
                        self.settings_page.custom_default_effort.prev();
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    self.settings_page.custom_default_effort =
                        self.settings_page.custom_default_effort.next();
                }
                _ => {}
            },
            8 => {
                if matches!(key.code, KeyCode::Enter | KeyCode::Char(' ')) {
                    self.submit_custom_model_from_settings()?;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_settings_tab_templates(&mut self, key: KeyEvent) -> Result<()> {
        let row = self.settings_page.selected_row;
        match row {
            0 => match key.code {
                KeyCode::Left => self.cycle_active_template(false),
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    self.cycle_active_template(true)
                }
                _ => {}
            },
            1 => match key.code {
                KeyCode::Backspace => {
                    self.settings_page.template_ingest_path.pop();
                }
                KeyCode::Enter => {
                    let raw_path = self.settings_page.template_ingest_path.trim().to_string();
                    if raw_path.is_empty() {
                        self.file_picker
                            .open_for_target(FilePickerTarget::SettingsTemplateIngest, false);
                    } else {
                        let path = PathBuf::from(&raw_path);
                        if let Err(err) = self.ingest_template_from_settings(&path) {
                            let msg = format!("Template ingest error: {}", err);
                            self.settings_page.status_note = msg.clone();
                            self.status_message = msg;
                        }
                    }
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.settings_page.template_ingest_path.push(c);
                }
                _ => {}
            },
            2 => {
                if matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                ) {
                    self.settings_page.template_ingest_local =
                        !self.settings_page.template_ingest_local;
                }
            }
            3 => match key.code {
                KeyCode::Left => {
                    let f = ModelFamily::from_str_loose(&self.global_config.default_model_family)
                        .prev();
                    self.global_config.default_model_family = f.as_str().to_string();
                    self.persist_global_config();
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    let f = ModelFamily::from_str_loose(&self.global_config.default_model_family)
                        .next();
                    self.global_config.default_model_family = f.as_str().to_string();
                    self.persist_global_config();
                }
                _ => {}
            },
            4 => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    let forward = !matches!(key.code, KeyCode::Left);
                    if !self.catalog.profiles.is_empty() {
                        let cur = self
                            .catalog
                            .profiles
                            .iter()
                            .position(|p| {
                                p.short_name
                                    .eq_ignore_ascii_case(&self.global_config.default_sub_model)
                            })
                            .unwrap_or(0);
                        let len = self.catalog.profiles.len();
                        let next_idx = if forward {
                            (cur + 1) % len
                        } else {
                            (cur + len - 1) % len
                        };
                        self.global_config.default_sub_model =
                            self.catalog.profiles[next_idx].short_name.clone();
                        self.global_config.default_model_family = self.catalog.profiles[next_idx]
                            .family
                            .as_str()
                            .to_string();
                        self.persist_global_config();
                    }
                }
                _ => {}
            },
            5 => match key.code {
                KeyCode::Left => {
                    let e = EffortLevel::from_str_loose(&self.global_config.default_effort).prev();
                    self.global_config.default_effort = e.as_str().to_string();
                    self.persist_global_config();
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    let e = EffortLevel::from_str_loose(&self.global_config.default_effort).next();
                    self.global_config.default_effort = e.as_str().to_string();
                    self.persist_global_config();
                }
                _ => {}
            },
            6 => match key.code {
                KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    let forward = !matches!(key.code, KeyCode::Left);
                    let opts = self.slash_command_options();
                    let cur = opts
                        .iter()
                        .position(|o| o == &self.global_config.default_slash_command)
                        .unwrap_or(0);
                    let next_idx = if forward {
                        (cur + 1) % opts.len()
                    } else {
                        (cur + opts.len() - 1) % opts.len()
                    };
                    self.global_config.default_slash_command = opts[next_idx].clone();
                    self.persist_global_config();
                }
                _ => {}
            },
            7 => {
                if matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                ) {
                    self.global_config.show_preview_on_startup =
                        !self.global_config.show_preview_on_startup;
                    self.show_preview = self.global_config.show_preview_on_startup;
                    self.persist_global_config();
                    let msg = format!(
                        "Saved startup defaults to config.toml (show_preview={})",
                        self.global_config.show_preview_on_startup
                    );
                    self.settings_page.status_note = msg.clone();
                    self.status_message = msg;
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn handle_settings_tab_execution(&mut self, key: KeyEvent) -> Result<()> {
        let row = self.settings_page.selected_row;
        match row {
            0 => match key.code {
                KeyCode::Left => {
                    self.global_config.agy_flags.mode = self.global_config.agy_flags.mode.prev();
                    self.persist_global_config();
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    self.global_config.agy_flags.mode = self.global_config.agy_flags.mode.next();
                    self.persist_global_config();
                }
                _ => {}
            },
            1 => {
                if matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                ) {
                    self.global_config.agy_flags.sandbox = !self.global_config.agy_flags.sandbox;
                    self.persist_global_config();
                }
            }
            2 => {
                if matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                ) {
                    self.global_config.agy_flags.continue_session =
                        !self.global_config.agy_flags.continue_session;
                    self.persist_global_config();
                }
            }
            3 => {
                if matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                ) {
                    self.global_config.agy_flags.dangerously_skip_permissions =
                        !self.global_config.agy_flags.dangerously_skip_permissions;
                    self.persist_global_config();
                }
            }
            4 => match key.code {
                KeyCode::Backspace => {
                    self.settings_page.add_dirs_input.pop();
                    self.global_config
                        .agy_flags
                        .set_add_dirs_from_str(&self.settings_page.add_dirs_input);
                    self.persist_global_config();
                }
                KeyCode::Enter => {
                    self.global_config
                        .agy_flags
                        .set_add_dirs_from_str(&self.settings_page.add_dirs_input);
                    self.persist_global_config();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.settings_page.add_dirs_input.push(c);
                    self.global_config
                        .agy_flags
                        .set_add_dirs_from_str(&self.settings_page.add_dirs_input);
                    self.persist_global_config();
                }
                _ => {}
            },
            5 => match key.code {
                KeyCode::Left => {
                    self.global_config.shell_formatting_policy =
                        self.global_config.shell_formatting_policy.prev();
                    self.persist_global_config();
                }
                KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ') => {
                    self.global_config.shell_formatting_policy =
                        self.global_config.shell_formatting_policy.next();
                    self.persist_global_config();
                }
                _ => {}
            },
            6 => {
                if matches!(
                    key.code,
                    KeyCode::Left | KeyCode::Right | KeyCode::Enter | KeyCode::Char(' ')
                ) {
                    self.global_config.workspace_storage_local =
                        !self.global_config.workspace_storage_local;
                    self.force_local_state = self.global_config.workspace_storage_local;
                    self.persist_global_config();
                    self.persist_workspace_state();
                }
            }
            7 => match key.code {
                KeyCode::Backspace => {
                    self.settings_page.default_plan_path_input.pop();
                    let trimmed = self.settings_page.default_plan_path_input.trim();
                    self.workspace_state.default_plan_path = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                    self.persist_workspace_state();
                }
                KeyCode::Enter => {
                    self.persist_workspace_state();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.settings_page.default_plan_path_input.push(c);
                    let trimmed = self.settings_page.default_plan_path_input.trim();
                    self.workspace_state.default_plan_path = if trimmed.is_empty() {
                        None
                    } else {
                        Some(trimmed.to_string())
                    };
                    self.persist_workspace_state();
                }
                _ => {}
            },
            8 => match key.code {
                KeyCode::Backspace => {
                    self.settings_page.pinned_constraints_input.pop();
                    self.apply_settings_pinned_constraints_input();
                }
                KeyCode::Enter => {
                    self.apply_settings_pinned_constraints_input();
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.settings_page.pinned_constraints_input.push(c);
                    self.apply_settings_pinned_constraints_input();
                }
                _ => {}
            },
            _ => {}
        }
        Ok(())
    }

    fn apply_settings_pinned_constraints_input(&mut self) {
        self.workspace_state.pinned_constraints = self
            .settings_page
            .pinned_constraints_input
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();
        self.persist_workspace_state();
        self.refresh_editor_themes();
    }

    fn save_current_settings_tab(&mut self) -> Result<()> {
        match self.settings_page.active_tab {
            SettingsTab::ThemeAndContrast => {
                self.apply_settings_theme_hex_overrides()?;
            }
            SettingsTab::ModelsAndToml => {
                if !self.settings_page.model_ingest_path.trim().is_empty() {
                    let p = PathBuf::from(self.settings_page.model_ingest_path.trim());
                    let _ = self.ingest_model_file_from_settings(&p)?;
                } else {
                    self.submit_custom_model_from_settings()?;
                }
            }
            SettingsTab::TemplatesAndDefaults => {
                self.persist_global_config();
                let msg = "Saved template & startup defaults to config.toml".to_string();
                self.settings_page.status_note = msg.clone();
                self.status_message = msg;
            }
            SettingsTab::ExecutionAndWorkspace => {
                self.global_config
                    .agy_flags
                    .set_add_dirs_from_str(&self.settings_page.add_dirs_input);
                self.persist_global_config();
                self.persist_workspace_state();
                let msg = "Saved execution flags, shell policy & workspace settings".to_string();
                self.settings_page.status_note = msg.clone();
                self.status_message = msg;
            }
        }
        Ok(())
    }

    /// Handles a terminal [`KeyEvent`], supporting Micro-style global bindings, modal navigation,
    /// full-screen Settings Page (`Alt+S`), shell quote confirmation modal, and multiline `tui-textarea` editing.
    pub fn handle_key_event(&mut self, key: KeyEvent) -> Result<()> {
        // 0a. If the Shell Formatting & Quote Confirmation Modal is open, route keys to it first
        if self.shell_quote_modal.is_open {
            match key.code {
                KeyCode::Enter => {
                    if let Some(action) = self.shell_quote_modal.pending_action.take() {
                        self.shell_quote_modal.close();
                        self.confirm_dispatch(action)?;
                    }
                    return Ok(());
                }
                KeyCode::Char('e') | KeyCode::Char('E')
                    if key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Some(action) = self.shell_quote_modal.pending_action.take() {
                        self.shell_quote_modal.close();
                        self.confirm_dispatch(action)?;
                    }
                    return Ok(());
                }
                KeyCode::Char('a') | KeyCode::Char('A')
                    if key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.global_config.shell_formatting_policy =
                        ShellFormattingPolicy::AutoEscapeSilently;
                    self.persist_global_config();
                    if let Some(action) = self.shell_quote_modal.pending_action.take() {
                        self.shell_quote_modal.close();
                        self.confirm_dispatch(action)?;
                    }
                    return Ok(());
                }
                KeyCode::Esc => {
                    self.shell_quote_modal.close();
                    self.focus = FocusPane::FieldEditor(0);
                    self.refresh_editor_themes();
                    self.status_message =
                        "Cancelled dispatch to edit or add quotes manually in prompt fields"
                            .to_string();
                    return Ok(());
                }
                KeyCode::Char('r') | KeyCode::Char('R')
                    if key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.shell_quote_modal.close();
                    self.focus = FocusPane::FieldEditor(0);
                    self.refresh_editor_themes();
                    self.status_message =
                        "Returned to prompt editor to edit / add quotes manually".to_string();
                    return Ok(());
                }
                _ => {}
            }
            return Ok(());
        }

        // 0b. If the Model Catalog Sync & Add Modal (`Alt+U`) is open, route keys to it
        if self.model_modal.is_open {
            match key.code {
                KeyCode::Esc => {
                    self.model_modal.close();
                    self.refresh_editor_themes();
                    self.status_message = "Closed Model Catalog modal".to_string();
                }
                KeyCode::Char('r') | KeyCode::Char('R')
                    if key.modifiers.contains(KeyModifiers::CONTROL)
                        || key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    if let Err(err) = self.sync_models_from_agy() {
                        let msg = format!("agy models sync error: {}", err);
                        self.model_modal.status_note = msg.clone();
                        self.status_message = msg;
                    }
                }
                KeyCode::Tab | KeyCode::Down => {
                    self.model_modal.focused_field = self.model_modal.focused_field.next();
                }
                KeyCode::BackTab | KeyCode::Up => {
                    self.model_modal.focused_field = self.model_modal.focused_field.prev();
                }
                KeyCode::Left => match self.model_modal.focused_field {
                    ModelModalField::Family => {
                        self.model_modal.family = self.model_modal.family.prev();
                    }
                    ModelModalField::DefaultEffort => {
                        self.model_modal.default_effort = self.model_modal.default_effort.prev();
                    }
                    _ => {}
                },
                KeyCode::Right => match self.model_modal.focused_field {
                    ModelModalField::Family => {
                        self.model_modal.family = self.model_modal.family.next();
                    }
                    ModelModalField::DefaultEffort => {
                        self.model_modal.default_effort = self.model_modal.default_effort.next();
                    }
                    _ => {}
                },
                KeyCode::Backspace => match self.model_modal.focused_field {
                    ModelModalField::ShortName => {
                        self.model_modal.short_name.pop();
                    }
                    ModelModalField::DisplayName => {
                        self.model_modal.display_name.pop();
                    }
                    ModelModalField::AgyBaseId => {
                        self.model_modal.agy_base_id.pop();
                    }
                    _ => {}
                },
                KeyCode::Enter => {
                    self.submit_custom_model_from_modal()?;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    match self.model_modal.focused_field {
                        ModelModalField::ShortName => self.model_modal.short_name.push(c),
                        ModelModalField::DisplayName => self.model_modal.display_name.push(c),
                        ModelModalField::AgyBaseId => self.model_modal.agy_base_id.push(c),
                        ModelModalField::Family => {
                            if c == ' ' {
                                self.model_modal.family = self.model_modal.family.next();
                            }
                        }
                        ModelModalField::DefaultEffort => {
                            if c == ' ' {
                                self.model_modal.default_effort =
                                    self.model_modal.default_effort.next();
                            }
                        }
                    }
                }
                _ => {}
            }
            return Ok(());
        }

        // 1. If the CWD Fuzzy File Picker Modal is open, route keys to the modal first
        if self.file_picker.is_open {
            match key.code {
                KeyCode::Esc => {
                    self.file_picker.close();
                    self.refresh_editor_themes();
                    self.status_message = "Closed file picker".to_string();
                }
                KeyCode::Enter => {
                    self.confirm_file_picker_selection();
                }
                KeyCode::Up => {
                    if self.file_picker.selected_index > 0 {
                        self.file_picker.selected_index -= 1;
                    }
                }
                KeyCode::Down => {
                    let count = self.file_picker.matches(&self.file_index, 50).len();
                    if count > 0 && self.file_picker.selected_index + 1 < count {
                        self.file_picker.selected_index += 1;
                    }
                }
                KeyCode::Backspace => {
                    self.file_picker.query.pop();
                    self.file_picker.selected_index = 0;
                }
                KeyCode::Char('f') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                    self.file_picker.markdown_only = !self.file_picker.markdown_only;
                    self.file_picker.selected_index = 0;
                }
                KeyCode::Char(c)
                    if !key.modifiers.contains(KeyModifiers::CONTROL)
                        && !key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.file_picker.query.push(c);
                    self.file_picker.selected_index = 0;
                }
                _ => {}
            }
            return Ok(());
        }

        // 1b. If the full-screen Settings Page (`Alt+S`) is open, route keys to `handle_settings_key_event`
        if self.settings_page.is_open {
            return self.handle_settings_key_event(key);
        }

        // 1c. If a pending `.md` plan file picker approval is staged (`Update <file>.md plan`),
        // handle explicit approval (`Enter` / `Alt+Y`) or rejection (`Esc` / `Alt+N`).
        if self.pending_plan_picker_approval.is_some() {
            match key.code {
                KeyCode::Enter => {
                    self.approve_pending_plan_picker();
                    return Ok(());
                }
                KeyCode::Char('y') | KeyCode::Char('Y')
                    if key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.approve_pending_plan_picker();
                    return Ok(());
                }
                KeyCode::Esc => {
                    self.decline_pending_plan_picker();
                    return Ok(());
                }
                KeyCode::Char('n') | KeyCode::Char('N')
                    if key.modifiers.contains(KeyModifiers::ALT) =>
                {
                    self.decline_pending_plan_picker();
                    return Ok(());
                }
                _ => {
                    self.pending_plan_picker_approval = None;
                }
            }
        }

        // 2. Global Micro-style keybindings (`Ctrl+Q`, `Ctrl+T`, `Ctrl+P`, `Ctrl+S`, `Ctrl+K`, `Ctrl+H`, `Ctrl+F`, `Alt+S`, `Alt+M`, `Alt+E`, `Alt+C`, `Alt+U`)
        if key.modifiers.contains(KeyModifiers::CONTROL) {
            match key.code {
                KeyCode::Char('q') | KeyCode::Char('Q') => {
                    self.should_quit = true;
                    return Ok(());
                }
                KeyCode::Char('t') | KeyCode::Char('T') => {
                    self.cycle_theme();
                    return Ok(());
                }
                KeyCode::Char('p') | KeyCode::Char('P') => {
                    self.show_preview = !self.show_preview;
                    self.status_message = format!(
                        "Live preview {}",
                        if self.show_preview {
                            "expanded"
                        } else {
                            "collapsed"
                        }
                    );
                    return Ok(());
                }
                KeyCode::Char('o') | KeyCode::Char('O') => {
                    self.toggle_terminal_output();
                    return Ok(());
                }
                KeyCode::Char('l') | KeyCode::Char('L') => {
                    if self.focus == FocusPane::TerminalOutput || self.terminal_output.is_open {
                        self.terminal_output.clear();
                        self.status_message = "Cleared Terminal Execution Output Pane".to_string();
                        return Ok(());
                    }
                }
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    let _ = self.save_rendered_prompt_to_md(None)?;
                    return Ok(());
                }
                KeyCode::Char('k') | KeyCode::Char('K') => {
                    self.toggle_pin_current_constraint();
                    return Ok(());
                }
                KeyCode::Char('h') | KeyCode::Char('H') => {
                    self.cycle_prompt_history();
                    return Ok(());
                }
                KeyCode::Char('f') | KeyCode::Char('F') => {
                    self.open_file_picker_for_active_field(false, None);
                    return Ok(());
                }
                KeyCode::Enter => {
                    let action = self.selected_action;
                    self.request_dispatch(action)?;
                    return Ok(());
                }
                _ => {}
            }
        }

        if key.modifiers.contains(KeyModifiers::ALT) {
            match key.code {
                KeyCode::Char('s') | KeyCode::Char('S') => {
                    self.open_settings_page();
                    return Ok(());
                }
                KeyCode::Char('m') | KeyCode::Char('M') => {
                    self.cycle_sub_model(true);
                    return Ok(());
                }
                KeyCode::Char('e') | KeyCode::Char('E') => {
                    self.cycle_effort(true);
                    return Ok(());
                }
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    self.cycle_focused_field_choice();
                    return Ok(());
                }
                KeyCode::Char('u') | KeyCode::Char('U') => {
                    self.open_model_manager_modal();
                    return Ok(());
                }
                KeyCode::Char('1') => {
                    self.request_dispatch(DispatchAction::AgyInteractive)?;
                    return Ok(());
                }
                KeyCode::Char('2') => {
                    self.request_dispatch(DispatchAction::AgyPrint)?;
                    return Ok(());
                }
                KeyCode::Char('3') => {
                    self.request_dispatch(DispatchAction::StdoutClipboard)?;
                    return Ok(());
                }
                KeyCode::Char('4') => {
                    self.request_dispatch(DispatchAction::SaveMarkdown)?;
                    return Ok(());
                }
                _ => {}
            }
        }

        // 3. Tab / Shift+Tab navigation across panes
        match key.code {
            KeyCode::Tab => {
                self.sync_values_from_editors();
                self.focus_next();
                return Ok(());
            }
            KeyCode::BackTab => {
                self.sync_values_from_editors();
                self.focus_prev();
                return Ok(());
            }
            KeyCode::Esc => {
                self.focus = FocusPane::ActionBar;
                self.refresh_editor_themes();
                self.status_message =
                    "Action Bar focused: Left/Right or 1-4 to select action, Enter to execute, Alt+S Settings, Ctrl+Q Quit"
                        .to_string();
                return Ok(());
            }
            _ => {}
        }

        // 4. Route remaining keys to the focused pane
        match self.focus {
            FocusPane::ControlBar(item) => match key.code {
                KeyCode::Left => {
                    self.focus = FocusPane::ControlBar(item.prev());
                }
                KeyCode::Right => {
                    self.focus = FocusPane::ControlBar(item.next());
                }
                KeyCode::Up | KeyCode::Down | KeyCode::Enter | KeyCode::Char(' ') => {
                    let forward = !matches!(key.code, KeyCode::Up);
                    match item {
                        ControlItem::ModelFamily => self.cycle_model_family(forward),
                        ControlItem::SubModel => self.cycle_sub_model(forward),
                        ControlItem::Effort => self.cycle_effort(forward),
                        ControlItem::SlashCommand => self.cycle_slash_command(forward),
                        ControlItem::Settings => self.open_settings_page(),
                    }
                }
                _ => {}
            },
            FocusPane::FieldEditor(idx) => {
                if key.code == KeyCode::Char('@')
                    && !key.modifiers.contains(KeyModifiers::CONTROL)
                    && !key.modifiers.contains(KeyModifiers::ALT)
                {
                    let markdown_only = self
                        .editors
                        .get(idx)
                        .map(|e| e.spec.id == "output")
                        .unwrap_or(false);
                    self.open_file_picker_for_active_field(markdown_only, None);
                    return Ok(());
                }

                if let Some(editor) = self.editors.get_mut(idx) {
                    editor.textarea.input(key);
                }
                self.sync_values_from_editors();
            }
            FocusPane::ActionBar => match key.code {
                KeyCode::Left | KeyCode::Up => {
                    self.selected_action = self.selected_action.prev();
                }
                KeyCode::Right | KeyCode::Down => {
                    self.selected_action = self.selected_action.next();
                }
                KeyCode::Char('1') => {
                    self.request_dispatch(DispatchAction::AgyInteractive)?;
                }
                KeyCode::Char('2') => {
                    self.request_dispatch(DispatchAction::AgyPrint)?;
                }
                KeyCode::Char('3') => {
                    self.request_dispatch(DispatchAction::StdoutClipboard)?;
                }
                KeyCode::Char('4') => {
                    self.request_dispatch(DispatchAction::SaveMarkdown)?;
                }
                KeyCode::Enter => {
                    let action = self.selected_action;
                    self.request_dispatch(action)?;
                }
                _ => {}
            },
            FocusPane::TerminalOutput => match key.code {
                KeyCode::Up | KeyCode::Char('k') => {
                    self.terminal_output.scroll_up(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    self.terminal_output.scroll_down(1);
                }
                KeyCode::PageUp => {
                    self.terminal_output.scroll_up(8);
                }
                KeyCode::PageDown => {
                    self.terminal_output.scroll_down(8);
                }
                KeyCode::Home => {
                    self.terminal_output.scroll_offset = 0;
                }
                KeyCode::End => {
                    self.terminal_output.scroll_to_bottom();
                }
                KeyCode::Char('c') | KeyCode::Char('C') => {
                    self.terminal_output.clear();
                    self.status_message = "Cleared Terminal Execution Output Pane".to_string();
                }
                KeyCode::Char('1') => {
                    self.request_dispatch(DispatchAction::AgyInteractive)?;
                }
                KeyCode::Char('2') => {
                    self.request_dispatch(DispatchAction::AgyPrint)?;
                }
                KeyCode::Char('3') => {
                    self.request_dispatch(DispatchAction::StdoutClipboard)?;
                }
                KeyCode::Char('4') => {
                    self.request_dispatch(DispatchAction::SaveMarkdown)?;
                }
                KeyCode::Enter => {
                    let action = self.selected_action;
                    self.request_dispatch(action)?;
                }
                _ => {}
            },
        }

        Ok(())
    }
}

/// Attempts to copy `content` to the system clipboard (`wl-copy`, `xclip`, or `xsel`) when available
/// and not disabled by `VERBOT_NO_CLIPBOARD=1`.
fn try_copy_to_system_clipboard(content: &str) -> bool {
    if std::env::var("VERBOT_NO_CLIPBOARD")
        .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
        .unwrap_or(false)
    {
        return false;
    }

    let candidates: [(&str, &[&str]); 3] = [
        ("wl-copy", &[]),
        ("xclip", &["-selection", "clipboard"]),
        ("xsel", &["--clipboard", "--input"]),
    ];

    for (bin, args) in candidates {
        if let Ok(mut child) = Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        {
            if let Some(mut stdin) = child.stdin.take() {
                use std::io::Write;
                let _ = stdin.write_all(content.as_bytes());
            }
            if let Ok(status) = child.wait() {
                if status.success() {
                    return true;
                }
            }
        }
    }
    false
}
