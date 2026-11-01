use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Built-in baseline Markdown template content (`templates/GeminiAgentPromptTemplate.md`).
pub const DEFAULT_GEMINI_TEMPLATE_MD: &str = include_str!("../templates/GeminiAgentPromptTemplate.md");

/// Canonical built-in Antigravity (`agy`) slash commands.
pub const BUILTIN_AGY_SLASH_COMMANDS: [&str; 11] = [
    "plan",
    "boost",
    "goal",
    "grill-me",
    "learn",
    "compact",
    "schedule",
    "automation",
    "browser",
    "plugin",
    "teamwork-preview",
];

/// Metadata and extracted structure of a prompt template.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptTemplate {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// Extracted slash-command choices (e.g., `["plan", "boost", "goal"]`).
    #[serde(default)]
    pub slash_commands: Vec<String>,
    /// Default selected slash command (e.g., `Some("plan")`).
    #[serde(default)]
    pub default_command: Option<String>,
    /// Extracted or configured template fields in display order.
    #[serde(default)]
    pub fields: Vec<TemplateFieldSpec>,
    /// Optional raw body template string.
    #[serde(default)]
    pub raw_markdown: String,
}

/// Specification for a single template field or selector extracted from Markdown or TOML/YAML.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TemplateFieldSpec {
    pub id: String,
    pub label: String,
    pub placeholder: String,
    /// Optional quick-choice options extracted from `[a | b | c]` or `<a | b | c>`.
    #[serde(default)]
    pub choices: Vec<String>,
    /// Whether this field appears inline right after the `/[cmd]` token on the first line.
    #[serde(default)]
    pub inline_with_command: bool,
}

/// Canonical, model-agnostic prompt field values held in memory.
///
/// Decoupled from [`crate::models::ModelProfile`] so switching between Gemini, Claude, and GPT
/// dynamically reformats the rendered prompt without losing or altering any user-entered text.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PromptValues {
    pub slash_command: Option<String>,
    pub objective: String,
    pub context_logs: String,
    pub constraints: String,
    pub output: String,
    #[serde(default)]
    pub custom_fields: Vec<(String, String)>,
}

impl Default for PromptValues {
    fn default() -> Self {
        Self {
            slash_command: Some("plan".to_string()),
            objective: String::new(),
            context_logs: String::new(),
            constraints: String::new(),
            output: String::new(),
            custom_fields: Vec::new(),
        }
    }
}

impl PromptValues {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns the normalized slash command with a leading `/` (e.g. `"/plan"`), or `None` if unset/empty.
    pub fn formatted_slash_command(&self) -> Option<String> {
        let cmd = self.slash_command.as_deref()?.trim();
        if cmd.is_empty() || cmd.eq_ignore_ascii_case("none") {
            return None;
        }
        if cmd.starts_with('/') {
            Some(cmd.to_string())
        } else {
            Some(format!("/{}", cmd))
        }
    }

    /// Reads a canonical or custom field value by ID or label.
    pub fn get(&self, id_or_label: &str) -> &str {
        match normalize_field_id(id_or_label).as_str() {
            "objective" => &self.objective,
            "context_logs" | "context" | "logs" => &self.context_logs,
            "constraints" | "constraint" => &self.constraints,
            "output" | "expected_output" => &self.output,
            other => self
                .custom_fields
                .iter()
                .find(|(k, _)| normalize_field_id(k) == other)
                .map(|(_, v)| v.as_str())
                .unwrap_or(""),
        }
    }

    /// Updates a canonical or custom field value by ID or label without affecting any other fields.
    pub fn set(&mut self, id_or_label: &str, value: impl Into<String>) {
        let val = value.into();
        match normalize_field_id(id_or_label).as_str() {
            "objective" => self.objective = val,
            "context_logs" | "context" | "logs" => self.context_logs = val,
            "constraints" | "constraint" => self.constraints = val,
            "output" | "expected_output" => self.output = val,
            other => {
                if let Some(entry) = self
                    .custom_fields
                    .iter_mut()
                    .find(|(k, _)| normalize_field_id(k) == other)
                {
                    entry.1 = val;
                } else {
                    self.custom_fields.push((id_or_label.to_string(), val));
                }
            }
        }
    }
}

impl Default for PromptTemplate {
    fn default() -> Self {
        Self::parse_markdown(DEFAULT_GEMINI_TEMPLATE_MD)
            .expect("Built-in GeminiAgentPromptTemplate.md must parse cleanly")
    }
}

impl PromptTemplate {
    /// Loads a template from a `.md`, `.toml`, or `.yaml`/`.yml` file on disk.
    /// If loading a `.md` file that has a companion `.toml` or `.yaml` file alongside it,
    /// merges the companion metadata automatically.
    pub fn load_from_path(path: &Path) -> Result<Self> {
        let raw = fs::read_to_string(path)
            .with_context(|| format!("Failed to read template file: {}", path.display()))?;
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();

        match ext.as_str() {
            "toml" => Self::parse_toml(&raw),
            "yaml" | "yml" => Self::parse_yaml(&raw),
            _ => {
                let mut tmpl = Self::parse_markdown(&raw)?;
                // Check for optional companion .toml or .yaml metadata stub
                let companion_toml = path.with_extension("toml");
                let companion_yaml = path.with_extension("yaml");
                if companion_toml.is_file() {
                    if let Ok(raw_toml) = fs::read_to_string(&companion_toml) {
                        if let Ok(meta) = Self::parse_toml(&raw_toml) {
                            tmpl.merge_companion_metadata(meta);
                        }
                    }
                } else if companion_yaml.is_file() {
                    if let Ok(raw_yaml) = fs::read_to_string(&companion_yaml) {
                        if let Ok(meta) = Self::parse_yaml(&raw_yaml) {
                            tmpl.merge_companion_metadata(meta);
                        }
                    }
                }
                Ok(tmpl)
            }
        }
    }

    pub fn parse_toml(raw: &str) -> Result<Self> {
        let parsed: FrontmatterConfig =
            toml::from_str(raw).context("Failed to parse TOML template config")?;
        Ok(parsed.into_prompt_template(String::new()))
    }

    pub fn parse_yaml(raw: &str) -> Result<Self> {
        let parsed: FrontmatterConfig =
            serde_yaml::from_str(raw).context("Failed to parse YAML template config")?;
        Ok(parsed.into_prompt_template(String::new()))
    }

    /// Parses a Markdown template (with optional `---` YAML or `+++` TOML frontmatter),
    /// extracting `/[a | b | c]` command selectors, `[a | b | c]` choice tokens, and `<placeholder>` fields.
    pub fn parse_markdown(raw: &str) -> Result<Self> {
        let (frontmatter, body) = split_frontmatter(raw)?;

        let mut name = "Gemini Agent Prompt Template".to_string();
        let mut description = String::new();
        let mut slash_commands: Vec<String> = Vec::new();
        let mut default_command: Option<String> = None;
        let mut fields: Vec<TemplateFieldSpec> = Vec::new();

        for raw_line in body.lines() {
            let line = raw_line.trim();
            if line.is_empty() {
                continue;
            }

            // Markdown heading `# Title`
            if let Some(heading) = line.strip_prefix('#') {
                let cleaned = heading.trim_start_matches('#').trim();
                if !cleaned.is_empty() {
                    name = cleaned.to_string();
                }
                continue;
            }

            // Line starting with `/[plan | boost | goal] <placeholder>`
            if let Some(rest_after_slash) = line.strip_prefix("/[") {
                if let Some((choices_raw, after_bracket)) = rest_after_slash.split_once(']') {
                    let parsed_cmds = split_pipe_choices(choices_raw);
                    if !parsed_cmds.is_empty() {
                        if default_command.is_none() {
                            default_command = Some(parsed_cmds[0].clone());
                        }
                        for c in parsed_cmds {
                            if !slash_commands.contains(&c) {
                                slash_commands.push(c);
                            }
                        }
                    }
                    let rem = after_bracket.trim();
                    if let Some(placeholder) = extract_outer_angle_placeholder(rem) {
                        let choices = extract_placeholder_choices(&placeholder);
                        fields.push(TemplateFieldSpec {
                            id: "objective".to_string(),
                            label: "Objective".to_string(),
                            placeholder,
                            choices,
                            inline_with_command: true,
                        });
                    }
                    continue;
                }
            }

            // Bullet or key-value line: `- Label: <placeholder>` or `- Label: [a | b | c]`
            let stripped_bullet = line
                .strip_prefix("- ")
                .or_else(|| line.strip_prefix("* "))
                .unwrap_or(line);

            if let Some((label_part, value_part)) = stripped_bullet.split_once(':') {
                let label = label_part.trim().to_string();
                let id = normalize_field_id(&label);
                let val_trimmed = value_part.trim();

                if let Some(placeholder) = extract_outer_angle_placeholder(val_trimmed) {
                    let choices = extract_placeholder_choices(&placeholder);
                    fields.push(TemplateFieldSpec {
                        id,
                        label,
                        placeholder,
                        choices,
                        inline_with_command: false,
                    });
                    continue;
                } else if let Some(bracket_inner) = extract_outer_bracket_choices(val_trimmed) {
                    let choices = split_pipe_choices(&bracket_inner);
                    fields.push(TemplateFieldSpec {
                        id,
                        label,
                        placeholder: bracket_inner,
                        choices,
                        inline_with_command: false,
                    });
                    continue;
                }
            }

            // Standalone `<placeholder>` line
            if let Some(placeholder) = extract_outer_angle_placeholder(stripped_bullet) {
                let choices = extract_placeholder_choices(&placeholder);
                let idx = fields.len() + 1;
                let (id, label) = if fields.is_empty() {
                    ("objective".to_string(), "Objective".to_string())
                } else {
                    (format!("field_{}", idx), format!("Field {}", idx))
                };
                fields.push(TemplateFieldSpec {
                    id,
                    label,
                    placeholder,
                    choices,
                    inline_with_command: false,
                });
            }
        }

        // Apply frontmatter overrides if present
        if let Some(fm) = frontmatter {
            if let Some(fm_name) = fm.name {
                name = fm_name;
            }
            if let Some(fm_desc) = fm.description {
                description = fm_desc;
            }
            if !fm.commands.is_empty() {
                slash_commands = fm.commands;
            }
            if fm.default_command.is_some() {
                default_command = fm.default_command;
            }
            if !fm.fields.is_empty() {
                fields = fm.fields;
            }
        }

        Ok(Self {
            name,
            description,
            slash_commands,
            default_command,
            fields,
            raw_markdown: body.to_string(),
        })
    }

    fn merge_companion_metadata(&mut self, companion: Self) {
        if !companion.name.is_empty() {
            self.name = companion.name;
        }
        if !companion.description.is_empty() {
            self.description = companion.description;
        }
        if !companion.slash_commands.is_empty() {
            self.slash_commands = companion.slash_commands;
        }
        if companion.default_command.is_some() {
            self.default_command = companion.default_command;
        }
        if !companion.fields.is_empty() {
            self.fields = companion.fields;
        }
    }

    /// Returns the complete deduplicated list of slash commands combining the template's own
    /// commands, all built-in `agy` slash commands (`BUILTIN_AGY_SLASH_COMMANDS`), and any
    /// skills/workflows discovered on disk.
    pub fn all_slash_commands(
        &self,
        cwd: Option<&Path>,
        extra_roots: Option<&[PathBuf]>,
    ) -> Vec<String> {
        let mut combined: Vec<String> = Vec::new();
        for cmd in &self.slash_commands {
            push_unique_slash_cmd(&mut combined, cmd);
        }
        for cmd in discover_agy_slash_commands(cwd, extra_roots) {
            push_unique_slash_cmd(&mut combined, &cmd);
        }
        combined
    }
}

fn push_unique_slash_cmd(list: &mut Vec<String>, raw: &str) {
    let Some(norm) = normalize_slash_command_slug(raw) else {
        return;
    };
    if !list.iter().any(|existing| existing.eq_ignore_ascii_case(&norm)) {
        list.push(norm);
    }
}

/// Normalizes a slash command or skill/workflow name into a clean lowercase slug (without leading `/`).
pub fn normalize_slash_command_slug(raw: &str) -> Option<String> {
    let trimmed = raw
        .trim()
        .trim_matches(|c: char| c == '"' || c == '\'' || c == '`')
        .trim_start_matches('/');
    if trimmed.is_empty() {
        return None;
    }
    let mut slug = String::with_capacity(trimmed.len());
    let mut prev_dash = false;
    for ch in trimmed.chars() {
        if ch.is_ascii_alphanumeric() {
            slug.push(ch.to_ascii_lowercase());
            prev_dash = false;
        } else if (ch == '-' || ch == '_' || ch == ' ') && !slug.is_empty() && !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    while slug.ends_with('-') {
        slug.pop();
    }
    if slug.is_empty()
        || matches!(
            slug.as_str(),
            "none" | "skill" | "skills" | "workflow" | "workflows" | "readme" | "instructions"
        )
    {
        return None;
    }
    Some(slug)
}

/// Dynamically discovers all built-in `agy` slash commands plus installed skills and workflows
/// from `~/.gemini/antigravity/builtin/skills/`, `~/.gemini/skills/`, `~/.gemini/workflows/`,
/// `./.gemini/`, `./.agent/`, and any `extra_roots`.
pub fn discover_agy_slash_commands(
    cwd: Option<&Path>,
    extra_roots: Option<&[PathBuf]>,
) -> Vec<String> {
    let mut commands: Vec<String> = Vec::new();
    for builtin in BUILTIN_AGY_SLASH_COMMANDS {
        push_unique_slash_cmd(&mut commands, builtin);
    }

    let mut search_roots: Vec<PathBuf> = Vec::new();
    if let Some(home) = dirs::home_dir() {
        search_roots.push(home.join(".gemini/antigravity/builtin/skills"));
        search_roots.push(home.join(".gemini/skills"));
        search_roots.push(home.join(".gemini/workflows"));
    }
    if let Some(dir) = cwd {
        search_roots.push(dir.join(".gemini"));
        search_roots.push(dir.join(".agent"));
    }
    if let Some(extras) = extra_roots {
        for r in extras {
            search_roots.push(r.clone());
        }
    }

    for root in search_roots {
        scan_skill_or_workflow_root(&root, &mut commands);
    }

    commands
}

fn scan_skill_or_workflow_root(root: &Path, commands: &mut Vec<String>) {
    if !root.is_dir() {
        return;
    }

    let root_name = root
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let is_skills_or_workflows_dir =
        root_name == "skills" || root_name == "workflows" || root_name.ends_with("_skills");

    // If this directory contains nested `skills` or `workflows` subdirectories (e.g. `.gemini/skills`), scan them too.
    for sub in ["skills", "workflows"] {
        let nested = root.join(sub);
        if nested.is_dir() {
            scan_skill_or_workflow_dir_entries(&nested, true, commands);
        }
    }

    scan_skill_or_workflow_dir_entries(root, is_skills_or_workflows_dir, commands);
}

fn scan_skill_or_workflow_dir_entries(
    dir: &Path,
    allow_md_stems_and_bare_dirs: bool,
    commands: &mut Vec<String>,
) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    let mut entries: Vec<PathBuf> = read_dir
        .filter_map(|e| e.ok().map(|entry| entry.path()))
        .collect();
    entries.sort();

    for path in entries {
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if file_name.starts_with('.') {
            continue;
        }

        if path.is_dir() {
            let skill_md = path.join("SKILL.md");
            if skill_md.is_file() {
                if let Some(fm_name) = extract_skill_md_name(&skill_md) {
                    push_unique_slash_cmd(commands, &fm_name);
                } else {
                    push_unique_slash_cmd(commands, file_name);
                }
            } else if allow_md_stems_and_bare_dirs
                && !matches!(
                    file_name.to_ascii_lowercase().as_str(),
                    "skills" | "workflows" | "scripts" | "resources" | "examples" | "references"
                )
            {
                push_unique_slash_cmd(commands, file_name);
            }
        } else if allow_md_stems_and_bare_dirs && path.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            if ext == "md" || ext == "toml" || ext == "yaml" || ext == "yml" {
                if let Some(stem) = path.file_stem().and_then(|s| s.to_str()) {
                    push_unique_slash_cmd(commands, stem);
                }
            }
        }
    }
}

fn extract_skill_md_name(skill_md_path: &Path) -> Option<String> {
    let raw = fs::read_to_string(skill_md_path).ok()?;
    let trimmed = raw.trim_start_matches('\u{feff}');
    let after_open = trimmed
        .strip_prefix("---\n")
        .or_else(|| trimmed.strip_prefix("---\r\n"))?;
    let end_idx = after_open.find("\n---")?;
    let fm_block = &after_open[..end_idx];
    for line in fm_block.lines() {
        if let Some((k, v)) = line.split_once(':') {
            if k.trim().eq_ignore_ascii_case("name") {
                return normalize_slash_command_slug(v);
            }
        }
    }
    None
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct FrontmatterConfig {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default)]
    pub default_command: Option<String>,
    #[serde(default)]
    pub commands: Vec<String>,
    #[serde(default)]
    pub fields: Vec<TemplateFieldSpec>,
}

impl FrontmatterConfig {
    fn into_prompt_template(self, raw_markdown: String) -> PromptTemplate {
        let default_command = self
            .default_command
            .or_else(|| self.commands.first().cloned());
        PromptTemplate {
            name: self
                .name
                .unwrap_or_else(|| "Custom Prompt Template".to_string()),
            description: self.description.unwrap_or_default(),
            slash_commands: self.commands,
            default_command,
            fields: self.fields,
            raw_markdown,
        }
    }
}

/// Splits optional YAML (`---`) or TOML (`+++`) frontmatter from the Markdown body.
fn split_frontmatter(raw: &str) -> Result<(Option<FrontmatterConfig>, &str)> {
    let trimmed = raw.trim_start_matches('\u{feff}');
    if let Some(after_open) = trimmed.strip_prefix("---\n").or_else(|| trimmed.strip_prefix("---\r\n")) {
        if let Some(end_idx) = after_open.find("\n---") {
            let fm_str = &after_open[..end_idx];
            let rest = &after_open[end_idx + 4..];
            let body = rest.strip_prefix('\r').unwrap_or(rest);
            let body = body.strip_prefix('\n').unwrap_or(body);
            let cfg: FrontmatterConfig =
                serde_yaml::from_str(fm_str).context("Invalid YAML frontmatter in template")?;
            return Ok((Some(cfg), body));
        }
    } else if let Some(after_open) = trimmed.strip_prefix("+++\n").or_else(|| trimmed.strip_prefix("+++\r\n")) {
        if let Some(end_idx) = after_open.find("\n+++") {
            let fm_str = &after_open[..end_idx];
            let rest = &after_open[end_idx + 4..];
            let body = rest.strip_prefix('\r').unwrap_or(rest);
            let body = body.strip_prefix('\n').unwrap_or(body);
            let cfg: FrontmatterConfig =
                toml::from_str(fm_str).context("Invalid TOML frontmatter in template")?;
            return Ok((Some(cfg), body));
        }
    }
    Ok((None, raw))
}

/// Normalizes a field label such as `"Context/Logs"` into a canonical key `"context_logs"`.
pub fn normalize_field_id(label: &str) -> String {
    let mut out = String::with_capacity(label.len());
    let mut prev_underscore = false;
    for ch in label.trim().chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
            prev_underscore = false;
        } else if !prev_underscore && !out.is_empty() {
            out.push('_');
            prev_underscore = true;
        }
    }
    while out.ends_with('_') {
        out.pop();
    }
    out
}

/// Extracts the content inside the outermost `<...>` on a line, preserving nested `<file>.md` tokens.
pub fn extract_outer_angle_placeholder(s: &str) -> Option<String> {
    let start = s.find('<')?;
    let end = s.rfind('>')?;
    if end > start + 1 {
        Some(s[start + 1..end].trim().to_string())
    } else {
        None
    }
}

/// Extracts the content inside `[...]` choice brackets.
pub fn extract_outer_bracket_choices(s: &str) -> Option<String> {
    let start = s.find('[')?;
    let end = s.rfind(']')?;
    if end > start + 1 {
        Some(s[start + 1..end].trim().to_string())
    } else {
        None
    }
}

/// Splits pipe-delimited options (`a | b | c`) into trimmed non-empty strings.
pub fn split_pipe_choices(raw: &str) -> Vec<String> {
    raw.split('|')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// If a `<...>` placeholder contains top-level `|` choices (such as
/// `Diagnose only | Update <file>.md plan | Execute Step X`), returns those options.
pub fn extract_placeholder_choices(placeholder: &str) -> Vec<String> {
    if !placeholder.contains('|') {
        return Vec::new();
    }
    let choices = split_pipe_choices(placeholder);
    if choices.len() >= 2 {
        choices
    } else {
        Vec::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_gemini_agent_prompt_template_md() {
        let tmpl = PromptTemplate::parse_markdown(DEFAULT_GEMINI_TEMPLATE_MD)
            .expect("should parse GeminiAgentPromptTemplate.md");
        assert_eq!(tmpl.name, "Gemini Agent Prompt Template");
        assert_eq!(tmpl.slash_commands, vec!["plan", "boost", "goal"]);
        assert_eq!(tmpl.default_command.as_deref(), Some("plan"));
        assert_eq!(tmpl.fields.len(), 4);

        assert_eq!(tmpl.fields[0].id, "objective");
        assert_eq!(tmpl.fields[0].placeholder, "1-2 sentence symptom or objective");
        assert!(tmpl.fields[0].inline_with_command);

        assert_eq!(tmpl.fields[1].id, "context_logs");
        assert_eq!(tmpl.fields[1].label, "Context/Logs");
        assert_eq!(tmpl.fields[1].placeholder, "file paths or terminal snippet");

        assert_eq!(tmpl.fields[2].id, "constraints");
        assert!(tmpl.fields[2].placeholder.contains("No QEMU"));

        assert_eq!(tmpl.fields[3].id, "output");
        assert_eq!(
            tmpl.fields[3].choices,
            vec!["Diagnose only", "Update <file>.md plan", "Execute Step X"]
        );
    }

    #[test]
    fn test_yaml_and_toml_frontmatter_parsing() {
        let md_with_yaml = r#"---
name: "Custom Incident Template"
default_command: "boost"
commands: ["boost", "goal"]
---
/[boost | goal] <Investigate outage>
- Context/Logs: <journalctl logs>
- Constraints: <Read-only diagnostic mode>
- Output: <Root cause report>
"#;
        let parsed = PromptTemplate::parse_markdown(md_with_yaml).expect("YAML frontmatter valid");
        assert_eq!(parsed.name, "Custom Incident Template");
        assert_eq!(parsed.default_command.as_deref(), Some("boost"));
        assert_eq!(parsed.slash_commands, vec!["boost", "goal"]);
        assert_eq!(parsed.fields.len(), 4);
    }

    #[test]
    fn test_discover_agy_slash_commands_includes_builtins_and_custom_skills() {
        let tmp_dir =
            std::env::temp_dir().join(format!("verbot_skill_scan_test_{}", std::process::id()));
        let _ = fs::remove_dir_all(&tmp_dir);
        let gemini_skills = tmp_dir.join(".gemini/skills/custom-audit");
        let agent_workflows = tmp_dir.join(".agent/workflows");
        fs::create_dir_all(&gemini_skills).unwrap();
        fs::create_dir_all(&agent_workflows).unwrap();

        fs::write(
            gemini_skills.join("SKILL.md"),
            "---\nname: custom-audit\ndescription: test\n---\n# Custom Audit\n",
        )
        .unwrap();
        fs::write(agent_workflows.join("release-gate.md"), "# Release Gate\n").unwrap();

        let tmpl = PromptTemplate::default();
        let all_cmds = tmpl.all_slash_commands(Some(&tmp_dir), None);
        for builtin in BUILTIN_AGY_SLASH_COMMANDS {
            assert!(
                all_cmds.contains(&builtin.to_string()),
                "Missing built-in agy slash command: {}",
                builtin
            );
        }
        assert!(all_cmds.contains(&"custom-audit".to_string()));
        assert!(all_cmds.contains(&"release-gate".to_string()));

        let _ = fs::remove_dir_all(&tmp_dir);
    }
}
