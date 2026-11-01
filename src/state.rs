use crate::formatting::{AgyCliFlags, ShellFormattingPolicy};
use crate::models::{EffortLevel, ModelFamily};
use crate::template::PromptValues;
use crate::theme::{default_verbot_config_dir, default_verbot_theme_path, Theme, ThemePreset};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};

/// Maximum number of recent prompts retained in the per-directory ring buffer.
pub const MAX_PROMPT_HISTORY: usize = 25;

/// Global user configuration stored at `~/.config/verbot/config.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GlobalConfig {
    /// Preferred theme preset (`"NovOS Cosmic Dark"`, `"COSMIC System Auto"`, `"NovOS Solar Light"`, `"Terminal ANSI"`).
    #[serde(default = "default_theme_preset_name")]
    pub default_theme_preset: String,
    /// Preferred default model family (`"Gemini"`, `"Claude"`, `"GPT"`).
    #[serde(default = "default_model_family_name")]
    pub default_model_family: String,
    /// Preferred default sub-model short name or ID (`"Argon"`, `"Flash"`, `"Opus"`, `"Sonnet"`, `"GPT-OSS 120B"`).
    #[serde(default = "default_sub_model_name")]
    pub default_sub_model: String,
    /// Preferred default reasoning effort level (`"low"`, `"medium"`, `"high"`, `"xhigh"`, `"max"`).
    #[serde(default = "default_effort_name")]
    pub default_effort: String,
    /// Preferred default slash command on startup (e.g. `Some("plan")`).
    #[serde(default = "default_slash_command")]
    pub default_slash_command: Option<String>,
    /// Whether the Live Prompt Preview pane starts expanded (`true` by default).
    #[serde(default = "default_true")]
    pub show_preview_on_startup: bool,
    /// Preferred default template name or filename in `~/.config/verbot/templates/`.
    #[serde(default)]
    pub default_template: Option<String>,
    /// Default `agy` CLI flags (`--mode`, `--sandbox`, `--continue`, `--dangerously-skip-permissions`, `--add-dir`).
    #[serde(default)]
    pub agy_flags: AgyCliFlags,
    /// Shell text formatting & escaping policy (`PromptOnSpecialChars`, `AutoEscapeSilently`, `RawLiteral`).
    #[serde(default)]
    pub shell_formatting_policy: ShellFormattingPolicy,
    /// Whether workspace state defaults to project-local `./.verbot/state.toml` (`false` = XDG Data Cache).
    #[serde(default)]
    pub workspace_storage_local: bool,
}

fn default_true() -> bool {
    true
}

fn default_theme_preset_name() -> String {
    ThemePreset::NovosCosmicDark.as_str().to_string()
}

fn default_model_family_name() -> String {
    ModelFamily::Gemini.as_str().to_string()
}

fn default_sub_model_name() -> String {
    "Argon".to_string()
}

fn default_effort_name() -> String {
    EffortLevel::Medium.as_str().to_string()
}

impl Default for GlobalConfig {
    fn default() -> Self {
        Self {
            default_theme_preset: default_theme_preset_name(),
            default_model_family: default_model_family_name(),
            default_sub_model: default_sub_model_name(),
            default_effort: default_effort_name(),
            default_slash_command: default_slash_command(),
            show_preview_on_startup: true,
            default_template: None,
            agy_flags: AgyCliFlags::default(),
            shell_formatting_policy: ShellFormattingPolicy::default(),
            workspace_storage_local: false,
        }
    }
}

impl GlobalConfig {
    /// Returns `~/.config/verbot/config.toml`.
    pub fn default_path() -> Option<PathBuf> {
        default_verbot_config_dir().map(|d| d.join("config.toml"))
    }

    /// Returns `~/.config/verbot/templates/`.
    pub fn default_templates_dir() -> Option<PathBuf> {
        default_verbot_config_dir().map(|d| d.join("templates"))
    }

    /// Loads `GlobalConfig` from `custom_path` or `~/.config/verbot/config.toml`, falling back to defaults.
    pub fn load_or_default(custom_path: Option<&Path>) -> Self {
        let path = custom_path
            .map(PathBuf::from)
            .or_else(Self::default_path);
        if let Some(ref p) = path {
            if p.is_file() {
                if let Ok(cfg) = Self::load_from_file(p) {
                    return cfg;
                }
            }
        }
        Self::default()
    }

    pub fn load_from_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed to read global config at {}", path.display()))?;
        toml::from_str(&raw).context("Failed to parse global config.toml")
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!("Failed to create config directory {}", parent.display())
            })?;
        }
        let serialized =
            toml::to_string_pretty(self).context("Failed to serialize GlobalConfig to TOML")?;
        fs::write(path, serialized)
            .with_context(|| format!("Failed to write global config to {}", path.display()))
    }
}

/// Entry in the per-directory recent prompt history ring buffer.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptHistoryEntry {
    pub timestamp_utc: String,
    pub model_id: String,
    pub effort: String,
    pub values: PromptValues,
    pub rendered_prompt: String,
}

/// Per-working-directory state.
///
/// Stored either in `<cwd>/.verbot/state.toml` ONLY if `<cwd>/.verbot/` already exists or the user
/// explicitly requests local project storage (`--local` / explicit local pin save), otherwise stored
/// cleanly in `~/.local/share/verbot/workspaces/<cwd-slug-hash>/state.toml` so `verbot` never
/// pollutes the user's working directory uninvited.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceState {
    /// Canonical working directory path this state belongs to.
    pub workspace_dir: String,
    /// Pinned project-level constraints (e.g., `["No QEMU", "Drop .img"]`) automatically pre-populated or toggled via `Ctrl+K`.
    #[serde(default)]
    pub pinned_constraints: Vec<String>,
    /// Default project plan `.md` file path (e.g., `"STEPPED_IMPLEMENTATION_PLAN.md"`).
    #[serde(default)]
    pub default_plan_path: Option<String>,
    /// Active theme preset name (`"NovOS Cosmic Dark"`, `"COSMIC System Auto"`, `"NovOS Solar Light"`, `"Terminal ANSI"`).
    #[serde(default = "default_theme_preset_name")]
    pub active_theme_preset: String,
    /// Last selected model family (`"Gemini"`, `"Claude"`, `"GPT"`).
    #[serde(default = "default_model_family_name")]
    pub selected_model_family: String,
    /// Last selected sub-model short name (`"Argon"`, `"Flash"`, `"Opus"`, `"Sonnet"`, `"GPT-OSS 120B"`).
    #[serde(default = "default_sub_model_name")]
    pub selected_sub_model: String,
    /// Last selected thinking / effort level (`"low"`, `"medium"`, `"high"`, `"xhigh"`, `"max"`).
    #[serde(default = "default_effort_name")]
    pub selected_effort: String,
    /// Last selected slash command (`Some("plan")`, `Some("boost")`, `Some("goal")`, or `None`).
    #[serde(default = "default_slash_command")]
    pub selected_slash_command: Option<String>,
    /// Ring buffer of recent prompts executed in this directory (capped at [`MAX_PROMPT_HISTORY`]).
    #[serde(default)]
    pub prompt_history: VecDeque<PromptHistoryEntry>,
}

fn default_slash_command() -> Option<String> {
    Some("plan".to_string())
}

impl Default for WorkspaceState {
    fn default() -> Self {
        Self::new_for_dir(Path::new("."))
    }
}

impl WorkspaceState {
    pub fn new_for_dir(cwd: &Path) -> Self {
        let canonical = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
        Self {
            workspace_dir: canonical.to_string_lossy().to_string(),
            pinned_constraints: Vec::new(),
            default_plan_path: None,
            active_theme_preset: default_theme_preset_name(),
            selected_model_family: default_model_family_name(),
            selected_sub_model: default_sub_model_name(),
            selected_effort: default_effort_name(),
            selected_slash_command: default_slash_command(),
            prompt_history: VecDeque::new(),
        }
    }

    /// Adds or removes a pinned constraint for the current workspace.
    /// Returns `true` if the constraint is now pinned, or `false` if it was unpinned.
    pub fn toggle_pinned_constraint(&mut self, constraint: &str) -> bool {
        let trimmed = constraint.trim();
        if trimmed.is_empty() {
            return false;
        }
        if let Some(idx) = self
            .pinned_constraints
            .iter()
            .position(|c| c.eq_ignore_ascii_case(trimmed))
        {
            self.pinned_constraints.remove(idx);
            false
        } else {
            self.pinned_constraints.push(trimmed.to_string());
            true
        }
    }

    /// Formats all pinned constraints as a single semicolon-separated string suitable for the `Constraints` field.
    pub fn formatted_pinned_constraints(&self) -> String {
        self.pinned_constraints.join("; ")
    }

    /// Merges pinned constraints into a user's `Constraints` string without duplicating existing items.
    pub fn apply_pinned_constraints(&self, current_constraints: &str) -> String {
        if self.pinned_constraints.is_empty() {
            return current_constraints.trim().to_string();
        }
        let mut combined: Vec<String> = Vec::new();
        for pinned in &self.pinned_constraints {
            let p = pinned.trim();
            if !p.is_empty() {
                combined.push(p.to_string());
            }
        }
        for part in current_constraints.split(';') {
            let trimmed = part.trim();
            if !trimmed.is_empty()
                && !combined
                    .iter()
                    .any(|existing| existing.eq_ignore_ascii_case(trimmed))
            {
                combined.push(trimmed.to_string());
            }
        }
        combined.join("; ")
    }

    /// Pushes a prompt snapshot onto the ring buffer, maintaining at most [`MAX_PROMPT_HISTORY`] entries.
    pub fn record_prompt(
        &mut self,
        model_id: impl Into<String>,
        effort: EffortLevel,
        values: PromptValues,
        rendered_prompt: impl Into<String>,
    ) {
        let entry = PromptHistoryEntry {
            timestamp_utc: current_iso_timestamp(),
            model_id: model_id.into(),
            effort: effort.as_str().to_string(),
            values,
            rendered_prompt: rendered_prompt.into(),
        };
        if self.prompt_history.len() >= MAX_PROMPT_HISTORY {
            self.prompt_history.pop_front();
        }
        self.prompt_history.push_back(entry);
    }

    /// Resolves the active [`Theme`] from this workspace state (or user's `theme.toml` if `Custom`).
    pub fn resolve_theme(&self, custom_theme_toml: Option<&Path>) -> Theme {
        let preset = ThemePreset::from_name(&self.active_theme_preset);
        match preset {
            ThemePreset::Custom => Theme::load_user_or_default(custom_theme_toml),
            other => Theme::from_preset(other),
        }
    }

    /// Updates the active theme preset and optionally persists the theme to `~/.config/verbot/theme.toml`.
    pub fn set_theme_preset(
        &mut self,
        preset: ThemePreset,
        persist_theme_toml: Option<&Path>,
    ) -> Result<Theme> {
        self.active_theme_preset = preset.as_str().to_string();
        let theme = Theme::from_preset(preset);
        if let Some(theme_path) = persist_theme_toml {
            save_theme_config(&theme, Some(theme_path))?;
        }
        Ok(theme)
    }

    /// Loads workspace state for `cwd`.
    ///
    /// Resolution order:
    /// 1. If `<cwd>/.verbot/state.toml` exists on disk, load from it.
    /// 2. Otherwise, load from `<xdg_data_dir>/verbot/workspaces/<cwd_key>/state.toml`.
    /// 3. If neither exists, initialize a fresh `WorkspaceState` seeded from `GlobalConfig`.
    pub fn load_for_dir(
        cwd: &Path,
        custom_xdg_data_dir: Option<&Path>,
        global_cfg: Option<&GlobalConfig>,
    ) -> Result<(Self, StateStorageLocation)> {
        let local_state_path = local_verbot_state_path(cwd);
        if local_state_path.is_file() {
            let state = Self::load_from_file(&local_state_path)?;
            return Ok((state, StateStorageLocation::ProjectLocal(local_state_path)));
        }

        let cache_state_path = xdg_workspace_state_path(cwd, custom_xdg_data_dir)?;
        if cache_state_path.is_file() {
            let state = Self::load_from_file(&cache_state_path)?;
            return Ok((state, StateStorageLocation::XdgCache(cache_state_path)));
        }

        let mut fresh = Self::new_for_dir(cwd);
        let mut prefer_local = false;
        if let Some(g) = global_cfg {
            fresh.active_theme_preset = g.default_theme_preset.clone();
            fresh.selected_model_family = g.default_model_family.clone();
            fresh.selected_sub_model = g.default_sub_model.clone();
            fresh.selected_effort = g.default_effort.clone();
            fresh.selected_slash_command = g.default_slash_command.clone();
            prefer_local = g.workspace_storage_local;
        }

        // If `<cwd>/.verbot` directory already exists (e.g. created by `verbot ingest --local`)
        // or global config prefers project-local storage, use ProjectLocal; otherwise default to XdgCache.
        let location = if prefer_local || local_verbot_dir(cwd).is_dir() {
            StateStorageLocation::ProjectLocal(local_state_path)
        } else {
            StateStorageLocation::XdgCache(cache_state_path)
        };
        Ok((fresh, location))
    }

    /// Saves this `WorkspaceState` to its resolved storage path.
    ///
    /// Respects the non-pollution constraint: only writes to `<cwd>/.verbot/state.toml` if `force_local` is true
    /// or `<cwd>/.verbot/` already exists; otherwise writes to `~/.local/share/verbot/workspaces/<cwd-hash>/state.toml`.
    pub fn save_for_dir(
        &self,
        cwd: &Path,
        force_local: bool,
        custom_xdg_data_dir: Option<&Path>,
    ) -> Result<StateStorageLocation> {
        let target = resolve_save_location(cwd, force_local, custom_xdg_data_dir)?;
        self.save_to_file(target.path())?;
        Ok(target)
    }

    pub fn load_from_file(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed to read workspace state at {}", path.display()))?;
        toml::from_str(&raw)
            .with_context(|| format!("Failed to parse workspace state TOML at {}", path.display()))
    }

    pub fn save_to_file(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).with_context(|| {
                format!(
                    "Failed to create workspace state directory {}",
                    parent.display()
                )
            })?;
        }
        let serialized =
            toml::to_string_pretty(self).context("Failed to serialize WorkspaceState to TOML")?;
        fs::write(path, serialized)
            .with_context(|| format!("Failed to write workspace state to {}", path.display()))
    }
}

/// Indicates where a workspace's state is persisted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StateStorageLocation {
    /// Stored inside `<cwd>/.verbot/state.toml` (only when `.verbot/` exists or `--local` / explicit local save was requested).
    ProjectLocal(PathBuf),
    /// Stored in XDG data directory (`~/.local/share/verbot/workspaces/<slug-hash>/state.toml`).
    XdgCache(PathBuf),
}

impl StateStorageLocation {
    pub fn path(&self) -> &Path {
        match self {
            Self::ProjectLocal(p) | Self::XdgCache(p) => p.as_path(),
        }
    }

    pub fn is_project_local(&self) -> bool {
        matches!(self, Self::ProjectLocal(_))
    }
}

/// Returns `<cwd>/.verbot`.
pub fn local_verbot_dir(cwd: &Path) -> PathBuf {
    cwd.join(".verbot")
}

/// Returns `<cwd>/.verbot/state.toml`.
pub fn local_verbot_state_path(cwd: &Path) -> PathBuf {
    local_verbot_dir(cwd).join("state.toml")
}

/// Returns `<cwd>/.verbot/templates`.
pub fn local_verbot_templates_dir(cwd: &Path) -> PathBuf {
    local_verbot_dir(cwd).join("templates")
}

/// Returns the base XDG data directory for `verbot` (`~/.local/share/verbot`).
pub fn default_verbot_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir()
        .or_else(dirs::data_dir)
        .map(|d| d.join("verbot"))
}

/// Computes a deterministic, human-inspectable directory key `<basename>-<fnv1a64_hex>` for a CWD path.
pub fn workspace_cache_key(cwd: &Path) -> String {
    let canonical = fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let path_str = canonical.to_string_lossy();
    let hash = fnv1a_64(path_str.as_bytes());
    let raw_name = canonical
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("root");
    let clean_slug: String = raw_name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    let slug = if clean_slug.is_empty() {
        "workspace"
    } else {
        clean_slug.as_str()
    };
    format!("{}-{:016x}", slug, hash)
}

/// Resolves the XDG workspace state file path (`~/.local/share/verbot/workspaces/<key>/state.toml`).
pub fn xdg_workspace_state_path(
    cwd: &Path,
    custom_xdg_data_dir: Option<&Path>,
) -> Result<PathBuf> {
    let base = custom_xdg_data_dir
        .map(PathBuf::from)
        .or_else(default_verbot_data_dir)
        .unwrap_or_else(|| std::env::temp_dir().join("verbot-data"));
    let key = workspace_cache_key(cwd);
    Ok(base.join("workspaces").join(key).join("state.toml"))
}

/// Determines whether state should be saved to `.verbot/state.toml` or XDG data cache.
pub fn resolve_save_location(
    cwd: &Path,
    force_local: bool,
    custom_xdg_data_dir: Option<&Path>,
) -> Result<StateStorageLocation> {
    let local_dir = local_verbot_dir(cwd);
    let local_state = local_verbot_state_path(cwd);
    if force_local || local_dir.is_dir() || local_state.is_file() {
        Ok(StateStorageLocation::ProjectLocal(local_state))
    } else {
        let xdg_path = xdg_workspace_state_path(cwd, custom_xdg_data_dir)?;
        Ok(StateStorageLocation::XdgCache(xdg_path))
    }
}

/// Saves a [`Theme`] to `~/.config/verbot/theme.toml` (or `custom_path`).
pub fn save_theme_config(theme: &Theme, custom_path: Option<&Path>) -> Result<PathBuf> {
    let path = custom_path
        .map(PathBuf::from)
        .or_else(default_verbot_theme_path)
        .unwrap_or_else(|| std::env::temp_dir().join("verbot-theme.toml"));
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).with_context(|| {
            format!("Failed to create theme config directory {}", parent.display())
        })?;
    }
    let cfg = theme.to_config();
    let raw = toml::to_string_pretty(&cfg).context("Failed to serialize ThemeConfig to TOML")?;
    fs::write(&path, raw)
        .with_context(|| format!("Failed to write theme config to {}", path.display()))?;
    Ok(path)
}

fn fnv1a_64(bytes: &[u8]) -> u64 {
    const FNV_OFFSET_BASIS: u64 = 0xcbf29ce484222325;
    const FNV_PRIME: u64 = 0x00000100000001b3;
    let mut hash = FNV_OFFSET_BASIS;
    for &b in bytes {
        hash ^= u64::from(b);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    hash
}

fn current_iso_timestamp() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{}", secs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_xdg_default_does_not_pollute_cwd_with_local_verbot_dir() {
        let tmp_root = std::env::temp_dir().join(format!(
            "verbot_state_no_pollute_{}",
            std::process::id()
        ));
        let fake_cwd = tmp_root.join("sample_project");
        let fake_xdg = tmp_root.join("xdg_data");
        fs::create_dir_all(&fake_cwd).unwrap();

        let (mut state, loc) =
            WorkspaceState::load_for_dir(&fake_cwd, Some(&fake_xdg), None).unwrap();
        assert!(!loc.is_project_local());
        assert!(!fake_cwd.join(".verbot").exists());

        // Pin a constraint and save without `--local`
        assert!(state.toggle_pinned_constraint("No QEMU"));
        state.default_plan_path = Some("STEPPED_IMPLEMENTATION_PLAN.md".to_string());
        state.selected_sub_model = "Argon".to_string();
        state.selected_effort = "high".to_string();

        let saved_loc = state
            .save_for_dir(&fake_cwd, false, Some(&fake_xdg))
            .unwrap();
        assert!(!saved_loc.is_project_local());
        assert!(
            !fake_cwd.join(".verbot").exists(),
            "Saving default workspace state must not create .verbot/ in user CWD"
        );
        assert!(saved_loc.path().is_file());

        // Reload and verify persistence
        let (reloaded, reloaded_loc) =
            WorkspaceState::load_for_dir(&fake_cwd, Some(&fake_xdg), None).unwrap();
        assert_eq!(reloaded_loc, saved_loc);
        assert_eq!(reloaded.pinned_constraints, vec!["No QEMU"]);
        assert_eq!(
            reloaded.default_plan_path.as_deref(),
            Some("STEPPED_IMPLEMENTATION_PLAN.md")
        );
        assert_eq!(reloaded.selected_effort, "high");

        let _ = fs::remove_dir_all(&tmp_root);
    }

    #[test]
    fn test_explicit_local_save_creates_and_uses_dot_verbot() {
        let tmp_root = std::env::temp_dir().join(format!(
            "verbot_state_local_opt_in_{}",
            std::process::id()
        ));
        let fake_cwd = tmp_root.join("project_with_local");
        let fake_xdg = tmp_root.join("xdg_data");
        fs::create_dir_all(&fake_cwd).unwrap();

        let (mut state, _) =
            WorkspaceState::load_for_dir(&fake_cwd, Some(&fake_xdg), None).unwrap();
        state.toggle_pinned_constraint("Drop .img");

        // Explicitly request local project storage (`force_local = true`)
        let saved_loc = state
            .save_for_dir(&fake_cwd, true, Some(&fake_xdg))
            .unwrap();
        assert!(saved_loc.is_project_local());
        assert!(fake_cwd.join(".verbot/state.toml").is_file());

        // Subsequent loads automatically detect `.verbot/state.toml`
        let (reloaded, reloaded_loc) =
            WorkspaceState::load_for_dir(&fake_cwd, Some(&fake_xdg), None).unwrap();
        assert!(reloaded_loc.is_project_local());
        assert_eq!(reloaded.pinned_constraints, vec!["Drop .img"]);

        let _ = fs::remove_dir_all(&tmp_root);
    }

    #[test]
    fn test_prompt_history_ring_buffer_and_theme_persistence() {
        let tmp_root = std::env::temp_dir().join(format!(
            "verbot_state_history_theme_{}",
            std::process::id()
        ));
        fs::create_dir_all(&tmp_root).unwrap();
        let theme_toml = tmp_root.join("theme.toml");

        let mut state = WorkspaceState::new_for_dir(&tmp_root);
        let resolved_theme = state
            .set_theme_preset(ThemePreset::NovosSolarLight, Some(&theme_toml))
            .unwrap();
        assert_eq!(resolved_theme.preset, ThemePreset::NovosSolarLight);
        assert!(theme_toml.is_file());

        let reloaded_theme = Theme::load_from_toml_file(&theme_toml).unwrap();
        assert_eq!(reloaded_theme.preset, ThemePreset::NovosSolarLight);

        for i in 0..(MAX_PROMPT_HISTORY + 5) {
            let mut vals = PromptValues::new();
            vals.objective = format!("Task {}", i);
            state.record_prompt(
                "gemini-4-argon-medium",
                EffortLevel::Medium,
                vals,
                format!("/plan Task {}", i),
            );
        }
        assert_eq!(state.prompt_history.len(), MAX_PROMPT_HISTORY);
        assert_eq!(
            state.prompt_history.front().unwrap().values.objective,
            "Task 5"
        );
        assert_eq!(
            state.prompt_history.back().unwrap().values.objective,
            format!("Task {}", MAX_PROMPT_HISTORY + 4)
        );

        let _ = fs::remove_dir_all(&tmp_root);
    }
}
