use crate::template::PromptValues;
use crate::theme::default_verbot_config_dir;
use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Supported LLM Model Families.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModelFamily {
    Gemini,
    Claude,
    Gpt,
}

impl ModelFamily {
    pub const ALL: [ModelFamily; 3] = [Self::Gemini, Self::Claude, Self::Gpt];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Gemini => "Gemini",
            Self::Claude => "Claude",
            Self::Gpt => "GPT",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Gemini => Self::Claude,
            Self::Claude => Self::Gpt,
            Self::Gpt => Self::Gemini,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Gemini => Self::Gpt,
            Self::Claude => Self::Gemini,
            Self::Gpt => Self::Claude,
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        let lower = s.trim().to_ascii_lowercase();
        if lower.starts_with("claude") || lower.contains("anthropic") {
            Self::Claude
        } else if lower.starts_with("gpt") || lower.starts_with("openai") || lower.starts_with('o') {
            Self::Gpt
        } else {
            Self::Gemini
        }
    }

    pub fn default_format_style(self) -> OutputFormatStyle {
        match self {
            Self::Gemini => OutputFormatStyle::GeminiConcise,
            Self::Claude => OutputFormatStyle::ClaudeXml,
            Self::Gpt => OutputFormatStyle::GptMarkdownSections,
        }
    }
}

/// Thinking / reasoning effort levels supported by `agy --effort`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    XHigh,
    Max,
}

impl EffortLevel {
    pub const ALL: [EffortLevel; 5] = [
        Self::Low,
        Self::Medium,
        Self::High,
        Self::XHigh,
        Self::Max,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High => "high",
            Self::XHigh => "xhigh",
            Self::Max => "max",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "low" | "l" => Self::Low,
            "medium" | "med" | "m" => Self::Medium,
            "high" | "h" => Self::High,
            "xhigh" | "xh" => Self::XHigh,
            "max" => Self::Max,
            _ => Self::Medium,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Low => Self::Medium,
            Self::Medium => Self::High,
            Self::High => Self::XHigh,
            Self::XHigh => Self::Max,
            Self::Max => Self::Low,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::Low => Self::Max,
            Self::Medium => Self::Low,
            Self::High => Self::Medium,
            Self::XHigh => Self::High,
            Self::Max => Self::XHigh,
        }
    }

    /// Maps an effort level onto the 3-tier model ID suffix (`low`, `medium`, `high`) used by `agy models`.
    pub fn model_tier_suffix(self) -> &'static str {
        match self {
            Self::Low => "low",
            Self::Medium => "medium",
            Self::High | Self::XHigh | Self::Max => "high",
        }
    }
}

/// Output formatting strategies tailored to each LLM family.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum OutputFormatStyle {
    /// Concise high-density Markdown bullets (`GeminiAgentPromptTemplate.md` format).
    GeminiConcise,
    /// Structured XML blocks (`<context>`, `<constraints>`, `<output>`) for Claude models.
    ClaudeXml,
    /// Markdown section headings (`### Context / Logs`, `### Constraints`, `### Output`) for GPT models.
    GptMarkdownSections,
    /// User-defined format string with `{{command}}`, `{{objective}}`, `{{context_logs}}`, `{{constraints}}`, `{{output}}`.
    CustomTemplate(String),
}

/// Sub-model profile definition mapping UI selection to `agy` model IDs and prompt formatting.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubModelProfile {
    pub family: ModelFamily,
    /// Short sub-model name (e.g., `"Argon"`, `"Flash"`, `"Opus"`, `"Sonnet"`, `"GPT-OSS 120B"`).
    pub short_name: String,
    /// Human-readable display label (e.g., `"Gemini 4 Argon"`).
    pub display_name: String,
    /// Base `agy` model prefix (e.g., `"gemini-4-argon"`, `"claude-opus-5-5"`, `"gpt-oss-120b"`).
    pub agy_base_id: String,
    /// Supported `agy` model tier suffixes for this sub-model (e.g., `["low", "medium", "high"]` or `["medium"]`).
    pub available_tiers: Vec<String>,
    /// Default effort level when selecting this sub-model.
    pub default_effort: EffortLevel,
    /// Output formatter used when rendering prompts for this sub-model.
    pub format_style: OutputFormatStyle,
}

impl SubModelProfile {
    /// Resolves the exact `agy` model ID (as listed in `agy models`) for a given [`EffortLevel`].
    pub fn resolve_agy_model_id(&self, effort: EffortLevel) -> String {
        let desired_tier = effort.model_tier_suffix();
        let chosen_tier = if self.available_tiers.iter().any(|t| t == desired_tier) {
            desired_tier
        } else if let Some(first) = self.available_tiers.first() {
            first.as_str()
        } else {
            "medium"
        };
        format!("{}-{}", self.agy_base_id, chosen_tier)
    }

    /// Renders canonical [`PromptValues`] into this model's output format without mutating `values`.
    pub fn render_prompt(&self, values: &PromptValues) -> String {
        render_with_style(&self.format_style, values)
    }
}

/// Catalog of built-in and custom model profiles.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelCatalog {
    pub profiles: Vec<SubModelProfile>,
}

impl Default for ModelCatalog {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ModelCatalog {
    /// Constructs the built-in catalog seeded from `agy models` and `GeminiAgentPromptTemplate.md`.
    pub fn builtin() -> Self {
        let tiers_lmh = vec![
            "low".to_string(),
            "medium".to_string(),
            "high".to_string(),
        ];
        Self {
            profiles: vec![
                SubModelProfile {
                    family: ModelFamily::Gemini,
                    short_name: "Argon".to_string(),
                    display_name: "Gemini 4 Argon".to_string(),
                    agy_base_id: "gemini-4-argon".to_string(),
                    available_tiers: tiers_lmh.clone(),
                    default_effort: EffortLevel::Medium,
                    format_style: OutputFormatStyle::GeminiConcise,
                },
                SubModelProfile {
                    family: ModelFamily::Gemini,
                    short_name: "Flash".to_string(),
                    display_name: "Gemini 3.8 Flash".to_string(),
                    agy_base_id: "gemini-3.8-flash".to_string(),
                    available_tiers: tiers_lmh.clone(),
                    default_effort: EffortLevel::High,
                    format_style: OutputFormatStyle::GeminiConcise,
                },
                SubModelProfile {
                    family: ModelFamily::Claude,
                    short_name: "Opus".to_string(),
                    display_name: "Claude Opus 5.5".to_string(),
                    agy_base_id: "claude-opus-5-5".to_string(),
                    available_tiers: tiers_lmh.clone(),
                    default_effort: EffortLevel::High,
                    format_style: OutputFormatStyle::ClaudeXml,
                },
                SubModelProfile {
                    family: ModelFamily::Claude,
                    short_name: "Sonnet".to_string(),
                    display_name: "Claude Sonnet 5.5".to_string(),
                    agy_base_id: "claude-sonnet-5-5".to_string(),
                    available_tiers: tiers_lmh,
                    default_effort: EffortLevel::Medium,
                    format_style: OutputFormatStyle::ClaudeXml,
                },
                SubModelProfile {
                    family: ModelFamily::Gpt,
                    short_name: "GPT-OSS 120B".to_string(),
                    display_name: "GPT-OSS 120B".to_string(),
                    agy_base_id: "gpt-oss-120b".to_string(),
                    available_tiers: vec!["medium".to_string()],
                    default_effort: EffortLevel::Medium,
                    format_style: OutputFormatStyle::GptMarkdownSections,
                },
            ],
        }
    }

    pub fn profiles_for_family(&self, family: ModelFamily) -> Vec<&SubModelProfile> {
        self.profiles
            .iter()
            .filter(|p| p.family == family)
            .collect()
    }

    /// Finds a profile by short name (`"Argon"`), display name, base ID (`"gemini-4-argon"`),
    /// or full `agy` model ID (`"gemini-4-argon-medium"`), returning the profile and optional inferred effort.
    pub fn find_by_query(&self, query: &str) -> Option<(&SubModelProfile, Option<EffortLevel>)> {
        let q = query.trim().to_ascii_lowercase();
        for profile in &self.profiles {
            if profile.short_name.to_ascii_lowercase() == q
                || profile.display_name.to_ascii_lowercase() == q
                || profile.agy_base_id.to_ascii_lowercase() == q
            {
                return Some((profile, None));
            }
            for tier in &profile.available_tiers {
                let full_id = format!("{}-{}", profile.agy_base_id, tier);
                if full_id == q {
                    return Some((profile, Some(EffortLevel::from_str_loose(tier))));
                }
            }
        }
        None
    }

    pub fn load_from_toml_str(raw: &str) -> Result<Self> {
        toml::from_str(raw).context("Failed to parse ModelCatalog TOML")
    }

    pub fn load_from_yaml_str(raw: &str) -> Result<Self> {
        serde_yaml::from_str(raw).context("Failed to parse ModelCatalog YAML")
    }

    /// Loads the built-in catalog and merges any user-saved models from `~/.config/verbot/models.toml`
    /// (or `custom_path` when provided).
    pub fn load_user_or_builtin(custom_path: Option<&Path>) -> Self {
        let mut catalog = Self::builtin();
        let path = custom_path
            .map(PathBuf::from)
            .or_else(default_verbot_models_path);
        if let Some(ref p) = path {
            if p.is_file() {
                if let Ok(raw) = fs::read_to_string(p) {
                    if let Ok(user_cat) = Self::load_from_toml_str(&raw) {
                        catalog.merge_profiles(user_cat.profiles);
                    }
                }
            }
        }
        catalog
    }

    /// Saves the catalog to `~/.config/verbot/models.toml` (or `custom_path`).
    pub fn save_to_toml_file(&self, custom_path: Option<&Path>) -> Result<PathBuf> {
        let path = custom_path
            .map(PathBuf::from)
            .or_else(default_verbot_models_path)
            .unwrap_or_else(|| std::env::temp_dir().join("verbot-models.toml"));
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                fs::create_dir_all(parent).with_context(|| {
                    format!(
                        "Failed to create model catalog config directory {}",
                        parent.display()
                    )
                })?;
            }
        }
        let raw = toml::to_string_pretty(self).context("Failed to serialize ModelCatalog to TOML")?;
        fs::write(&path, raw)
            .with_context(|| format!("Failed to write model catalog to {}", path.display()))?;
        Ok(path)
    }

    /// Inserts or updates a [`SubModelProfile`] in the catalog (matching by `agy_base_id` or `short_name`).
    /// Returns the index of the upserted profile in `self.profiles`.
    pub fn upsert_profile(&mut self, mut incoming: SubModelProfile) -> usize {
        if incoming.available_tiers.is_empty() {
            incoming.available_tiers = vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
            ];
        }
        if let Some(idx) = self.profiles.iter().position(|existing| {
            existing
                .agy_base_id
                .eq_ignore_ascii_case(&incoming.agy_base_id)
                || (existing.family == incoming.family
                    && existing
                        .short_name
                        .eq_ignore_ascii_case(&incoming.short_name))
        }) {
            let existing = &mut self.profiles[idx];
            existing.family = incoming.family;
            existing.short_name = incoming.short_name;
            existing.display_name = incoming.display_name;
            existing.agy_base_id = incoming.agy_base_id;
            existing.default_effort = incoming.default_effort;
            existing.format_style = incoming.format_style;
            for tier in incoming.available_tiers {
                if !existing
                    .available_tiers
                    .iter()
                    .any(|t| t.eq_ignore_ascii_case(&tier))
                {
                    existing.available_tiers.push(tier);
                }
            }
            idx
        } else {
            self.profiles.push(incoming);
            self.profiles.len() - 1
        }
    }

    /// Merges a list of discovered [`SubModelProfile`] entries into the catalog.
    /// Returns the number of profiles added or updated.
    pub fn merge_profiles(&mut self, profiles: Vec<SubModelProfile>) -> usize {
        let mut touched = 0;
        for profile in profiles {
            self.upsert_profile(profile);
            touched += 1;
        }
        touched
    }

    /// Invokes `agy models` on the system (or `custom_agy_bin`) and merges any discovered models into the catalog.
    pub fn sync_from_agy_cli(&mut self, custom_agy_bin: Option<&Path>) -> Result<usize> {
        let agy_bin = if let Some(p) = custom_agy_bin {
            p.to_path_buf()
        } else if let Ok(env_bin) = std::env::var("VERBOT_AGY_BIN") {
            PathBuf::from(env_bin)
        } else if let Some(home_agy) = dirs::home_dir().map(|h| h.join(".local/bin/agy")) {
            if home_agy.is_file() {
                home_agy
            } else {
                PathBuf::from("agy")
            }
        } else {
            PathBuf::from("agy")
        };

        let out = Command::new(&agy_bin)
            .arg("models")
            .output()
            .with_context(|| format!("Failed to execute '{} models'", agy_bin.display()))?;
        if !out.status.success() {
            return Err(anyhow!(
                "'{} models' exited with status {}: {}",
                agy_bin.display(),
                out.status,
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        let discovered = parse_agy_models_output(&stdout);
        if discovered.is_empty() {
            return Err(anyhow!(
                "No model identifiers discovered in '{} models' output",
                agy_bin.display()
            ));
        }
        Ok(self.merge_profiles(discovered))
    }

    /// Validates and ingests an external `.toml` or `.yaml`/`.yml` model file (supporting either a
    /// single sub-model definition or a `[[profiles]]` / `[[models]]` array), merges the discovered
    /// profile(s) into `self`, and persists the updated catalog to `models.toml`.
    pub fn ingest_model_file(
        &mut self,
        source_path: &Path,
        custom_models_toml: Option<&Path>,
    ) -> Result<Vec<SubModelProfile>> {
        let raw = fs::read_to_string(source_path).with_context(|| {
            format!(
                "Failed to read model file for ingestion: {}",
                source_path.display()
            )
        })?;
        let ext = source_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("toml")
            .to_ascii_lowercase();
        let profiles = parse_external_model_profiles(&raw, &ext).with_context(|| {
            format!(
                "Failed to parse model profile(s) from {}",
                source_path.display()
            )
        })?;
        if profiles.is_empty() {
            return Err(anyhow!(
                "No valid model profiles found in {}",
                source_path.display()
            ));
        }
        for profile in &profiles {
            self.upsert_profile(profile.clone());
        }
        self.save_to_toml_file(custom_models_toml)?;
        Ok(profiles)
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FlexibleSubModelInput {
    #[serde(default)]
    pub family: Option<String>,
    #[serde(default)]
    pub short_name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
    #[serde(default)]
    pub agy_base_id: Option<String>,
    #[serde(default)]
    pub available_tiers: Vec<String>,
    #[serde(default)]
    pub default_effort: Option<String>,
    #[serde(default)]
    pub format_style: Option<OutputFormatStyle>,
}

impl FlexibleSubModelInput {
    fn into_profile(self) -> Result<SubModelProfile> {
        let raw_short = self.short_name.unwrap_or_default().trim().to_string();
        let raw_base = self.agy_base_id.unwrap_or_default().trim().to_string();
        let raw_display = self.display_name.unwrap_or_default().trim().to_string();

        if raw_short.is_empty() && raw_base.is_empty() && raw_display.is_empty() {
            return Err(anyhow!(
                "Model profile must specify at least `short_name` or `agy_base_id`"
            ));
        }

        let family = match self.family.as_deref() {
            Some(f) if !f.trim().is_empty() => ModelFamily::from_str_loose(f),
            None => {
                if !raw_base.is_empty() {
                    ModelFamily::from_str_loose(&raw_base)
                } else {
                    ModelFamily::Gemini
                }
            }
            _ => ModelFamily::Gemini,
        };

        let short_name = if !raw_short.is_empty() {
            raw_short
        } else if !raw_display.is_empty() {
            raw_display.clone()
        } else {
            derive_names_from_base_id(&raw_base, family).0
        };

        let display_name = if !raw_display.is_empty() {
            raw_display
        } else {
            format!("{} {}", family.as_str(), short_name)
        };

        let agy_base_id = if !raw_base.is_empty() {
            raw_base
                .trim_end_matches("-low")
                .trim_end_matches("-medium")
                .trim_end_matches("-high")
                .to_ascii_lowercase()
        } else {
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
            match family {
                ModelFamily::Gemini => format!("gemini-{}", slug),
                ModelFamily::Claude => format!("claude-{}", slug),
                ModelFamily::Gpt => format!("gpt-{}", slug),
            }
        };

        let available_tiers = if self.available_tiers.is_empty() {
            vec![
                "low".to_string(),
                "medium".to_string(),
                "high".to_string(),
            ]
        } else {
            self.available_tiers
        };

        let default_effort = self
            .default_effort
            .as_deref()
            .map(EffortLevel::from_str_loose)
            .unwrap_or(EffortLevel::Medium);

        let format_style = self
            .format_style
            .unwrap_or_else(|| family.default_format_style());

        Ok(SubModelProfile {
            family,
            short_name,
            display_name,
            agy_base_id,
            available_tiers,
            default_effort,
            format_style,
        })
    }
}

#[derive(Debug, Clone, Deserialize)]
struct FlexibleModelListWrapper {
    #[serde(default)]
    pub profiles: Vec<FlexibleSubModelInput>,
    #[serde(default)]
    pub models: Vec<FlexibleSubModelInput>,
}

/// Parses either a `[[profiles]]` / `[[models]]` collection or a single top-level sub-model profile
/// from a `.toml` or `.yaml`/`.yml` string.
pub fn parse_external_model_profiles(raw: &str, ext: &str) -> Result<Vec<SubModelProfile>> {
    let is_yaml = matches!(ext, "yaml" | "yml");

    if is_yaml {
        if let Ok(cat) = ModelCatalog::load_from_yaml_str(raw) {
            if !cat.profiles.is_empty() {
                return Ok(cat.profiles);
            }
        }
        if let Ok(wrapper) = serde_yaml::from_str::<FlexibleModelListWrapper>(raw) {
            let items = if !wrapper.profiles.is_empty() {
                wrapper.profiles
            } else {
                wrapper.models
            };
            if !items.is_empty() {
                return items
                    .into_iter()
                    .map(FlexibleSubModelInput::into_profile)
                    .collect();
            }
        }
        let single: FlexibleSubModelInput =
            serde_yaml::from_str(raw).context("Invalid YAML model profile")?;
        Ok(vec![single.into_profile()?])
    } else {
        if let Ok(cat) = ModelCatalog::load_from_toml_str(raw) {
            if !cat.profiles.is_empty() {
                return Ok(cat.profiles);
            }
        }
        if let Ok(wrapper) = toml::from_str::<FlexibleModelListWrapper>(raw) {
            let items = if !wrapper.profiles.is_empty() {
                wrapper.profiles
            } else {
                wrapper.models
            };
            if !items.is_empty() {
                return items
                    .into_iter()
                    .map(FlexibleSubModelInput::into_profile)
                    .collect();
            }
        }
        let single: FlexibleSubModelInput =
            toml::from_str(raw).context("Invalid TOML model profile")?;
        Ok(vec![single.into_profile()?])
    }
}

pub fn default_verbot_models_path() -> Option<PathBuf> {
    default_verbot_config_dir().map(|d| d.join("models.toml"))
}

/// Parses the plain-text output of `agy models` into a deduplicated list of [`SubModelProfile`]s.
pub fn parse_agy_models_output(raw: &str) -> Vec<SubModelProfile> {
    let mut discovered: Vec<SubModelProfile> = Vec::new();
    let known_tiers = ["low", "medium", "high", "xhigh", "max"];

    for line in raw.lines() {
        let trimmed = line
            .trim()
            .trim_start_matches(|c: char| c == '-' || c == '*' || c == '•' || c.is_whitespace());
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with("Available") {
            continue;
        }

        // Extract first whitespace/colon/pipe-delimited token as candidate model ID
        let Some(first_token) = trimmed
            .split(|c: char| c.is_whitespace() || c == ':' || c == '|')
            .find(|s| !s.is_empty())
        else {
            continue;
        };
        let token_clean = first_token
            .trim_matches(|c: char| c == '`' || c == '\'' || c == '"' || c == ',' || c == '(' || c == ')')
            .to_ascii_lowercase();

        if !token_clean.contains('-')
            || !token_clean
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
        {
            continue;
        }

        // Determine if the model ID ends with a tier suffix (`-low`, `-medium`, `-high`, etc.)
        let mut base_id = token_clean.as_str();
        let mut detected_tier: Option<String> = None;
        for &tier in &known_tiers {
            let suffix = format!("-{}", tier);
            if let Some(stripped) = token_clean.strip_suffix(&suffix) {
                if !stripped.is_empty() {
                    base_id = stripped;
                    detected_tier = Some(tier.to_string());
                    break;
                }
            }
        }

        let family = if base_id.starts_with("gemini") {
            ModelFamily::Gemini
        } else if base_id.starts_with("claude") {
            ModelFamily::Claude
        } else if base_id.starts_with("gpt")
            || base_id.starts_with("o1")
            || base_id.starts_with("o3")
            || base_id.starts_with("o4")
        {
            ModelFamily::Gpt
        } else if detected_tier.is_some() {
            ModelFamily::Gemini
        } else {
            continue;
        };

        let tier_to_add = detected_tier.unwrap_or_else(|| "medium".to_string());

        if let Some(existing) = discovered
            .iter_mut()
            .find(|p| p.agy_base_id.eq_ignore_ascii_case(base_id))
        {
            if !existing
                .available_tiers
                .iter()
                .any(|t| t.eq_ignore_ascii_case(&tier_to_add))
            {
                existing.available_tiers.push(tier_to_add);
            }
            continue;
        }

        let (short_name, display_name) = derive_names_from_base_id(base_id, family);
        let default_effort = if tier_to_add == "high" {
            EffortLevel::High
        } else {
            EffortLevel::Medium
        };

        discovered.push(SubModelProfile {
            family,
            short_name,
            display_name,
            agy_base_id: base_id.to_string(),
            available_tiers: vec![tier_to_add],
            default_effort,
            format_style: family.default_format_style(),
        });
    }

    discovered
}

fn derive_names_from_base_id(base_id: &str, family: ModelFamily) -> (String, String) {
    match base_id {
        "gemini-4-argon" => ("Argon".to_string(), "Gemini 4 Argon".to_string()),
        "gemini-3.8-flash" => ("Flash".to_string(), "Gemini 3.8 Flash".to_string()),
        "claude-opus-5-5" => ("Opus".to_string(), "Claude Opus 5.5".to_string()),
        "claude-sonnet-5-5" => ("Sonnet".to_string(), "Claude Sonnet 5.5".to_string()),
        "gpt-oss-120b" => ("GPT-OSS 120B".to_string(), "GPT-OSS 120B".to_string()),
        _ => {
            let parts: Vec<&str> = base_id.split('-').filter(|s| !s.is_empty()).collect();
            let title_parts: Vec<String> = parts
                .iter()
                .map(|part| {
                    if part.eq_ignore_ascii_case("gpt") || part.eq_ignore_ascii_case("oss") {
                        part.to_ascii_uppercase()
                    } else {
                        let mut chars = part.chars();
                        match chars.next() {
                            Some(first) => {
                                format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
                            }
                            None => String::new(),
                        }
                    }
                })
                .collect();
            let display_name = title_parts.join(" ");
            let short_name = match family {
                ModelFamily::Gemini | ModelFamily::Claude => parts
                    .iter()
                    .skip(1)
                    .find(|p| p.chars().any(|c| c.is_ascii_alphabetic()))
                    .map(|s| {
                        let mut c = s.chars();
                        match c.next() {
                            Some(f) => format!("{}{}", f.to_ascii_uppercase(), c.as_str()),
                            None => s.to_string(),
                        }
                    })
                    .unwrap_or_else(|| display_name.clone()),
                ModelFamily::Gpt => display_name.clone(),
            };
            (short_name, display_name)
        }
    }
}

/// Known template placeholder hints that must never leak into rendered prompts or CLI invocations.
const KNOWN_PLACEHOLDER_HINTS: [&str; 4] = [
    "<1-2 sentence symptom or objective>",
    "<file paths or terminal snippet>",
    "<what to omit, change structurally, or not run (e.g., \"No QEMU\", \"Drop .img\")>",
    "<Diagnose only | Update <file>.md plan | Execute Step X>",
];

/// Returns `Some(trimmed)` if `raw` contains genuine user text, or `None` if it is empty or
/// consists solely of a template `<placeholder>` hint.
pub fn sanitize_field_value(raw: &str) -> Option<&str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return None;
    }
    for hint in KNOWN_PLACEHOLDER_HINTS {
        if trimmed.eq_ignore_ascii_case(hint)
            || trimmed.eq_ignore_ascii_case(hint.trim_start_matches('<').trim_end_matches('>'))
        {
            return None;
        }
    }
    if trimmed.starts_with('<') && trimmed.ends_with('>') && !trimmed.contains('\n') {
        let inner = &trimmed[1..trimmed.len() - 1];
        if inner.eq_ignore_ascii_case("placeholder") {
            return None;
        }
    }
    Some(trimmed)
}

/// Renders [`PromptValues`] according to the specified [`OutputFormatStyle`], omitting empty fields
/// and stripping all `<placeholder>` hint strings.
pub fn render_with_style(style: &OutputFormatStyle, values: &PromptValues) -> String {
    let cmd_prefix = values.formatted_slash_command();
    let objective = sanitize_field_value(&values.objective);
    let context_logs = sanitize_field_value(&values.context_logs);
    let constraints = sanitize_field_value(&values.constraints);
    let output = sanitize_field_value(&values.output);

    let headline = match (&cmd_prefix, objective) {
        (Some(cmd), Some(obj)) => Some(format!("{} {}", cmd, obj)),
        (Some(cmd), None) => Some(cmd.clone()),
        (None, Some(obj)) => Some(obj.to_string()),
        (None, None) => None,
    };

    match style {
        OutputFormatStyle::GeminiConcise => {
            let mut lines: Vec<String> = Vec::new();
            if let Some(ref h) = headline {
                lines.push(h.clone());
            }
            if let Some(ctx) = context_logs {
                lines.push(format!("- Context/Logs: {}", ctx));
            }
            if let Some(cons) = constraints {
                lines.push(format!("- Constraints: {}", cons));
            }
            if let Some(out) = output {
                lines.push(format!("- Output: {}", out));
            }
            for (k, v) in &values.custom_fields {
                if let Some(val) = sanitize_field_value(v) {
                    lines.push(format!("- {}: {}", k, val));
                }
            }
            lines.join("\n")
        }
        OutputFormatStyle::ClaudeXml => {
            let mut xml_blocks: Vec<String> = Vec::new();
            if let Some(ctx) = context_logs {
                xml_blocks.push(format!("<context>\n{}\n</context>", ctx));
            }
            if let Some(cons) = constraints {
                xml_blocks.push(format!("<constraints>\n{}\n</constraints>", cons));
            }
            if let Some(out) = output {
                xml_blocks.push(format!("<output>\n{}\n</output>", out));
            }
            for (k, v) in &values.custom_fields {
                if let Some(val) = sanitize_field_value(v) {
                    let tag = crate::template::normalize_field_id(k);
                    xml_blocks.push(format!("<{tag}>\n{}\n</{tag}>", val));
                }
            }
            match (headline, xml_blocks.is_empty()) {
                (Some(h), false) => format!("{}\n\n{}", h, xml_blocks.join("\n")),
                (Some(h), true) => h,
                (None, false) => xml_blocks.join("\n"),
                (None, true) => String::new(),
            }
        }
        OutputFormatStyle::GptMarkdownSections => {
            let mut sections: Vec<String> = Vec::new();
            if let Some(h) = headline {
                sections.push(h);
            }
            if let Some(ctx) = context_logs {
                sections.push(format!("### Context / Logs\n{}", ctx));
            }
            if let Some(cons) = constraints {
                sections.push(format!("### Constraints\n{}", cons));
            }
            if let Some(out) = output {
                sections.push(format!("### Output\n{}", out));
            }
            for (k, v) in &values.custom_fields {
                if let Some(val) = sanitize_field_value(v) {
                    sections.push(format!("### {}\n{}", k, val));
                }
            }
            sections.join("\n\n")
        }
        OutputFormatStyle::CustomTemplate(tpl) => {
            let raw = tpl
                .replace("{{command}}", cmd_prefix.as_deref().unwrap_or(""))
                .replace("{{headline}}", headline.as_deref().unwrap_or(""))
                .replace("{{objective}}", objective.unwrap_or(""))
                .replace("{{context_logs}}", context_logs.unwrap_or(""))
                .replace("{{constraints}}", constraints.unwrap_or(""))
                .replace("{{output}}", output.unwrap_or(""));
            raw.lines()
                .map(str::trim_end)
                .filter(|line| {
                    let t = line.trim();
                    !t.is_empty()
                        && t != "- Context/Logs:"
                        && t != "- Constraints:"
                        && t != "- Output:"
                })
                .collect::<Vec<_>>()
                .join("\n")
        }
    }
}

/// Dispatch mode for `agy` CLI invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgyLaunchMode {
    Interactive,
    Print,
}

impl AgyLaunchMode {
    pub fn flag(self) -> &'static str {
        match self {
            Self::Interactive => "-i",
            Self::Print => "-p",
        }
    }
}

/// Formats a shell-safe preview of the exact `agy` CLI invocation.
pub fn build_agy_command_preview(
    mode: AgyLaunchMode,
    model_id: &str,
    effort: EffortLevel,
    prompt_preview: &str,
) -> String {
    let single_line = prompt_preview
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .collect::<Vec<_>>()
        .join(" ");
    let truncated = if single_line.chars().count() > 64 {
        let prefix: String = single_line.chars().take(61).collect();
        format!("{}...", prefix)
    } else {
        single_line
    };
    let escaped = truncated.replace('"', "\\\"");
    format!(
        "agy --model {} --effort {} {} \"{}\"",
        model_id,
        effort.as_str(),
        mode.flag(),
        escaped
    )
}

/// Conciseness metrics for the live prompt preview footer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PromptMetrics {
    pub chars: usize,
    pub words: usize,
    pub estimated_tokens: usize,
    pub lines: usize,
}

pub fn estimate_prompt_metrics(rendered: &str) -> PromptMetrics {
    let chars = rendered.chars().count();
    let words = rendered.split_whitespace().count();
    // Standard technical text heuristic: ~4 chars per token or ~1.35 tokens per word
    let estimated_tokens = ((chars as f64 / 3.8).ceil() as usize).max(words);
    let lines = rendered.lines().count().max(1);
    PromptMetrics {
        chars,
        words,
        estimated_tokens,
        lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_decoupled_values_across_model_formatters() {
        let catalog = ModelCatalog::builtin();
        let mut values = PromptValues::new();
        values.slash_command = Some("goal".to_string());
        values.objective = "Fix greetd PAM session startup".to_string();
        values.context_logs = "src/main.rs, journalctl -u greetd".to_string();
        values.constraints = "No QEMU; keep existing PAM stack".to_string();
        values.output = "Execute Step 1".to_string();

        let (argon, _) = catalog.find_by_query("Argon").unwrap();
        let (opus, _) = catalog.find_by_query("Opus").unwrap();
        let (gpt, _) = catalog.find_by_query("GPT-OSS 120B").unwrap();

        let rendered_gemini = argon.render_prompt(&values);
        assert!(rendered_gemini.starts_with("/goal Fix greetd PAM session startup"));
        assert!(rendered_gemini.contains("- Context/Logs: src/main.rs, journalctl -u greetd"));
        assert!(rendered_gemini.contains("- Constraints: No QEMU; keep existing PAM stack"));
        assert!(rendered_gemini.contains("- Output: Execute Step 1"));

        let rendered_claude = opus.render_prompt(&values);
        assert!(rendered_claude.contains("<context>\nsrc/main.rs, journalctl -u greetd\n</context>"));
        assert!(rendered_claude.contains("<constraints>\nNo QEMU; keep existing PAM stack\n</constraints>"));

        let rendered_gpt = gpt.render_prompt(&values);
        assert!(rendered_gpt.contains("### Context / Logs\nsrc/main.rs, journalctl -u greetd"));

        // Verify original canonical field state was never mutated by formatting
        assert_eq!(values.objective, "Fix greetd PAM session startup");
        assert_eq!(values.constraints, "No QEMU; keep existing PAM stack");
    }

    #[test]
    fn test_agy_model_id_resolution() {
        let catalog = ModelCatalog::builtin();
        let (argon, _) = catalog.find_by_query("Argon").unwrap();
        assert_eq!(
            argon.resolve_agy_model_id(EffortLevel::Low),
            "gemini-4-argon-low"
        );
        assert_eq!(
            argon.resolve_agy_model_id(EffortLevel::Medium),
            "gemini-4-argon-medium"
        );
        assert_eq!(
            argon.resolve_agy_model_id(EffortLevel::XHigh),
            "gemini-4-argon-high"
        );

        let (gpt, _) = catalog.find_by_query("GPT-OSS 120B").unwrap();
        assert_eq!(
            gpt.resolve_agy_model_id(EffortLevel::High),
            "gpt-oss-120b-medium"
        );
    }

    #[test]
    fn test_parse_agy_models_output_and_catalog_upsert() {
        let sample_output = r#"
Available models:
  gemini-4-argon-low
  gemini-4-argon-medium
  gemini-4-argon-high
  gemini-4-nebula-medium   Gemini 4 Nebula (Medium)
  gemini-4-nebula-high     Gemini 4 Nebula (High)
  claude-opus-5-6-high     Claude Opus 5.6
  gpt-oss-120b-medium
"#;
        let parsed = parse_agy_models_output(sample_output);
        assert!(parsed.iter().any(|p| p.agy_base_id == "gemini-4-nebula"
            && p.short_name == "Nebula"
            && p.available_tiers == vec!["medium", "high"]));
        assert!(parsed
            .iter()
            .any(|p| p.agy_base_id == "claude-opus-5-6" && p.family == ModelFamily::Claude));

        let mut catalog = ModelCatalog::builtin();
        let merged = catalog.merge_profiles(parsed);
        assert!(merged >= 4);
        let (nebula, _) = catalog.find_by_query("Nebula").expect("should find Nebula");
        assert_eq!(
            nebula.resolve_agy_model_id(EffortLevel::High),
            "gemini-4-nebula-high"
        );
    }

    #[test]
    fn test_placeholder_free_rendered_output_omits_empty_fields() {
        let mut empty_values = PromptValues::new();
        empty_values.slash_command = Some("plan".to_string());

        for style in [
            OutputFormatStyle::GeminiConcise,
            OutputFormatStyle::ClaudeXml,
            OutputFormatStyle::GptMarkdownSections,
            OutputFormatStyle::CustomTemplate(
                "{{headline}}\n- Context/Logs: {{context_logs}}\n- Constraints: {{constraints}}\n- Output: {{output}}"
                    .to_string(),
            ),
        ] {
            let rendered = render_with_style(&style, &empty_values);
            assert_eq!(rendered, "/plan");
            assert!(!rendered.contains("<1-2 sentence"));
            assert!(!rendered.contains("<file paths"));
            assert!(!rendered.contains("<what to omit"));
            assert!(!rendered.contains("<Diagnose only"));
        }

        // Also verify literal placeholder hints are stripped if passed
        empty_values.objective = "<1-2 sentence symptom or objective>".to_string();
        empty_values.context_logs = "src/tui.rs".to_string();
        let partial = render_with_style(&OutputFormatStyle::GeminiConcise, &empty_values);
        assert_eq!(partial, "/plan\n- Context/Logs: src/tui.rs");
    }
}
