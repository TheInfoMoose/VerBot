use crate::state::{local_verbot_templates_dir, GlobalConfig};
use crate::template::{PromptTemplate, TemplateFieldSpec};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Options controlling `verbot ingest <path>`.
#[derive(Debug, Clone)]
pub struct IngestOptions {
    /// Source template file path (`.md`, `.toml`, `.yaml`, `.yml`).
    pub source_path: PathBuf,
    /// If true, imports into `<cwd>/.verbot/templates/` instead of `~/.config/verbot/templates/`.
    pub local: bool,
    /// Optional explicit working directory when `local` is true (defaults to `std::env::current_dir()`).
    pub cwd: Option<PathBuf>,
    /// Optional override for the global templates directory (used for isolated testing).
    pub custom_global_templates_dir: Option<PathBuf>,
    /// Optional custom slug/filename stem for the ingested template.
    pub custom_name: Option<String>,
    /// Validate and preview the ingestion without writing files to disk.
    pub dry_run: bool,
}

/// Result returned by [`ingest_template`].
#[derive(Debug, Clone)]
pub struct IngestReport {
    pub parsed_template: PromptTemplate,
    pub target_dir: PathBuf,
    pub primary_dest_path: PathBuf,
    pub companion_toml_path: Option<PathBuf>,
    pub companion_toml_content: Option<String>,
    pub dry_run: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct CompanionMetadataStub {
    pub name: String,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub default_command: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub commands: Vec<String>,
    pub fields: Vec<TemplateFieldSpec>,
}

/// Validates and imports a `.md`, `.toml`, or `.yaml`/`.yml` template file into
/// `~/.config/verbot/templates/` (or `./.verbot/templates/` when `opts.local` is true).
///
/// When a bare `.md` file is provided, generates a companion `.toml` metadata stub alongside
/// the imported `.md` file so users can customize commands, placeholders, or model defaults.
pub fn ingest_template(opts: &IngestOptions) -> Result<IngestReport> {
    ensure!(
        opts.source_path.is_file(),
        "Template source file does not exist: {}",
        opts.source_path.display()
    );

    let ext = opts
        .source_path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();

    match ext.as_str() {
        "md" | "markdown" | "toml" | "yaml" | "yml" => {}
        other => bail!(
            "Unsupported template extension '.{}' (expected .md, .toml, .yaml, or .yml)",
            other
        ),
    }

    let parsed_template = PromptTemplate::load_from_path(&opts.source_path)
        .with_context(|| format!("Failed to validate template at {}", opts.source_path.display()))?;

    ensure!(
        !parsed_template.fields.is_empty(),
        "Template '{}' at {} did not define or yield any prompt fields (<placeholder> or fields list)",
        parsed_template.name,
        opts.source_path.display()
    );

    let target_dir = resolve_target_templates_dir(opts)?;

    let stem = opts
        .custom_name
        .as_deref()
        .map(sanitize_template_stem)
        .or_else(|| {
            opts.source_path
                .file_stem()
                .and_then(|s| s.to_str())
                .map(sanitize_template_stem)
        })
        .unwrap_or_else(|| "custom_template".to_string());

    let primary_dest_path = target_dir.join(format!("{}.{}", stem, ext));

    // If a `.md` / `.markdown` template was provided, prepare the companion `.toml` metadata stub.
    let (companion_toml_path, companion_toml_content) =
        if ext == "md" || ext == "markdown" {
            let companion_path = target_dir.join(format!("{}.toml", stem));
            let stub_content = generate_companion_toml_stub(&parsed_template)?;
            (Some(companion_path), Some(stub_content))
        } else {
            (None, None)
        };

    if !opts.dry_run {
        fs::create_dir_all(&target_dir).with_context(|| {
            format!(
                "Failed to create target templates directory {}",
                target_dir.display()
            )
        })?;

        let raw_source = fs::read_to_string(&opts.source_path).with_context(|| {
            format!(
                "Failed to read source template {}",
                opts.source_path.display()
            )
        })?;
        fs::write(&primary_dest_path, raw_source).with_context(|| {
            format!(
                "Failed to write ingested template to {}",
                primary_dest_path.display()
            )
        })?;

        if let (Some(ref comp_path), Some(ref comp_body)) =
            (&companion_toml_path, &companion_toml_content)
        {
            // If the source `.md` already had a companion `.toml` next to it, copy that; otherwise write generated stub.
            let source_companion = opts.source_path.with_extension("toml");
            if source_companion.is_file() && source_companion != opts.source_path {
                let existing_toml = fs::read_to_string(&source_companion)?;
                fs::write(comp_path, existing_toml)?;
            } else {
                fs::write(comp_path, comp_body).with_context(|| {
                    format!(
                        "Failed to write companion TOML metadata stub to {}",
                        comp_path.display()
                    )
                })?;
            }
        }
    }

    Ok(IngestReport {
        parsed_template,
        target_dir,
        primary_dest_path,
        companion_toml_path,
        companion_toml_content,
        dry_run: opts.dry_run,
    })
}

/// Generates a human-friendly TOML metadata stub for a parsed Markdown [`PromptTemplate`].
pub fn generate_companion_toml_stub(template: &PromptTemplate) -> Result<String> {
    let stub = CompanionMetadataStub {
        name: template.name.clone(),
        description: if template.description.is_empty() {
            format!(
                "Companion metadata stub generated by `verbot ingest` for '{}'",
                template.name
            )
        } else {
            template.description.clone()
        },
        default_command: template.default_command.clone(),
        commands: template.slash_commands.clone(),
        fields: template.fields.clone(),
    };

    let body = toml::to_string_pretty(&stub)
        .context("Failed to serialize companion metadata stub to TOML")?;
    Ok(format!(
        "# Verbocity (verbot) Companion Template Metadata\n# Edit commands, labels, or choices below to customize TUI selectors.\n\n{}",
        body
    ))
}

/// Lists all available ingested templates across project-local `.verbot/templates/` (if present)
/// and global `~/.config/verbot/templates/`.
pub fn discover_ingested_templates(
    cwd: &Path,
    custom_global_dir: Option<&Path>,
) -> Vec<(PathBuf, PromptTemplate)> {
    let mut results = Vec::new();
    let mut dirs_to_check = Vec::new();

    let local_dir = local_verbot_templates_dir(cwd);
    if local_dir.is_dir() {
        dirs_to_check.push(local_dir);
    }

    if let Some(custom) = custom_global_dir {
        if custom.is_dir() {
            dirs_to_check.push(custom.to_path_buf());
        }
    } else if let Some(global_dir) = GlobalConfig::default_templates_dir() {
        if global_dir.is_dir() {
            dirs_to_check.push(global_dir);
        }
    }

    for dir in dirs_to_check {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        let mut paths: Vec<PathBuf> = entries
            .filter_map(|e| e.ok().map(|ent| ent.path()))
            .filter(|p| p.is_file())
            .collect();
        paths.sort();

        for path in paths {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .unwrap_or("")
                .to_ascii_lowercase();
            // If both `foo.md` and `foo.toml` exist in the same directory, load `foo.md` (which merges `foo.toml` automatically)
            if (ext == "toml" || ext == "yaml" || ext == "yml")
                && path.with_extension("md").is_file()
            {
                continue;
            }
            if matches!(ext.as_str(), "md" | "markdown" | "toml" | "yaml" | "yml") {
                if let Ok(tmpl) = PromptTemplate::load_from_path(&path) {
                    results.push((path, tmpl));
                }
            }
        }
    }

    results
}

fn resolve_target_templates_dir(opts: &IngestOptions) -> Result<PathBuf> {
    if opts.local {
        let base_cwd = match &opts.cwd {
            Some(c) => c.clone(),
            None => std::env::current_dir().context("Failed to determine current working directory")?,
        };
        Ok(local_verbot_templates_dir(&base_cwd))
    } else if let Some(ref custom) = opts.custom_global_templates_dir {
        Ok(custom.clone())
    } else {
        Ok(GlobalConfig::default_templates_dir()
            .unwrap_or_else(|| std::env::temp_dir().join("verbot-config/templates")))
    }
}

fn sanitize_template_stem(raw: &str) -> String {
    let trimmed = raw.trim();
    let cleaned: String = trimmed
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if cleaned.is_empty() {
        "template".to_string()
    } else {
        cleaned
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_ingest_bare_markdown_generates_companion_toml_and_respects_dry_run() {
        let tmp_root = std::env::temp_dir().join(format!(
            "verbot_ingest_test_{}",
            std::process::id()
        ));
        let fake_cwd = tmp_root.join("workspace");
        let fake_global = tmp_root.join("global_templates");
        fs::create_dir_all(&fake_cwd).unwrap();

        let repo_template =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("templates/GeminiAgentPromptTemplate.md");

        // 1. Dry-run ingestion into global templates dir
        let dry_opts = IngestOptions {
            source_path: repo_template.clone(),
            local: false,
            cwd: Some(fake_cwd.clone()),
            custom_global_templates_dir: Some(fake_global.clone()),
            custom_name: None,
            dry_run: true,
        };
        let dry_report = ingest_template(&dry_opts).expect("dry-run ingest should succeed");
        assert!(dry_report.dry_run);
        assert!(!fake_global.exists(), "dry-run must not create target directory");
        assert!(!fake_cwd.join(".verbot").exists(), "must not create .verbot in CWD");
        assert!(dry_report.companion_toml_content.as_ref().unwrap().contains("Gemini Agent Prompt Template"));

        // 2. Real ingestion into global templates dir (still does not pollute CWD with `.verbot/`)
        let write_opts = IngestOptions {
            dry_run: false,
            ..dry_opts
        };
        let write_report = ingest_template(&write_opts).expect("global ingest should succeed");
        assert!(write_report.primary_dest_path.is_file());
        assert!(write_report.companion_toml_path.as_ref().unwrap().is_file());
        assert!(!fake_cwd.join(".verbot").exists(), "global ingest must not create .verbot in CWD");

        let discovered = discover_ingested_templates(&fake_cwd, Some(&fake_global));
        assert_eq!(discovered.len(), 1);
        assert_eq!(discovered[0].1.name, "Gemini Agent Prompt Template");
        assert_eq!(discovered[0].1.fields.len(), 4);

        // 3. Explicit `--local` ingestion creates `<cwd>/.verbot/templates/`
        let local_opts = IngestOptions {
            source_path: repo_template,
            local: true,
            cwd: Some(fake_cwd.clone()),
            custom_global_templates_dir: Some(fake_global.clone()),
            custom_name: Some("local_agent_prompt".to_string()),
            dry_run: false,
        };
        let local_report = ingest_template(&local_opts).expect("local ingest should succeed");
        assert!(local_report.primary_dest_path.starts_with(fake_cwd.join(".verbot/templates")));
        assert!(local_report.primary_dest_path.is_file());
        assert!(local_report.companion_toml_path.as_ref().unwrap().is_file());

        let _ = fs::remove_dir_all(&tmp_root);
    }
}
