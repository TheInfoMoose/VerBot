use anyhow::{ensure, Context, Result};
use clap::{Parser, Subcommand};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use std::io::IsTerminal;
use std::path::PathBuf;
use verbot_tui::app::{App, DispatchAction, DispatchManifest, FocusPane};
use verbot_tui::branding::{
    render_banner_lines, render_banner_plain, select_banner_tier, BannerTier,
};
use verbot_tui::files::WorkspaceFileIndex;
use verbot_tui::formatting::{
    inspect_prompt_values_safety, parse_posix_shell_words, AgyExecutionMode, ShellFormattingPolicy,
};
use verbot_tui::ingest::{discover_ingested_templates, ingest_template, IngestOptions};
use verbot_tui::models::{
    build_agy_command_preview, estimate_prompt_metrics, parse_agy_models_output, AgyLaunchMode,
    EffortLevel, ModelCatalog,
};
use verbot_tui::settings::SettingsTab;
use verbot_tui::state::{GlobalConfig, WorkspaceState};
use verbot_tui::template::{
    PromptTemplate, PromptValues, BUILTIN_AGY_SLASH_COMMANDS, DEFAULT_GEMINI_TEMPLATE_MD,
};
use verbot_tui::theme::{novos_palette, parse_ron_rgb, RgbColor, Theme, ThemePreset};
use verbot_tui::tui::{
    buffer_to_string, compute_field_editor_rects_for_terminal, render_headless_buffer,
    run_interactive_tui, wrap_editor_lines_with_cursor, MIN_FIELD_EDITOR_HEIGHT,
    MIN_FIELD_EDITOR_WIDTH, MIN_FIELD_INNER_TEXT_ROWS,
};

#[derive(Debug, Parser)]
#[command(
    name = "verbot",
    version,
    about = "Verbocity (verbot) — Interactive terminal LLM prompt template engine and agy dispatcher"
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,

    /// Run automated self-test verification suite for branding, NovOS/COSMIC theme, template parser, state, ingest, CWD scanner, and headless Ratatui TUI
    #[arg(long)]
    self_test: bool,

    /// Optional path to a template file (.md, .toml, .yaml)
    #[arg(short, long)]
    template: Option<PathBuf>,

    /// Pre-select or immediately dispatch `agy -i` interactive mode
    #[arg(short = 'i', long = "interactive")]
    interactive: bool,

    /// Pre-select or immediately dispatch `agy -p` print mode
    #[arg(short = 'p', long = "print")]
    print: bool,

    /// Launch Verbocity directly onto the full-screen Settings Page (`Alt+S`)
    #[arg(short = 's', long = "settings")]
    settings: bool,

    /// Set the active `agy` slash command (e.g., `plan`, `boost`, `goal`, `grill-me`, `none`)
    #[arg(short = 'c', long = "command", alias = "slash-command")]
    slash_command: Option<String>,

    /// Target model family, short name, or full `agy` model ID (e.g., gemini-4-argon-medium, Argon, Flash, Opus, Sonnet, GPT-OSS 120B)
    #[arg(short = 'm', long = "model")]
    model: Option<String>,

    /// Reasoning / thinking effort level (low, medium, high, xhigh, max)
    #[arg(short = 'e', long = "effort")]
    effort: Option<String>,

    /// Pass execution mode (`auto`, `plan`, `accept-edits`, `none`) to `agy`
    #[arg(long = "mode")]
    mode: Option<String>,

    /// Enable `--sandbox` for `agy`
    #[arg(long = "sandbox")]
    sandbox: bool,

    /// Pass `--continue` to `agy` to resume the most recent conversation
    #[arg(long = "continue")]
    continue_session: bool,

    /// Ingest a model `.toml` or `.yaml` file directly into `~/.config/verbot/models.toml`
    #[arg(long = "ingest-model")]
    ingest_model: Option<PathBuf>,

    /// Active theme preset (NovOS Cosmic Dark, COSMIC System Auto, NovOS Solar Light, Terminal ANSI)
    #[arg(long)]
    theme: Option<String>,

    /// Opt-in project-local state storage in `./.verbot/state.toml` (default uses XDG data cache)
    #[arg(long)]
    local: bool,

    /// Optional file containing captured piped stdin to pre-populate the Context/Logs field
    #[arg(long)]
    stdin_file: Option<PathBuf>,

    /// Optional path where the structured JSON DispatchManifest is written on exit
    #[arg(long)]
    dispatch_out: Option<PathBuf>,

    /// Run in non-interactive headless mode (renders prompt and executes/emits dispatch manifest immediately)
    #[arg(long)]
    headless: bool,

    /// Dispatch action for headless mode (`agy-interactive`, `agy-print`, `stdout`, `save-md`)
    #[arg(long, default_value = "stdout")]
    action: String,

    /// Optional path for saving the rendered prompt as Markdown (`verbot-prompt.md` by default)
    #[arg(long)]
    save_md: Option<PathBuf>,

    /// Optional Objective field override
    #[arg(long)]
    objective: Option<String>,

    /// Optional Context/Logs field override
    #[arg(long)]
    context: Option<String>,

    /// Optional Constraints field override
    #[arg(long)]
    constraints: Option<String>,

    /// Optional Output field override
    #[arg(long)]
    output: Option<String>,

    /// Trailing positional arguments captured into the Objective field (e.g. `verbot -c plan Fix scaling on branding`)
    #[arg(trailing_var_arg = true, allow_hyphen_values = false)]
    objective_args: Vec<String>,
}

#[derive(Debug, Subcommand)]
enum Commands {
    /// Validate and ingest a .md, .toml, or .yaml prompt template into ~/.config/verbot/templates/ (or ./.verbot/templates/ with --local)
    Ingest {
        /// Path to the template file (.md, .toml, .yaml)
        path: PathBuf,

        /// Store the ingested template in project-local ./.verbot/templates/ instead of ~/.config/verbot/templates/
        #[arg(long)]
        local: bool,

        /// Optional custom template stem name
        #[arg(short, long)]
        name: Option<String>,

        /// Validate template and preview companion TOML metadata without writing files to disk
        #[arg(long)]
        dry_run: bool,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();

    if cli.self_test {
        run_self_test()?;
        return Ok(());
    }

    if let Some(Commands::Ingest {
        path,
        local,
        name,
        dry_run,
    }) = cli.command
    {
        let opts = IngestOptions {
            source_path: path,
            local: local || cli.local,
            cwd: None,
            custom_global_templates_dir: None,
            custom_name: name,
            dry_run,
        };
        let report = ingest_template(&opts)?;
        if report.dry_run {
            println!(
                "==> [Verbocity Ingest Dry-Run] Validated template '{}' ({} fields, commands: {:?})",
                report.parsed_template.name,
                report.parsed_template.fields.len(),
                report.parsed_template.slash_commands
            );
            println!(
                "  Target Template Path: {}",
                report.primary_dest_path.display()
            );
            if let Some(ref comp_path) = report.companion_toml_path {
                println!("  Companion Metadata:   {}", comp_path.display());
            }
            if let Some(ref stub) = report.companion_toml_content {
                println!("\n--- Generated Companion .toml Preview ---\n{}", stub.trim());
            }
        } else {
            println!(
                "==> [Verbocity Ingest] Imported '{}' -> {}",
                report.parsed_template.name,
                report.primary_dest_path.display()
            );
            if let Some(ref comp_path) = report.companion_toml_path {
                println!("  Generated companion metadata: {}", comp_path.display());
            }
        }
        return Ok(());
    }

    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let global_cfg = GlobalConfig::load_or_default(None);
    let (workspace_state, state_location) =
        WorkspaceState::load_for_dir(&cwd, None, Some(&global_cfg))?;

    let theme = match cli.theme.as_deref() {
        Some(name) => Theme::from_preset(ThemePreset::from_name(name)),
        None => workspace_state.resolve_theme(None),
    };

    let template = match cli.template.as_deref() {
        Some(path) => PromptTemplate::load_from_path(path)?,
        None => PromptTemplate::default(),
    };

    let mut catalog = ModelCatalog::load_user_or_builtin(None);
    if let Some(ref model_toml_path) = cli.ingest_model {
        let resolved = if model_toml_path.is_absolute() {
            model_toml_path.clone()
        } else {
            cwd.join(model_toml_path)
        };
        let ingested = catalog.ingest_model_file(&resolved, None)?;
        let names: Vec<String> = ingested
            .iter()
            .map(|p| format!("{} ({})", p.display_name, p.agy_base_id))
            .collect();
        if !cli.headless {
            eprintln!(
                "==> [Verbocity Model Ingest] Imported {} model profile(s) from {}: {}",
                ingested.len(),
                resolved.display(),
                names.join(", ")
            );
        }
        // If only `--ingest-model` was passed without interactive/headless prompt args, exit cleanly when non-interactive
        if !cli.headless
            && !cli.interactive
            && !cli.print
            && !cli.settings
            && cli.objective.is_none()
            && cli.objective_args.is_empty()
            && !std::io::stdout().is_terminal()
        {
            return Ok(());
        }
    }

    let file_index = WorkspaceFileIndex::scan(&cwd)?;

    let mut app = App::new(
        cwd.clone(),
        template,
        catalog,
        workspace_state,
        state_location,
        theme,
        file_index,
        cli.local,
    );

    if let Some(ref model_toml_path) = cli.ingest_model {
        let resolved = if model_toml_path.is_absolute() {
            model_toml_path.clone()
        } else {
            cwd.join(model_toml_path)
        };
        if let Ok(ingested) = app.ingest_model_file_from_settings(&resolved) {
            if let Some(first) = ingested.first() {
                app.select_model_by_query(&first.short_name);
            }
        }
    }

    if let Some(ref model_query) = cli.model {
        app.select_model_by_query(model_query);
    }
    if let Some(ref effort_str) = cli.effort {
        app.selected_effort = EffortLevel::from_str_loose(effort_str);
        app.explicit_agy_model_id = None;
    }
    if let Some(ref mode_str) = cli.mode {
        app.global_config.agy_flags.mode = AgyExecutionMode::from_str_loose(mode_str);
    }
    if cli.sandbox {
        app.global_config.agy_flags.sandbox = true;
    }
    if cli.continue_session {
        app.global_config.agy_flags.continue_session = true;
    }
    if let Some(ref save_md) = cli.save_md {
        app.saved_markdown_path = Some(save_md.clone());
    }

    // Populate initial Context/Logs from `--stdin-file` if provided
    if let Some(ref stdin_path) = cli.stdin_file {
        if stdin_path.is_file() {
            if let Ok(captured) = std::fs::read_to_string(stdin_path) {
                app.set_initial_context_logs(&captured);
            }
        }
    }

    // Apply optional CLI field overrides & trailing positional objective args
    if let Some(ref cmd) = cli.slash_command {
        if cmd.eq_ignore_ascii_case("none") || cmd.trim().is_empty() {
            app.values.slash_command = None;
        } else {
            app.values.slash_command = Some(cmd.trim_start_matches('/').to_string());
        }
    }
    if let Some(ref obj) = cli.objective {
        app.values.objective = obj.clone();
    }
    if !cli.objective_args.is_empty() {
        let trailing_obj = cli.objective_args.join(" ");
        if app.values.objective.trim().is_empty() {
            app.values.objective = trailing_obj;
        } else {
            app.values.objective = format!("{} {}", app.values.objective.trim(), trailing_obj);
        }
    }
    if let Some(ref ctx) = cli.context {
        if app.values.context_logs.is_empty() {
            app.values.context_logs = ctx.clone();
        } else {
            app.values.context_logs = format!("{}\n{}", ctx, app.values.context_logs);
        }
    }
    if let Some(ref cons) = cli.constraints {
        app.values.constraints = app.workspace_state.apply_pinned_constraints(cons);
    }
    if let Some(ref out) = cli.output {
        app.values.output = out.clone();
    }
    app.sync_editors_from_values();

    if cli.settings {
        app.open_settings_page();
    }

    let requested_action = if cli.interactive {
        DispatchAction::AgyInteractive
    } else if cli.print {
        DispatchAction::AgyPrint
    } else {
        DispatchAction::from_slug(&cli.action).unwrap_or(DispatchAction::StdoutClipboard)
    };
    app.selected_action = requested_action;

    if cli.headless || !std::io::stdout().is_terminal() {
        let manifest = app.confirm_dispatch(requested_action)?;
        if let Some(ref out_path) = cli.dispatch_out {
            manifest.write_to_path(out_path)?;
        } else if requested_action == DispatchAction::StdoutClipboard {
            println!("{}", manifest.rendered_prompt);
        } else {
            println!("{}", manifest.agy_command_preview);
        }
        return Ok(());
    }

    // In interactive mode, `run_interactive_tui` handles all actions in-app (TerminalOutputDrawer +
    // suspend/resume handoff for `agy -i`) so Verbocity stays open until the user presses `Ctrl+Q`.
    let _ = run_interactive_tui(&mut app)?;

    Ok(())
}

/// Resolves a modular dispatch script (`dispatch-agy.sh` or `dispatch-output.sh`) across installed
/// prefix locations, FHS system paths (`/usr/local/share/verbot/scripts`, `/usr/share/verbot/scripts`),
/// repository root, or CWD.
fn resolve_dispatch_script(script_name: &str, cwd: &std::path::Path) -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    candidates.push(cwd.join("scripts").join(script_name));
    if let Ok(exe) = std::env::current_exe() {
        if let Some(bin_dir) = exe.parent() {
            candidates.push(bin_dir.join("../share/verbot/scripts").join(script_name));
            candidates.push(bin_dir.join("../../scripts").join(script_name));
        }
    }
    candidates.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("scripts")
            .join(script_name),
    );
    if let Some(home) = dirs::home_dir() {
        candidates.push(home.join(".local/share/verbot/scripts").join(script_name));
    }
    candidates.push(PathBuf::from("/usr/local/share/verbot/scripts").join(script_name));
    candidates.push(PathBuf::from("/usr/share/verbot/scripts").join(script_name));

    candidates.into_iter().find(|p| p.is_file())
}

/// Captures standalone dispatch execution output for automated self-testing.
fn execute_standalone_dispatch_captured(
    manifest: &DispatchManifest,
    cwd: &std::path::Path,
    dry_run_agy: bool,
) -> Result<std::process::Output> {
    let action = DispatchAction::from_slug(&manifest.action).unwrap_or(DispatchAction::StdoutClipboard);
    let script_name = match action {
        DispatchAction::AgyInteractive | DispatchAction::AgyPrint => "dispatch-agy.sh",
        DispatchAction::StdoutClipboard | DispatchAction::SaveMarkdown => "dispatch-output.sh",
    };
    let script_path = resolve_dispatch_script(script_name, cwd)
        .with_context(|| format!("Could not locate dispatch script {}", script_name))?;
    let tmp_manifest = std::env::temp_dir().join(format!(
        "verbot-standalone-test-manifest-{}.json",
        std::process::id()
    ));
    manifest.write_to_path(&tmp_manifest)?;
    let mut cmd = std::process::Command::new("bash");
    cmd.arg(&script_path).arg("--manifest").arg(&tmp_manifest);
    cmd.env("VERBOT_NO_CLIPBOARD", "1");
    if dry_run_agy {
        cmd.env("VERBOT_AGY_DRY_RUN", "1");
    }
    let output = cmd.output();
    let _ = std::fs::remove_file(&tmp_manifest);
    output.with_context(|| format!("Failed to run {}", script_path.display()))
}

fn run_self_test() -> Result<()> {
    println!("==> [Verbocity Self-Test] Step 1: Branding, NovOS/COSMIC Theme Engine & Template/Model Parser");

    // 1. Verify SVG Logo Asset exists and contains NovOS palette stops + speaker-ear geometry
    let svg_path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets/verbot-logo.svg");
    let svg_raw = std::fs::read_to_string(&svg_path)
        .with_context(|| format!("Missing logo SVG at {}", svg_path.display()))?;
    for required_hex in [
        "#0B0C10", "#07080B", "#121521", "#1E2233", "#FF6B1A", "#FF9E44", "#FFB347", "#FFF3E0",
        "#D8E2FF",
    ] {
        ensure!(
            svg_raw.contains(required_hex),
            "verbot-logo.svg missing NovOS palette color {}",
            required_hex
        );
    }
    ensure!(
        svg_raw.contains("speaker-ear-left") && svg_raw.contains("speaker-ear-right"),
        "verbot-logo.svg missing speaker-cone robot ears"
    );
    println!("  [OK] assets/verbot-logo.svg verified (NovOS Cosmic Nova Orange palette + speaker-eared boxy robot)");

    // 2. Verify In-App ASCII Branding Banner (`VERBOCITY` + speaker-eared robot head mascot)
    let banner_wide = render_banner_plain(110);
    let banner_med = render_banner_plain(72);
    let banner_compact = render_banner_plain(48);
    ensure!(
        banner_wide.contains("VERBOCITY")
            && banner_wide.contains("◁((│ ◉   ◉ │))▷")
            && banner_wide.contains("(( [◉_◉] ))"),
        "Wide ASCII banner missing VERBOCITY or speaker-eared robot mascot"
    );
    ensure!(
        banner_med.contains("VERBOCITY") && banner_med.contains("(( [◉_◉] ))"),
        "Medium ASCII banner missing VERBOCITY or robot mascot"
    );
    ensure!(
        banner_compact.contains("VERBOCITY") && banner_compact.contains("(( [◉_◉] ))"),
        "Compact ASCII banner missing VERBOCITY or robot mascot"
    );
    println!("\n{}\n", banner_wide);
    println!("  [OK] src/branding.rs ASCII header + boxy speaker-eared robot mascot verified across terminal widths");

    // 3. Verify NovOS Default Palette & COSMIC DE RON Theme Ingestion
    let default_theme = Theme::novos_cosmic_dark();
    ensure!(default_theme.bg_primary == RgbColor::new(0x0B, 0x0C, 0x10));
    ensure!(default_theme.surface_primary == RgbColor::new(0x12, 0x15, 0x21));
    ensure!(default_theme.border_slate == RgbColor::new(0x2A, 0x31, 0x4A));
    ensure!(default_theme.accent_primary == RgbColor::new(0xFF, 0x6B, 0x1A));
    ensure!(default_theme.accent_secondary == RgbColor::new(0xFF, 0x9E, 0x44));
    ensure!(default_theme.accent_tertiary == RgbColor::new(0xFF, 0xB3, 0x47));
    ensure!(default_theme.highlight_core == RgbColor::new(0xFF, 0xF3, 0xE0));
    ensure!(default_theme.text_primary == RgbColor::new(0xD8, 0xE2, 0xFF));
    ensure!(default_theme.title_inactive == RgbColor::new(0xFF, 0x9E, 0x44));
    ensure!(default_theme.badge_inactive == RgbColor::new(0x9B, 0xA8, 0xCC));

    let ron_accent = parse_ron_rgb("(red: 1.0, green: 0.41960785, blue: 0.101960786)")
        .context("Failed to parse NovOS COSMIC RON accent tuple")?;
    ensure!(
        ron_accent.to_hex() == "#FF6B1A",
        "NovOS RON float tuple did not resolve to #FF6B1A"
    );

    let cosmic_theme = Theme::from_preset(ThemePreset::CosmicSystemAuto);
    let styled_lines = render_banner_lines(110, &default_theme);
    ensure!(styled_lines.len() == 7, "Expected 7 styled banner lines");
    println!(
        "  [OK] src/theme.rs verified (NovOS Default={} [{}], COSMIC Auto={})",
        default_theme.name,
        default_theme.accent_primary.to_hex(),
        cosmic_theme.name
    );

    // 4. Verify Template Parser on `templates/GeminiAgentPromptTemplate.md`
    let repo_template_path = {
        let canonical = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("templates/GeminiAgentPromptTemplate.md");
        if canonical.is_file() {
            canonical
        } else {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("GeminiAgentPromptTemplate.md")
        }
    };
    let parsed_template = if repo_template_path.is_file() {
        PromptTemplate::load_from_path(&repo_template_path)?
    } else {
        PromptTemplate::parse_markdown(DEFAULT_GEMINI_TEMPLATE_MD)?
    };
    ensure!(
        parsed_template.slash_commands == vec!["plan", "boost", "goal"],
        "Unexpected slash commands: {:?}",
        parsed_template.slash_commands
    );
    ensure!(
        parsed_template.fields.len() == 4,
        "Expected 4 template fields in GeminiAgentPromptTemplate.md, got {}",
        parsed_template.fields.len()
    );
    ensure!(
        parsed_template.fields[3].choices
            == vec!["Diagnose only", "Update <file>.md plan", "Execute Step X"],
        "Output choices mismatch: {:?}",
        parsed_template.fields[3].choices
    );
    println!(
        "  [OK] src/template.rs verified (Parsed '{}' with commands {:?} and {} fields)",
        parsed_template.name,
        parsed_template.slash_commands,
        parsed_template.fields.len()
    );

    // 5. Verify Decoupled Canonical Field State & Multi-Model Formatting
    let mut values = PromptValues::new();
    values.slash_command = Some("goal".to_string());
    values.objective = "Execute Step 1 of STEPPED_IMPLEMENTATION_PLAN.md".to_string();
    values.context_logs =
        "STEPPED_IMPLEMENTATION_PLAN.md, README.md, GeminiAgentPromptTemplate.md".to_string();
    values.constraints =
        "Match exact NovOS palette; keep canonical field values decoupled".to_string();
    values.output = "Execute Step 1 and verify with cargo test and cargo run -- --self-test".to_string();

    let catalog = ModelCatalog::builtin();
    ensure!(
        catalog.profiles.len() >= 5,
        "Expected at least 5 model profiles"
    );

    for profile in &catalog.profiles {
        let model_id = profile.resolve_agy_model_id(profile.default_effort);
        let rendered = profile.render_prompt(&values);
        let metrics = estimate_prompt_metrics(&rendered);
        let cli_preview = build_agy_command_preview(
            AgyLaunchMode::Interactive,
            &model_id,
            profile.default_effort,
            &rendered,
        );
        ensure!(
            rendered.contains(&values.objective)
                && rendered.contains(&values.context_logs)
                && rendered.contains(&values.constraints)
                && rendered.contains(&values.output),
            "Model '{}' lost canonical field content during formatting",
            profile.display_name
        );
        println!(
            "  [OK] Model Profile: {:<18} -> id={:<24} | words={} est_tokens={} | {}",
            profile.display_name,
            model_id,
            metrics.words,
            metrics.estimated_tokens,
            cli_preview
        );
    }

    println!("\n==> [Verbocity Self-Test] Step 2: Per-Directory State, Template Ingestion & CWD File Scanner");

    let tmp_test_root =
        std::env::temp_dir().join(format!("verbot_selftest_step2_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp_test_root);
    let fake_cwd = tmp_test_root.join("sample_workspace");
    let fake_xdg_data = tmp_test_root.join("xdg_data");
    let fake_global_templates = tmp_test_root.join("config_templates");
    let fake_theme_toml = tmp_test_root.join("theme.toml");
    std::fs::create_dir_all(&fake_cwd)?;

    // 6. Verify WorkspaceState XDG cache persistence (without polluting CWD with `.verbot/`)
    let (mut ws_state, initial_loc) =
        WorkspaceState::load_for_dir(&fake_cwd, Some(&fake_xdg_data), None)?;
    ensure!(
        !initial_loc.is_project_local() && !fake_cwd.join(".verbot").exists(),
        "Default WorkspaceState load must not create .verbot/ in user CWD"
    );
    ws_state.toggle_pinned_constraint("No QEMU");
    ws_state.toggle_pinned_constraint("Drop .img");
    ws_state.default_plan_path = Some("STEPPED_IMPLEMENTATION_PLAN.md".to_string());
    ws_state.selected_model_family = "Claude".to_string();
    ws_state.selected_sub_model = "Opus".to_string();
    ws_state.selected_effort = "high".to_string();
    ws_state.set_theme_preset(ThemePreset::CosmicSystemAuto, Some(&fake_theme_toml))?;
    ws_state.record_prompt(
        "claude-opus-5-5-high",
        EffortLevel::High,
        values.clone(),
        "sample rendered prompt",
    );

    let saved_loc = ws_state.save_for_dir(&fake_cwd, false, Some(&fake_xdg_data))?;
    ensure!(
        !saved_loc.is_project_local() && !fake_cwd.join(".verbot").exists(),
        "Saving default WorkspaceState must use XDG cache and never pollute CWD with .verbot/"
    );

    let (reloaded_ws, reloaded_loc) =
        WorkspaceState::load_for_dir(&fake_cwd, Some(&fake_xdg_data), None)?;
    ensure!(reloaded_loc == saved_loc, "Reloaded location mismatch");
    ensure!(
        reloaded_ws.pinned_constraints == vec!["No QEMU", "Drop .img"],
        "Pinned constraints did not round-trip: {:?}",
        reloaded_ws.pinned_constraints
    );
    ensure!(
        reloaded_ws.active_theme_preset == ThemePreset::CosmicSystemAuto.as_str(),
        "Theme preference did not persist in WorkspaceState"
    );
    ensure!(
        reloaded_ws.prompt_history.len() == 1,
        "Prompt history ring-buffer did not persist"
    );
    let merged_constraints = reloaded_ws.apply_pinned_constraints("Keep PAM stack");
    ensure!(
        merged_constraints == "No QEMU; Drop .img; Keep PAM stack",
        "Unexpected merged constraints: {}",
        merged_constraints
    );
    println!(
        "  [OK] src/state.rs verified (XDG cache state={}, pinned={:?}, theme={})",
        saved_loc.path().display(),
        reloaded_ws.pinned_constraints,
        reloaded_ws.active_theme_preset
    );

    // 7. Verify `verbot ingest` dry-run and companion `.toml` stub generation
    let ingest_opts = IngestOptions {
        source_path: repo_template_path.clone(),
        local: false,
        cwd: Some(fake_cwd.clone()),
        custom_global_templates_dir: Some(fake_global_templates.clone()),
        custom_name: None,
        dry_run: false,
    };
    let ingest_report = ingest_template(&ingest_opts)?;
    ensure!(
        ingest_report.primary_dest_path.is_file(),
        "Ingested Markdown template not written"
    );
    let comp_toml = ingest_report
        .companion_toml_path
        .as_ref()
        .context("Missing companion .toml path")?;
    ensure!(comp_toml.is_file(), "Companion .toml stub not written");
    let discovered = discover_ingested_templates(&fake_cwd, Some(&fake_global_templates));
    ensure!(
        discovered.len() == 1 && discovered[0].1.fields.len() == 4,
        "Failed to discover ingested template and companion metadata"
    );
    println!(
        "  [OK] src/ingest.rs verified (Ingested '{}' + companion '{}')",
        ingest_report.primary_dest_path.display(),
        comp_toml.display()
    );

    // 8. Verify CWD gitignore-aware file scanner & plan `.md` detection on repository root
    let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let file_index = WorkspaceFileIndex::scan(&repo_root)?;
    ensure!(
        !file_index.files.is_empty(),
        "WorkspaceFileIndex::scan found 0 files in repository root"
    );
    ensure!(
        !file_index
            .files
            .iter()
            .any(|f| f.relative_path.starts_with("target/")),
        "WorkspaceFileIndex::scan should exclude target/ directory"
    );
    let primary_plan = file_index
        .detect_primary_plan_file()
        .context("Failed to detect primary plan .md file in repository root")?;
    ensure!(
        primary_plan.relative_path == "STEPPED_IMPLEMENTATION_PLAN.md",
        "Expected STEPPED_IMPLEMENTATION_PLAN.md as primary plan candidate, got {}",
        primary_plan.relative_path
    );
    let fuzzy_matches = file_index.fuzzy_search("state", false, 5);
    ensure!(
        fuzzy_matches
            .iter()
            .any(|m| m.relative_path == "src/state.rs"),
        "Fuzzy search for 'state' did not match src/state.rs"
    );
    println!(
        "  [OK] src/files.rs verified (Scanned {} files, primary plan='{}', fuzzy search OK)",
        file_index.files.len(),
        primary_plan.relative_path
    );

    println!("\n==> [Verbocity Self-Test] Step 3: Headless Multi-Pane Ratatui Interface (TestBackend)");

    // 9. Initialize App and verify headless Ratatui rendering with NovOS Cosmic Dark theme
    let (tui_ws, tui_loc) =
        WorkspaceState::load_for_dir(&repo_root, Some(&fake_xdg_data), None)?;
    let mut app = App::new(
        repo_root.clone(),
        parsed_template.clone(),
        ModelCatalog::builtin(),
        tui_ws,
        tui_loc,
        Theme::novos_cosmic_dark(),
        file_index,
        false,
    );
    app.custom_xdg_data_dir = Some(fake_xdg_data.clone());
    app.custom_theme_toml_path = Some(fake_theme_toml.clone());

    app.editors[0].set_text("Build multi-pane Ratatui interface with NovOS/COSMIC theme");
    app.editors[1].set_text("STEPPED_IMPLEMENTATION_PLAN.md, src/app.rs, src/tui.rs");
    app.editors[2].set_text("Preserve field text across model and theme switches");
    app.editors[3].set_text("Verify with cargo test and cargo run -- --self-test");
    app.sync_values_from_editors();

    let buf = render_headless_buffer(&mut app, 150, 45)?;
    let screen_str = buffer_to_string(&buf);
    ensure!(
        screen_str.contains("VERBOCITY")
            && screen_str.contains("◁((│ ◉   ◉ │))▷")
            && screen_str.contains("(( [◉_◉] ))"),
        "Headless TUI buffer missing VERBOCITY ASCII banner or speaker-eared robot mascot"
    );
    ensure!(
        screen_str.contains("Build multi-pane Ratatui interface with NovOS/COSMIC theme")
            && screen_str.contains("Live Prompt Preview")
            && screen_str.contains("agy -i --model gemini-4-argon-medium"),
        "Headless TUI buffer missing editor text, live preview, or agy CLI footer"
    );

    // Check NovOS RGB colors in rendered cells
    let mut found_surface = false;
    let mut found_nova_orange = false;
    let mut found_border_slate = false;
    for y in 0..45 {
        for x in 0..150 {
            let cell = &buf[(x, y)];
            if cell.bg == novos_palette::COSMIC_SPACE_SURFACE.to_ratatui() {
                found_surface = true;
            }
            if cell.fg == novos_palette::COSMIC_NOVA_ORANGE.to_ratatui()
                || cell.bg == novos_palette::COSMIC_NOVA_ORANGE.to_ratatui()
            {
                found_nova_orange = true;
            }
            if cell.fg == novos_palette::ELEVATED_BORDER_SLATE.to_ratatui() {
                found_border_slate = true;
            }
        }
    }
    ensure!(
        found_surface && found_nova_orange && found_border_slate,
        "Headless TUI buffer missing expected NovOS Cosmic Nova Orange / Surface / Border RGB cells"
    );
    println!("  [OK] Headless Ratatui TestBackend rendered VERBOCITY + speaker-eared robot header and NovOS RGB palette");

    // 10. Verify live theme cycling (`Ctrl+T`) preserves field text
    app.handle_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))?;
    ensure!(
        app.theme.preset == ThemePreset::CosmicSystemAuto,
        "Ctrl+T did not switch to COSMIC System Auto"
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))?;
    ensure!(
        app.theme.preset == ThemePreset::NovosSolarLight,
        "Second Ctrl+T did not switch to NovOS Solar Light"
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))?;
    ensure!(
        app.theme.preset == ThemePreset::TerminalAnsi,
        "Third Ctrl+T did not switch to Terminal ANSI"
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))?;
    ensure!(
        app.theme.preset == ThemePreset::NovosCosmicDark,
        "Fourth Ctrl+T did not cycle back to NovOS Cosmic Dark"
    );
    ensure!(
        app.values.objective == "Build multi-pane Ratatui interface with NovOS/COSMIC theme"
            && app.values.constraints == "Preserve field text across model and theme switches",
        "Theme cycling altered canonical field text"
    );
    println!("  [OK] Live theme cycling (`Ctrl+T`) verified across all presets with preserved field text");

    // 11. Verify model & effort switching (`Alt+M` / `Alt+E`) preserves field text and updates live preview & agy footer
    app.select_model_by_query("Opus");
    app.handle_key_event(KeyEvent::new(KeyCode::Char('e'), KeyModifiers::ALT))?; // medium -> high
    let claude_buf = render_headless_buffer(&mut app, 150, 45)?;
    let claude_screen = buffer_to_string(&claude_buf);
    ensure!(
        claude_screen.contains("<context>")
            && claude_screen.contains("claude-opus-5-5-high")
            && app.values.objective
                == "Build multi-pane Ratatui interface with NovOS/COSMIC theme",
        "Switching to Claude Opus (high) did not update live preview / agy footer or lost field text"
    );

    app.select_model_by_query("GPT-OSS 120B");
    let gpt_buf = render_headless_buffer(&mut app, 150, 45)?;
    let gpt_screen = buffer_to_string(&gpt_buf);
    ensure!(
        gpt_screen.contains("### Context / Logs")
            && gpt_screen.contains("gpt-oss-120b-medium")
            && app.values.objective
                == "Build multi-pane Ratatui interface with NovOS/COSMIC theme",
        "Switching to GPT-OSS 120B did not update live preview / agy footer or lost field text"
    );
    app.select_model_by_query("Argon");
    println!("  [OK] Model & effort switching verified (Gemini Argon <-> Claude Opus <-> GPT-OSS 120B)");

    // 12. Verify `@` / `Ctrl+F` CWD Fuzzy File Picker Modal and dispatch manifest emission
    app.focus = FocusPane::FieldEditor(1);
    app.handle_key_event(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE))?;
    ensure!(app.file_picker.is_open, "'@' did not open CWD file picker modal");
    for ch in "branding.rs".chars() {
        app.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
    }
    let modal_buf = render_headless_buffer(&mut app, 130, 42)?;
    let modal_screen = buffer_to_string(&modal_buf);
    ensure!(
        modal_screen.contains("CWD Fuzzy File Picker")
            && modal_screen.contains("src/branding.rs"),
        "File picker modal did not render or match src/branding.rs"
    );
    app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    ensure!(
        !app.file_picker.is_open && app.values.context_logs.contains("src/branding.rs"),
        "File picker Enter did not insert src/branding.rs into Context/Logs"
    );

    let dispatch_json_path = tmp_test_root.join("dispatch-manifest.json");
    let manifest = app.confirm_dispatch(DispatchAction::AgyInteractive)?;
    manifest.write_to_path(&dispatch_json_path)?;
    ensure!(
        dispatch_json_path.is_file()
            && manifest.action == "agy-interactive"
            && manifest.rendered_prompt.contains("src/branding.rs"),
        "DispatchManifest failed to serialize or lacked inserted file path"
    );
    println!(
        "  [OK] CWD Fuzzy File Picker (`@` / `Ctrl+F`) and DispatchManifest emission verified"
    );

    println!("\n==> [Verbocity Self-Test] Step 4: Bash Launcher (`bin/verbot`), Desktop/Icon Assets, Dispatch Scripts & Installer");

    // 13. Verify `assets/verbot.desktop` and Bash scripts (`bash -n`)
    let desktop_path = repo_root.join("assets/verbot.desktop");
    let desktop_raw = std::fs::read_to_string(&desktop_path)
        .with_context(|| format!("Missing desktop launcher at {}", desktop_path.display()))?;
    ensure!(
        desktop_raw.contains("Name=Verbocity")
            && desktop_raw.contains("Exec=verbot")
            && desktop_raw.contains("Icon=verbot")
            && desktop_raw.contains("Terminal=true"),
        "assets/verbot.desktop missing required COSMIC desktop keys"
    );
    println!("  [OK] assets/verbot.desktop verified (Name=Verbocity, Exec=verbot, Icon=verbot)");

    let scripts_to_check = [
        "bin/verbot",
        "scripts/dispatch-agy.sh",
        "scripts/dispatch-output.sh",
        "install.sh",
    ];
    for rel_script in scripts_to_check {
        let script_path = repo_root.join(rel_script);
        ensure!(
            script_path.is_file(),
            "Missing Step 4 script: {}",
            script_path.display()
        );
        let status = std::process::Command::new("bash")
            .arg("-n")
            .arg(&script_path)
            .status()
            .with_context(|| format!("Failed to invoke bash -n on {}", rel_script))?;
        ensure!(status.success(), "bash -n failed for {}", rel_script);
    }
    println!("  [OK] bash -n syntax validation passed for bin/verbot, scripts/dispatch-agy.sh, scripts/dispatch-output.sh, and install.sh");

    // 14. Verify end-to-end piped stdin capture + dispatch scripts (`stdout` and `agy-interactive` dry-run)
    let current_exe = std::env::current_exe().context("Failed to resolve current executable path")?;
    let bin_verbot = repo_root.join("bin/verbot");
    let mut pipe_child = std::process::Command::new(&bin_verbot)
        .env("VERBOT_TUI_BIN", &current_exe)
        .env("VERBOT_NO_CLIPBOARD", "1")
        .args([
            "--headless",
            "--model",
            "gemini-4-argon-medium",
            "--effort",
            "high",
            "--action",
            "stdout",
            "--objective",
            "Verify piped stdin log capture",
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn bin/verbot for piped stdin test")?;

    if let Some(mut child_stdin) = pipe_child.stdin.take() {
        use std::io::Write;
        writeln!(child_stdin, "sample log failure: panic at line 42")?;
    }
    let pipe_out = pipe_child.wait_with_output()?;
    ensure!(
        pipe_out.status.success(),
        "bin/verbot piped headless stdout test failed: {}",
        String::from_utf8_lossy(&pipe_out.stderr)
    );
    let rendered_stdout = String::from_utf8_lossy(&pipe_out.stdout);
    ensure!(
        rendered_stdout.contains("Verify piped stdin log capture")
            && rendered_stdout.contains("sample log failure: panic at line 42"),
        "bin/verbot stdout output missing objective or captured stdin log:\n{}",
        rendered_stdout
    );
    println!("  [OK] End-to-end piped stdin capture -> bin/verbot -> dispatch-output.sh verified");

    // Verify `dispatch-agy.sh --dry-run` via `bin/verbot --dry-run-dispatch`
    let agy_dry_out = std::process::Command::new(&bin_verbot)
        .env("VERBOT_TUI_BIN", &current_exe)
        .args([
            "--headless",
            "--dry-run-dispatch",
            "--model",
            "claude-opus-5-5-high",
            "--effort",
            "high",
            "--action",
            "agy-interactive",
            "--objective",
            "Dry-run agy dispatch check",
        ])
        .output()
        .context("Failed to run bin/verbot --dry-run-dispatch")?;
    ensure!(
        agy_dry_out.status.success(),
        "bin/verbot --dry-run-dispatch failed: {}",
        String::from_utf8_lossy(&agy_dry_out.stderr)
    );
    let agy_dry_str = String::from_utf8_lossy(&agy_dry_out.stdout);
    ensure!(
        agy_dry_str.contains("-i --model claude-opus-5-5-high --effort high")
            && agy_dry_str.contains("Dry-run agy dispatch check"),
        "dispatch-agy.sh dry-run output mismatch:\n{}",
        agy_dry_str
    );
    println!("  [OK] End-to-end dispatch-agy.sh dry-run (`-i --model claude-opus-5-5-high --effort high`) verified");

    // 15. Verify `install.sh --skip-build` into isolated staging prefix
    let stage_prefix = tmp_test_root.join("install_stage_prefix");
    let stage_config = tmp_test_root.join("install_stage_config");
    let install_out = std::process::Command::new(repo_root.join("install.sh"))
        .env("VERBOT_TUI_BIN", &current_exe)
        .args([
            "--skip-build",
            "--no-modify-path",
            "--prefix",
            &stage_prefix.to_string_lossy(),
            "--config-dir",
            &stage_config.to_string_lossy(),
        ])
        .output()
        .context("Failed to run install.sh into staging prefix")?;
    ensure!(
        install_out.status.success(),
        "install.sh staging test failed: {}",
        String::from_utf8_lossy(&install_out.stderr)
    );
    ensure!(
        stage_prefix.join("bin/verbot").is_file()
            && stage_prefix.join("bin/verbot-tui").is_file()
            && stage_prefix
                .join("share/verbot/scripts/dispatch-agy.sh")
                .is_file()
            && stage_prefix
                .join("share/verbot/scripts/dispatch-output.sh")
                .is_file()
            && stage_prefix
                .join("share/icons/hicolor/scalable/apps/verbot.svg")
                .is_file()
            && stage_prefix
                .join("share/applications/verbot.desktop")
                .is_file()
            && stage_config.join("theme.toml").is_file()
            && stage_config
                .join("templates/GeminiAgentPromptTemplate.md")
                .is_file(),
        "install.sh did not populate all expected binaries, scripts, icon, desktop launcher, and config files"
    );
    println!("  [OK] install.sh verified in isolated staging prefix (binaries, scripts, SVG icon, COSMIC .desktop, and theme.toml)");

    println!("\n==> [Verbocity Self-Test] Step 5: Lighter Placeholders (#5A6588), Soft-Wrapping Boxes, Model Sync/Add Modal (Alt+U), Compact Guard, Plan Picker Approval Gate & Standalone Dispatch");

    // 16. Verify #5A6588 (`PLACEHOLDER_SLATE_MUTED`) placeholder contrast in rendered empty editors
    let (step5_ws, step5_loc) =
        WorkspaceState::load_for_dir(&repo_root, Some(&fake_xdg_data), None)?;
    let step5_index = WorkspaceFileIndex::scan(&repo_root)?;
    let mut step5_app = App::new(
        repo_root.clone(),
        parsed_template.clone(),
        ModelCatalog::builtin(),
        step5_ws,
        step5_loc,
        Theme::novos_cosmic_dark(),
        step5_index,
        false,
    );
    step5_app.custom_xdg_data_dir = Some(fake_xdg_data.clone());
    step5_app.custom_theme_toml_path = Some(fake_theme_toml.clone());
    step5_app.custom_models_toml_path = Some(tmp_test_root.join("models.toml"));

    // Clear all editors so placeholders render
    for editor in &mut step5_app.editors {
        editor.set_text("");
    }
    step5_app.sync_values_from_editors();

    let empty_buf = render_headless_buffer(&mut step5_app, 125, 40)?;
    let mut has_placeholder_5a6588 = false;
    for y in 0..40 {
        for x in 0..125 {
            if empty_buf[(x, y)].fg == novos_palette::PLACEHOLDER_SLATE_MUTED.to_ratatui() {
                has_placeholder_5a6588 = true;
                break;
            }
        }
    }
    ensure!(
        step5_app.theme.placeholder_muted.to_hex() == "#5A6588" && has_placeholder_5a6588,
        "Empty field placeholders did not render with #5A6588 (PLACEHOLDER_SLATE_MUTED)"
    );
    println!("  [OK] Lighter placeholder contrast (#5A6588 PLACEHOLDER_SLATE_MUTED) verified in rendered editor panes");

    // 17. Verify visual soft-wrapping across all 4 input boxes (`Objective`, `Context/Logs`, `Constraints`, `Output`)
    let long_input = "wrap_token_alpha wrap_token_beta wrap_token_gamma wrap_token_delta wrap_token_epsilon wrap_token_zeta wrap_token_eta";
    for editor in &mut step5_app.editors {
        editor.set_text(long_input);
    }
    step5_app.sync_values_from_editors();

    let (wrapped_lines, v_cursor_row, _) =
        wrap_editor_lines_with_cursor(&[long_input.to_string()], (0, long_input.len()), 38);
    ensure!(
        wrapped_lines.len() >= 3 && v_cursor_row >= 2,
        "Expected long line to soft-wrap across >=3 visual lines, got {:?}",
        wrapped_lines
    );
    let softwrap_buf = render_headless_buffer(&mut step5_app, 105, 40)?;
    let softwrap_screen = buffer_to_string(&softwrap_buf);
    ensure!(
        softwrap_screen.contains("wrap_token_alpha") && softwrap_screen.contains("wrap_token_eta"),
        "Soft-wrapped start and end tokens not both visible in rendered editor panes"
    );
    println!("  [OK] Width-aware visual soft-wrapping and wrapped cursor projection verified across all 4 input boxes");

    // 18. Verify In-Interface Model Catalog Sync & Add Modal (`Alt+U`) + `agy models` output parser
    let parsed_agy = parse_agy_models_output(
        "gemini-4-argon-medium\ngemini-4-quantum-high Gemini 4 Quantum\nclaude-opus-5-6-high\n",
    );
    ensure!(
        parsed_agy
            .iter()
            .any(|p| p.agy_base_id == "gemini-4-quantum" && p.short_name == "Quantum"),
        "parse_agy_models_output failed to extract gemini-4-quantum"
    );

    // Create a mock `agy` executable script to test `Ctrl+R` sync inside the modal
    let mock_agy_path = tmp_test_root.join("mock-agy");
    std::fs::write(
        &mock_agy_path,
        "#!/usr/bin/env bash\nif [[ \"${1:-}\" == \"models\" ]]; then\n  echo \"gemini-4-quantum-medium\"\n  echo \"gemini-4-quantum-high\"\nfi\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&mock_agy_path)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&mock_agy_path, perms)?;
    }
    step5_app.custom_agy_bin = Some(mock_agy_path);

    // Open modal with `Alt+U`
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT))?;
    ensure!(
        step5_app.model_modal.is_open,
        "Alt+U did not open ModelManagerModal"
    );
    let model_modal_buf = render_headless_buffer(&mut step5_app, 125, 40)?;
    let model_modal_screen = buffer_to_string(&model_modal_buf);
    ensure!(
        model_modal_screen.contains("Model Catalog Sync & Add Custom Sub-Model (Alt+U)"),
        "ModelManagerModal did not render expected title"
    );

    // Trigger `Ctrl+R` inside modal to sync from mock `agy models`
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL))?;
    ensure!(
        step5_app.catalog.find_by_query("Quantum").is_some(),
        "Ctrl+R inside ModelManagerModal did not sync 'Quantum' from `agy models`"
    );

    // Also type a custom sub-model name `"Nebula"` and press `Enter` to add & select it
    for ch in "Nebula".chars() {
        step5_app.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
    }
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    ensure!(
        !step5_app.model_modal.is_open
            && step5_app.active_agy_model_id().starts_with("gemini-nebula-")
            && step5_app.values.objective == long_input,
        "Submitting custom model from Alt+U modal failed or lost canonical field values (got model_id={})",
        step5_app.active_agy_model_id()
    );
    println!("  [OK] In-UI Model Catalog Sync (`Ctrl+R`) & Add Custom Sub-Model Modal (`Alt+U`) verified");

    // 19. Verify Adaptive Compact Layout (`75x30`) and Minimum Window Size Guard (`< 75x28`)
    let compact_buf = render_headless_buffer(&mut step5_app, 75, 30)?;
    let compact_screen = buffer_to_string(&compact_buf);
    ensure!(
        compact_screen.contains("Objective") && compact_screen.contains("Output"),
        "Compact 75x30 layout failed to render all 4 input boxes"
    );
    let guard_buf = render_headless_buffer(&mut step5_app, 60, 18)?;
    let guard_screen = buffer_to_string(&guard_buf);
    ensure!(
        guard_screen.contains("Minimum size: 75x28"),
        "Minimum window size guard (< 75x28) did not render resize notice"
    );
    println!("  [OK] Adaptive compact layout (75x30) and minimum window size guard (< 75x28) verified");

    // 20. Verify Pending Approval Gate (`Enter` / `Alt+Y` vs `Esc` / `Alt+N`) before `Update <file>.md plan` opens `.md` file picker
    step5_app.focus = FocusPane::FieldEditor(3);
    step5_app.editors[3].active_choice_idx = None;
    // First Alt+C -> "Diagnose only"
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT))?;
    ensure!(step5_app.values.output == "Diagnose only");
    // Second Alt+C -> "Update <file>.md plan" (stages pending approval, does NOT immediately open file_picker)
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT))?;
    ensure!(
        step5_app.values.output == "Update <file>.md plan"
            && step5_app.pending_plan_picker_approval.is_some()
            && !step5_app.file_picker.is_open,
        "Selecting 'Update <file>.md plan' must stage pending approval before opening FilePickerModal"
    );
    let approval_buf = render_headless_buffer(&mut step5_app, 120, 38)?;
    let approval_screen = buffer_to_string(&approval_buf);
    ensure!(
        approval_screen.contains("Confirm Opening .md Plan File Picker"),
        "Pending .md plan file picker approval banner did not render"
    );
    // Test declining with Esc keeps literal "Update <file>.md plan" without opening picker
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?;
    ensure!(
        step5_app.pending_plan_picker_approval.is_none()
            && !step5_app.file_picker.is_open
            && step5_app.values.output == "Update <file>.md plan",
        "Declining plan picker approval with Esc should keep literal 'Update <file>.md plan'"
    );
    // Cycle back to "Update <file>.md plan" and approve with Alt+Y
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT))?; // Execute Step X
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT))?; // Diagnose only
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT))?; // Update <file>.md plan
    ensure!(step5_app.pending_plan_picker_approval.is_some());
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('y'), KeyModifiers::ALT))?;
    ensure!(
        step5_app.file_picker.is_open && step5_app.file_picker.markdown_only,
        "Approving with Alt+Y did not open markdown FilePickerModal"
    );
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    ensure!(
        step5_app.values.output == "Update STEPPED_IMPLEMENTATION_PLAN.md plan",
        "Confirming file in picker did not replace <file>.md with STEPPED_IMPLEMENTATION_PLAN.md"
    );
    println!("  [OK] Approval gate prior to opening .md plan file picker (`Enter`/`Alt+Y` approve, `Esc`/`Alt+N` keep literal) verified");

    // 21. Verify Standalone `verbot-tui` Dispatch Execution (`execute_standalone_dispatch_captured`)
    step5_app.select_model_by_query("gemini-4-argon-medium");
    let standalone_manifest = step5_app.confirm_dispatch(DispatchAction::AgyInteractive)?;
    let standalone_agy_out =
        execute_standalone_dispatch_captured(&standalone_manifest, &repo_root, true)?;
    ensure!(
        standalone_agy_out.status.success(),
        "Standalone agy dispatch failed: {}",
        String::from_utf8_lossy(&standalone_agy_out.stderr)
    );
    let standalone_agy_str = String::from_utf8_lossy(&standalone_agy_out.stdout);
    ensure!(
        standalone_agy_str.contains("-i --model gemini-4-argon-medium")
            && standalone_agy_str.contains("Update STEPPED_IMPLEMENTATION_PLAN.md plan"),
        "Standalone agy dispatch output mismatch:\n{}",
        standalone_agy_str
    );

    let standalone_stdout_manifest = step5_app.confirm_dispatch(DispatchAction::StdoutClipboard)?;
    let standalone_stdout_out =
        execute_standalone_dispatch_captured(&standalone_stdout_manifest, &repo_root, false)?;
    ensure!(
        standalone_stdout_out.status.success()
            && String::from_utf8_lossy(&standalone_stdout_out.stdout)
                .contains("Update STEPPED_IMPLEMENTATION_PLAN.md plan"),
        "Standalone output dispatch failed"
    );
    println!("  [OK] Standalone `verbot-tui` dispatch execution (`dispatch-agy.sh` & `dispatch-output.sh`) verified");

    println!("\n==> [Verbocity Self-Test] Step 6: High-Contrast Unfocused Field Headers (#FF9E44/#2A314A), Non-Warping 3-Tier Branding, >=75x6 Field Scaling, Full agy Slash Commands & Placeholder-Free Output");

    // 22. Verify High-Contrast Unfocused Field Title RGB Cells (`#FF9E44` / `#9BA8CC` on `#121521` surface, `#2A314A` border)
    step5_app.terminal_output.is_open = false;
    step5_app.focus = FocusPane::FieldEditor(0);
    for editor in &mut step5_app.editors {
        editor.set_text("");
    }
    step5_app.sync_values_from_editors();

    let step6_buf_150 = render_headless_buffer(&mut step5_app, 150, 45)?;
    let step6_rects_150 = compute_field_editor_rects_for_terminal(150, 45, &step5_app);
    ensure!(step6_rects_150.len() == 4, "Expected 4 field editor rects");
    for (idx, rect) in step6_rects_150.iter().enumerate().skip(1) {
        let top_y = rect.y;
        let mut found_title_ff9e44 = false;
        let mut found_badge_9ba8cc = false;
        let mut found_border_2a314a = false;
        for x in rect.x..rect.x + rect.width {
            let cell = &step6_buf_150[(x, top_y)];
            if cell.fg == novos_palette::ACCRETION_AMBER.to_ratatui()
                && cell.bg == novos_palette::COSMIC_SPACE_SURFACE.to_ratatui()
            {
                found_title_ff9e44 = true;
            }
            if cell.fg == novos_palette::INACTIVE_BADGE_SLATE.to_ratatui()
                && cell.bg == novos_palette::COSMIC_SPACE_SURFACE.to_ratatui()
            {
                found_badge_9ba8cc = true;
            }
            if cell.fg == novos_palette::ELEVATED_BORDER_SLATE.to_ratatui() {
                found_border_2a314a = true;
            }
        }
        ensure!(
            found_title_ff9e44 && found_badge_9ba8cc && found_border_2a314a,
            "Unfocused field {} missing #FF9E44 title, #9BA8CC badge, or #2A314A border",
            idx
        );
    }
    println!("  [OK] High-contrast unfocused field labels (#FF9E44 Accretion Amber + #9BA8CC badge on #121521 surface, #2A314A inactive border) verified");

    // 23. Verify Non-Warping 3-Tier Branding Banner & >=75x6 Field Scaling + Non-Truncating Options across 150x45, 100x36, and 75x30
    for (w, h, expected_tier, tier_label) in [
        (150u16, 45u16, BannerTier::Full7Row, "Full 7-Row Banner"),
        (100u16, 36u16, BannerTier::Medium3Row, "Medium 3-Row Banner"),
        (75u16, 30u16, BannerTier::Compact1Row, "Compact 1-Row Emblem"),
    ] {
        ensure!(
            select_banner_tier(w.saturating_sub(2), h) == expected_tier,
            "Unexpected banner tier at {}x{}",
            w,
            h
        );
        let rects = compute_field_editor_rects_for_terminal(w, h, &step5_app);
        ensure!(rects.len() == 4, "Expected 4 field editors at {}x{}", w, h);
        for (idx, r) in rects.iter().enumerate() {
            ensure!(
                r.width >= MIN_FIELD_EDITOR_WIDTH
                    && r.height >= MIN_FIELD_EDITOR_HEIGHT
                    && r.height.saturating_sub(2) >= MIN_FIELD_INNER_TEXT_ROWS,
                "Field {} at {}x{} violated minimum 75x6 (4 inner text rows) constraint: got {}x{}",
                idx,
                w,
                h,
                r.width,
                r.height
            );
        }

        let buf = render_headless_buffer(&mut step5_app, w, h)?;
        let screen = buffer_to_string(&buf);
        for required_token in [
            "VERBOCITY",
            "Family:",
            "Sub-Model (Alt+M):",
            "Effort",
            "Command:",
            "Settings (Alt+S):",
            "Objective",
            "Context/Logs",
            "Constraints",
            "Output",
            "Diagnose only",
            "Update <file>.md plan",
            "Execute Step X",
            "1: Run agy -i (Interactive)",
            "2: Run agy -p (Print)",
            "3: Output to Stdout / Clipboard",
            "4: Save to .md File",
        ] {
            ensure!(
                screen.contains(required_token),
                "Option/header '{}' was truncated or missing at {}x{} ({})",
                required_token,
                w,
                h,
                tier_label
            );
        }
        println!(
            "  [OK] Layout scaling verified at {}x{} ({}): all 4 fields >=75x6 with zero option truncation",
            w, h, tier_label
        );
    }

    // 24. Verify Full `agy` Slash Command Discovery (all 11 built-ins + `(none)` + custom skill/workflow directories)
    let custom_skill_dir = fake_cwd.join(".gemini/skills/custom-step6-skill");
    std::fs::create_dir_all(&custom_skill_dir)?;
    std::fs::write(
        custom_skill_dir.join("SKILL.md"),
        "---\nname: custom-step6-skill\ndescription: test\n---\n",
    )?;
    step5_app.refresh_slash_commands(Some(&[fake_cwd.join(".gemini")]));

    let mut cycled_cmds: Vec<Option<String>> = Vec::new();
    let total_cmd_opts = step5_app.slash_command_options().len();
    for _ in 0..total_cmd_opts {
        cycled_cmds.push(step5_app.values.slash_command.clone());
        step5_app.cycle_slash_command(true);
    }
    for builtin in BUILTIN_AGY_SLASH_COMMANDS {
        ensure!(
            cycled_cmds.contains(&Some(builtin.to_string())),
            "Missing built-in agy slash command /{} in selector",
            builtin
        );
    }
    ensure!(
        cycled_cmds.contains(&Some("custom-step6-skill".to_string())),
        "Missing dynamically discovered skill /custom-step6-skill in selector"
    );
    ensure!(
        cycled_cmds.contains(&None),
        "Missing (none) option in slash command selector"
    );
    println!(
        "  [OK] Full `agy` slash command discovery verified ({} commands selectable including all 11 built-ins, discovered skills, and (none))",
        total_cmd_opts
    );

    // 25. Verify Placeholder-Free Rendered Output & DispatchManifest
    step5_app.values.slash_command = Some("plan".to_string());
    for editor in &mut step5_app.editors {
        editor.set_text("");
    }
    step5_app.sync_values_from_editors();
    let empty_manifest = step5_app.confirm_dispatch(DispatchAction::StdoutClipboard)?;
    ensure!(
        empty_manifest.rendered_prompt == "/plan"
            && !empty_manifest.rendered_prompt.contains('<')
            && !empty_manifest.agy_command_preview.contains("1-2 sentence")
            && !empty_manifest.agy_command_preview.contains("file paths"),
        "Empty fields leaked <placeholder> text into rendered_prompt or DispatchManifest: {}",
        empty_manifest.rendered_prompt
    );
    println!("  [OK] Placeholder-free rendered prompt & DispatchManifest verified (empty fields cleanly omitted)");

    println!("\n==> [Verbocity Self-Test] Step 7: Dedicated Settings Page (Alt+S), Model/Template TOML Ingestion & Shell-Safe Text Formatting");

    // 26. Verify Full-Screen Settings Page (`Alt+S`), Navigating all 4 Tabs, Hex Color Overrides, and `agy` Flags Persistence
    step5_app.should_quit = false;
    let fake_config_toml = tmp_test_root.join("config.toml");
    let fake_models_toml = tmp_test_root.join("models.toml");
    step5_app.custom_config_toml_path = Some(fake_config_toml.clone());
    step5_app.custom_models_toml_path = Some(fake_models_toml.clone());
    step5_app.custom_global_templates_dir = Some(fake_global_templates.clone());

    let complex_feature_prompt = "Please add the following features:\n* Create a Settings Page (Alt+S) and move theming + model addition into it\n* Allow model .toml file ingestion in Settings\n* Automatically format text (quotes \"...\", single '...', $VARS, `code`, and /paths) for terminal execution";
    step5_app.editors[0].set_text(complex_feature_prompt);
    step5_app.editors[1].set_text("STEPPED_IMPLEMENTATION_PLAN.md, src/settings.rs, src/formatting.rs");
    step5_app.editors[2].set_text("Preserve all field text when entering/exiting Settings (Alt+S / Esc)");
    step5_app.editors[3].set_text("Execute Step 7 and verify with cargo test and cargo run -- --self-test");
    step5_app.sync_values_from_editors();

    // Open Settings Page via `Alt+S`
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT))?;
    ensure!(
        step5_app.settings_page.is_open
            && step5_app.settings_page.active_tab == SettingsTab::ThemeAndContrast,
        "Alt+S did not open Settings Page on Tab 1 (Theme & Contrast)"
    );
    let settings_tab1_buf = render_headless_buffer(&mut step5_app, 130, 40)?;
    let settings_tab1_str = buffer_to_string(&settings_tab1_buf);
    ensure!(
        settings_tab1_str.contains("VERBOCITY SETTINGS PAGE (Alt+S)")
            && settings_tab1_str.contains("1: Theme & Contrast")
            && settings_tab1_str.contains("Primary Accent (accent_primary)"),
        "Settings Tab 1 did not render expected Theme & Contrast controls"
    );

    // Modify hex color override on Tab 1 (`accent_primary` -> `#FF8800`) and apply
    step5_app.settings_page.hex_accent_primary = "#FF8800".to_string();
    step5_app.apply_settings_theme_hex_overrides()?;
    ensure!(
        step5_app.theme.accent_primary == RgbColor::new(0xFF, 0x88, 0x00)
            && fake_theme_toml.is_file(),
        "Applying hex color override on Settings Tab 1 did not update theme or persist theme.toml"
    );
    // Reset back to NovOS Cosmic Dark preset (`#FF6B1A`)
    step5_app.set_theme_preset(ThemePreset::NovosCosmicDark);
    ensure!(step5_app.theme.accent_primary == novos_palette::COSMIC_NOVA_ORANGE);
    println!("  [OK] Settings Tab 1 (Theme & Contrast): live preset & hex color overrides persisted to theme.toml");

    // 27. Switch to Tab 2 (`Models & TOML Ingestion`) and ingest an external model `.toml` file
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))?;
    ensure!(
        step5_app.settings_page.active_tab == SettingsTab::ModelsAndToml,
        "Pressing '2' did not switch Settings Page to Tab 2 (Models & TOML Ingestion)"
    );
    let external_model_toml = tmp_test_root.join("gemini-4-hyperion.toml");
    std::fs::write(
        &external_model_toml,
        r#"
family = "Gemini"
short_name = "Hyperion"
display_name = "Gemini 4 Hyperion"
agy_base_id = "gemini-4-hyperion"
available_tiers = ["low", "medium", "high"]
default_effort = "high"
"#,
    )?;
    for ch in external_model_toml.to_string_lossy().chars() {
        step5_app.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))?;
    }
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    ensure!(
        step5_app.catalog.find_by_query("Hyperion").is_some()
            && step5_app.active_profile().short_name == "Hyperion"
            && fake_models_toml.is_file(),
        "Ingesting external model .toml file in Settings Tab 2 did not add/select 'Hyperion' or persist models.toml"
    );
    println!(
        "  [OK] Settings Tab 2 (Models & TOML Ingestion): ingested '{}' -> active sub-model '{}' ({})",
        external_model_toml.display(),
        step5_app.active_profile().display_name,
        step5_app.active_agy_model_id()
    );

    // 28. Switch to Tab 3 (`Templates & Defaults`) and Tab 4 (`Execution, Shell Formatting & Workspace Storage`)
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT))?;
    ensure!(
        step5_app.settings_page.active_tab == SettingsTab::TemplatesAndDefaults,
        "Alt+3 did not switch to Tab 3 (Templates & Defaults)"
    );
    let settings_tab3_buf = render_headless_buffer(&mut step5_app, 130, 40)?;
    ensure!(
        buffer_to_string(&settings_tab3_buf).contains("Active Prompt Template"),
        "Settings Tab 3 missing Active Prompt Template row"
    );

    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE))?;
    ensure!(
        step5_app.settings_page.active_tab == SettingsTab::ExecutionAndWorkspace,
        "Pressing '4' did not switch to Tab 4 (Execution & Workspace)"
    );
    // Row 0 = Default agy --mode -> cycle to 'auto' then 'plan'
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))?; // auto
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))?; // plan
    // Move to Row 1 = Default agy --sandbox -> toggle to true
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))?;
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))?;
    ensure!(
        step5_app.global_config.agy_flags.mode.as_str() == "plan"
            && step5_app.global_config.agy_flags.sandbox
            && fake_config_toml.is_file(),
        "Toggling agy --mode and --sandbox on Settings Tab 4 did not persist to config.toml"
    );
    let reloaded_cfg = GlobalConfig::load_from_file(&fake_config_toml)?;
    ensure!(
        reloaded_cfg.agy_flags.mode.as_str() == "plan" && reloaded_cfg.agy_flags.sandbox,
        "Reloaded config.toml did not preserve agy_flags (--mode plan --sandbox)"
    );
    println!("  [OK] Settings Tab 3 & Tab 4: startup defaults, agy flags (--mode plan --sandbox), and workspace storage persisted to config.toml");

    // Exit Settings Page with `Esc` and verify all entered prompt text is 100% intact
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))?;
    ensure!(
        !step5_app.settings_page.is_open
            && step5_app.values.objective == complex_feature_prompt
            && step5_app.values.constraints
                == "Preserve all field text when entering/exiting Settings (Alt+S / Esc)",
        "Exiting Settings Page with Esc lost or altered prompt field text"
    );
    println!("  [OK] Entering/exiting Settings Page (`Alt+S` / `Esc`) preserved 100% of field text");

    // 29. Verify `src/formatting.rs` shell metacharacter detection, Quote Confirmation Modal, and 100% shell-parseable command output
    let safety_report = inspect_prompt_values_safety(&step5_app.values);
    ensure!(
        safety_report.needs_escaping_or_quoting()
            && safety_report.has_special_chars
            && safety_report.has_multiline,
        "inspect_prompt_values_safety failed to detect special characters and newlines in complex prompt"
    );
    ensure!(
        step5_app.global_config.shell_formatting_policy == ShellFormattingPolicy::PromptOnSpecialChars,
        "Expected default ShellFormattingPolicy::PromptOnSpecialChars"
    );

    // Trigger interactive dispatch (`Alt+1`) -> must open `ShellQuoteModal` first
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT))?;
    ensure!(
        step5_app.shell_quote_modal.is_open && !step5_app.should_quit,
        "Triggering dispatch with PromptOnSpecialChars on complex input did not open ShellQuoteModal"
    );
    let quote_modal_buf = render_headless_buffer(&mut step5_app, 130, 40)?;
    let quote_modal_str = buffer_to_string(&quote_modal_buf);
    ensure!(
        quote_modal_str.contains("Shell Metacharacter & Quote Confirmation")
            && quote_modal_str.contains("Auto-Escape & Quote for Terminal Execution"),
        "ShellQuoteModal did not render expected confirmation prompt"
    );

    // Approve with `Enter` (`Auto-Escape & Quote for Terminal Execution`)
    // In Step 8+, confirming dispatch executes in-app and keeps `should_quit == false`!
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))?;
    ensure!(
        !step5_app.shell_quote_modal.is_open && !step5_app.should_quit,
        "Approving ShellQuoteModal with Enter should complete in-app dispatch while keeping Verbocity open (should_quit == false)"
    );
    let step7_manifest = step5_app
        .confirmed_manifest
        .as_ref()
        .context("Missing confirmed DispatchManifest after ShellQuoteModal approval")?;
    let parsed_shell_argv = parse_posix_shell_words(&step7_manifest.shell_safe_command)
        .context("Generated shell_safe_command failed POSIX shell word parsing")?;
    ensure!(
        parsed_shell_argv == step7_manifest.agy_argv
            && parsed_shell_argv.last().map(String::as_str) == Some(step7_manifest.rendered_prompt.as_str())
            && step7_manifest.agy_extra_flags == vec!["--mode".to_string(), "plan".to_string(), "--sandbox".to_string()],
        "POSIX-parsed shell_safe_command did not match exact agy_argv or lacked configured flags"
    );
    println!(
        "  [OK] src/formatting.rs verified ({}; POSIX-quoted command parsed into {} exact argv words)",
        safety_report.summary(),
        parsed_shell_argv.len()
    );

    println!("\n==> [Verbocity Self-Test] Step 8: Persistent In-App Terminal Execution Pane, Non-Closing agy Loop, CLI Modifiers & PATH-Aware Installer");

    // 30. Verify Persistent Execution Loop & In-App Terminal Output Pane (`FocusPane::TerminalOutput`)
    // Create a mock `agy` executable that echoes its invocation for both `-p` and `-i`
    let step8_mock_agy = tmp_test_root.join("mock-agy-step8");
    std::fs::write(
        &step8_mock_agy,
        "#!/usr/bin/env bash\nif [[ \"${1:-}\" == \"models\" ]]; then\n  echo \"gemini-4-argon-medium\"\nelse\n  echo \"STEP8_AGY_STREAM: $*\"\nfi\n",
    )?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = std::fs::metadata(&step8_mock_agy)?.permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&step8_mock_agy, perms)?;
    }
    step5_app.custom_agy_bin = Some(step8_mock_agy);
    step5_app.global_config.shell_formatting_policy = ShellFormattingPolicy::AutoEscapeSilently;
    let step8_md_out = tmp_test_root.join("step8-output.md");
    step5_app.saved_markdown_path = Some(step8_md_out.clone());

    // Action 2 (`agy -p`): streams live output into `FocusPane::TerminalOutput` and keeps `should_quit == false`
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT))?;
    ensure!(
        !step5_app.should_quit
            && step5_app.terminal_output.is_open
            && step5_app.focus == FocusPane::TerminalOutput
            && step5_app.terminal_output.last_exit_code == Some(0)
            && step5_app
                .terminal_output
                .lines
                .iter()
                .any(|l| l.contains("STEP8_AGY_STREAM: -p --model")),
        "Running Action 2 (agy -p) did not keep Verbocity open or failed to populate TerminalOutputDrawer"
    );
    let step8_term_buf = render_headless_buffer(&mut step5_app, 150, 45)?;
    let step8_term_screen = buffer_to_string(&step8_term_buf);
    ensure!(
        step8_term_screen.contains("Terminal Execution Output Pane [Ctrl+O]")
            && step8_term_screen.contains("STEP8_AGY_STREAM: -p --model"),
        "Headless TUI buffer did not render Terminal Execution Output Pane with streamed agy -p output"
    );

    // Action 3 (`Stdout / Clipboard`) and Action 4 (`Save .md`): keep `should_quit == false`
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT))?;
    ensure!(
        !step5_app.should_quit
            && step5_app
                .terminal_output
                .lines
                .iter()
                .any(|l| l.contains("Action 3: Rendered Prompt Output / Clipboard")),
        "Running Action 3 (Stdout/Clipboard) closed Verbocity or missed Terminal Output log"
    );
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT))?;
    ensure!(
        !step5_app.should_quit && step8_md_out.is_file(),
        "Running Action 4 (Save .md) closed Verbocity or failed to write markdown file"
    );

    // Action 1 (`agy -i` Interactive Mode): executes child handoff and returns to active App loop with `should_quit == false`
    step5_app.handle_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT))?;
    ensure!(
        !step5_app.should_quit
            && step5_app.terminal_output.last_exit_code == Some(0)
            && step5_app
                .terminal_output
                .lines
                .iter()
                .any(|l| l.contains("STEP8_AGY_STREAM: -i --model"))
            && step5_app.values.objective == complex_feature_prompt,
        "Running Action 1 (agy -i) closed Verbocity or lost prompt field text on resume"
    );
    println!("  [OK] Persistent execution loop verified: agy -p, agy -i handoff, Stdout/Clipboard, and Save .md keep Verbocity open and stream into Terminal Output Pane (Ctrl+O)");

    // 31. Verify Step 8 CLI Modifiers (`-i`, `-p`, `-s`, `-c`, `-m`, `-e`, `--mode`, `--sandbox`, `--continue`, `--ingest-model`, and trailing positional `[OBJECTIVE]...`)
    let parsed_cli_print = Cli::try_parse_from([
        "verbot",
        "-p",
        "-c",
        "grill-me",
        "-m",
        "Opus",
        "-e",
        "xhigh",
        "--mode",
        "accept-edits",
        "--sandbox",
        "--continue",
        "Fix",
        "scaling",
        "on",
        "branding",
        "and",
        "fields",
    ])
    .context("Failed to parse Step 8 CLI short/long modifiers and trailing objective args")?;
    ensure!(
        parsed_cli_print.print
            && !parsed_cli_print.interactive
            && parsed_cli_print.slash_command.as_deref() == Some("grill-me")
            && parsed_cli_print.model.as_deref() == Some("Opus")
            && parsed_cli_print.effort.as_deref() == Some("xhigh")
            && parsed_cli_print.mode.as_deref() == Some("accept-edits")
            && parsed_cli_print.sandbox
            && parsed_cli_print.continue_session
            && parsed_cli_print.objective_args.join(" ")
                == "Fix scaling on branding and fields",
        "Step 8 Cli::try_parse_from mismatch on short/long modifiers or trailing objective args"
    );

    let parsed_cli_settings = Cli::try_parse_from([
        "verbot",
        "-i",
        "-s",
        "--ingest-model",
        &external_model_toml.to_string_lossy(),
    ])
    .context("Failed to parse -i -s --ingest-model CLI flags")?;
    ensure!(
        parsed_cli_settings.interactive
            && parsed_cli_settings.settings
            && parsed_cli_settings.ingest_model == Some(external_model_toml.clone()),
        "Step 8 Cli::try_parse_from mismatch on -i -s --ingest-model"
    );

    // End-to-end subprocess test of `bin/verbot` with `-p -c plan -m gemini-4-argon-medium -e high --mode plan --sandbox` and trailing positional objective words
    let cli_e2e_manifest_path = tmp_test_root.join("step8-cli-manifest.json");
    let cli_e2e_out = std::process::Command::new(&current_exe)
        .env("VERBOT_AGY_DRY_RUN", "1")
        .args([
            "--headless",
            "-p",
            "-c",
            "plan",
            "-m",
            "gemini-4-argon-medium",
            "-e",
            "high",
            "--mode",
            "plan",
            "--sandbox",
            "--dispatch-out",
            &cli_e2e_manifest_path.to_string_lossy(),
            "Audit",
            "persistent",
            "terminal",
            "execution",
            "loop",
        ])
        .output()
        .context("Failed to execute verbot-tui with Step 8 CLI modifiers")?;
    ensure!(
        cli_e2e_out.status.success() && cli_e2e_manifest_path.is_file(),
        "Step 8 CLI modifier subprocess failed: {}",
        String::from_utf8_lossy(&cli_e2e_out.stderr)
    );
    let cli_e2e_json: DispatchManifest = serde_json::from_str(
        &std::fs::read_to_string(&cli_e2e_manifest_path)?,
    )?;
    ensure!(
        cli_e2e_json.action == "agy-print"
            && cli_e2e_json.slash_command.as_deref() == Some("/plan")
            && cli_e2e_json.model_id == "gemini-4-argon-high"
            && cli_e2e_json.effort == "high"
            && cli_e2e_json
                .rendered_prompt
                .contains("/plan Audit persistent terminal execution loop")
            && cli_e2e_json.agy_extra_flags
                == vec!["--mode".to_string(), "plan".to_string(), "--sandbox".to_string()],
        "Step 8 CLI modifier DispatchManifest mismatch: {:?}",
        cli_e2e_json
    );
    println!("  [OK] Step 8 CLI modifiers (-i, -p, -s, -c, -m, -e, --mode, --sandbox, --continue, --ingest-model, [OBJECTIVE]...) verified");

    // 32. Verify `install.sh --dry-run` and isolated prefix + idempotent shell profile `PATH` setup
    let dry_install_out = std::process::Command::new(repo_root.join("install.sh"))
        .arg("--dry-run")
        .output()
        .context("Failed to run ./install.sh --dry-run")?;
    ensure!(
        dry_install_out.status.success()
            && String::from_utf8_lossy(&dry_install_out.stdout).contains("Dry-run complete"),
        "./install.sh --dry-run failed"
    );

    let fake_shell_home = tmp_test_root.join("fake_shell_home");
    std::fs::create_dir_all(&fake_shell_home)?;
    // Run install.sh twice against `fake_shell_home` to verify idempotency of `# Verbocity (verbot) PATH`
    for _ in 0..2 {
        let rc_out = std::process::Command::new(repo_root.join("install.sh"))
            .env("VERBOT_TUI_BIN", &current_exe)
            .args([
                "--skip-build",
                "--prefix",
                &stage_prefix.to_string_lossy(),
                "--config-dir",
                &stage_config.to_string_lossy(),
                "--shell-rc-dir",
                &fake_shell_home.to_string_lossy(),
            ])
            .output()
            .context("Failed to run install.sh with --shell-rc-dir")?;
        ensure!(
            rc_out.status.success(),
            "install.sh with --shell-rc-dir failed: {}",
            String::from_utf8_lossy(&rc_out.stderr)
        );
    }

    for rc_name in [".bashrc", ".zshrc", ".profile"] {
        let rc_path = fake_shell_home.join(rc_name);
        let rc_content = std::fs::read_to_string(&rc_path)
            .with_context(|| format!("Missing {} after install.sh", rc_path.display()))?;
        let marker_count = rc_content.matches("# Verbocity (verbot) PATH").count();
        ensure!(
            marker_count == 1 && rc_content.contains("export PATH="),
            "Expected exactly 1 idempotent '# Verbocity (verbot) PATH' block in {}, found {}",
            rc_name,
            marker_count
        );
    }
    ensure!(
        stage_config.join("config.toml").is_file()
            && stage_config.join("theme.toml").is_file()
            && stage_config.join("models.toml").is_file(),
        "install.sh did not create default config.toml, theme.toml, and models.toml"
    );
    println!("  [OK] install.sh --dry-run, default configs (config.toml, theme.toml, models.toml), and idempotent shell PATH setup (.bashrc, .zshrc, .profile) verified");

    println!("\n==> [Verbocity Self-Test] Step 9: Git Repository Hygiene, .gitignore, curl | bash Remote Installer, Automated Packaging & Documentation");

    // 33. Verify `.gitignore`, absence of `verbot-prompt.md` and duplicate root `GeminiAgentPromptTemplate.md`,
    //     and zero hardcoded local user directory paths across tracked/workspace files.
    let gitignore_path = repo_root.join(".gitignore");
    let gitignore_raw = std::fs::read_to_string(&gitignore_path)
        .with_context(|| format!("Missing .gitignore at {}", gitignore_path.display()))?;
    for required_ignore in [
        "/target/",
        "/.verbot/",
        "/dist/",
        "/packages/",
        "*.tar.gz",
        "*.deb",
        "/verbot-prompt.md",
    ] {
        ensure!(
            gitignore_raw.contains(required_ignore),
            ".gitignore missing required entry '{}'",
            required_ignore
        );
    }
    ensure!(
        !repo_root.join("verbot-prompt.md").exists(),
        "Scratch file verbot-prompt.md must be removed from repository root"
    );
    ensure!(
        !repo_root.join("GeminiAgentPromptTemplate.md").exists()
            && repo_root
                .join("templates/GeminiAgentPromptTemplate.md")
                .is_file(),
        "Root GeminiAgentPromptTemplate.md must be deduplicated in favor of templates/GeminiAgentPromptTemplate.md"
    );

    let audit_index = WorkspaceFileIndex::scan(&repo_root)?;
    let forbidden_user_prefix = format!("/{}/", ["ho", "me"].concat());
    let placeholder_dots = format!("/{}/...", ["ho", "me"].concat());
    let placeholder_user = format!("/{}/<user>", ["ho", "me"].concat());
    let placeholder_tick = format!("`/{}/`", ["ho", "me"].concat());
    let forbidden_screenshot = ["Screen", "shot_"].concat();
    for entry in &audit_index.files {
        let full_path = repo_root.join(&entry.relative_path);
        if let Ok(content) = std::fs::read_to_string(&full_path) {
            for (line_no, line) in content.lines().enumerate() {
                if line.contains(&forbidden_user_prefix) {
                    let is_generic_doc = entry.relative_path == "STEPPED_IMPLEMENTATION_PLAN.md"
                        && (line.contains(&placeholder_tick)
                            || line.contains(&placeholder_dots)
                            || line.contains(&placeholder_user));
                    ensure!(
                        is_generic_doc,
                        "Hardcoded local user path found in {}:{} -> {}",
                        entry.relative_path,
                        line_no + 1,
                        line.trim()
                    );
                }
                ensure!(
                    !line.contains(&forbidden_screenshot),
                    "Local screenshot reference found in {}:{} -> {}",
                    entry.relative_path,
                    line_no + 1,
                    line.trim()
                );
            }
        }
    }
    println!(
        "  [OK] Repository hygiene verified (.gitignore present, verbot-prompt.md removed, {} tracked files audited with 0 hardcoded local paths)",
        audit_index.files.len()
    );

    // 34. Verify `scripts/package.sh` syntax (`bash -n`) and execution (`--skip-build --out-dir`)
    let package_script = repo_root.join("scripts/package.sh");
    ensure!(
        package_script.is_file(),
        "Missing scripts/package.sh at {}",
        package_script.display()
    );
    let pkg_syntax = std::process::Command::new("bash")
        .arg("-n")
        .arg(&package_script)
        .status()
        .context("Failed to run bash -n on scripts/package.sh")?;
    ensure!(pkg_syntax.success(), "bash -n failed for scripts/package.sh");

    let pkg_out_dir = tmp_test_root.join("dist_test");
    let pkg_out = std::process::Command::new(&package_script)
        .env("VERBOT_TUI_BIN", &current_exe)
        .args([
            "--skip-build",
            "--out-dir",
            &pkg_out_dir.to_string_lossy(),
        ])
        .output()
        .context("Failed to execute scripts/package.sh --skip-build")?;
    ensure!(
        pkg_out.status.success(),
        "scripts/package.sh failed: {}",
        String::from_utf8_lossy(&pkg_out.stderr)
    );
    let sums_path = pkg_out_dir.join("SHA256SUMS");
    ensure!(
        sums_path.is_file(),
        "scripts/package.sh did not generate SHA256SUMS"
    );
    let mut found_tarball: Option<PathBuf> = None;
    for entry in std::fs::read_dir(&pkg_out_dir)? {
        let p = entry?.path();
        if p.file_name()
            .and_then(|n| n.to_str())
            .map(|n| n.ends_with(".tar.gz"))
            .unwrap_or(false)
        {
            found_tarball = Some(p);
            break;
        }
    }
    let tarball_path = found_tarball.context("scripts/package.sh did not produce a .tar.gz archive")?;
    ensure!(
        repo_root.join("packaging/arch/PKGBUILD").is_file()
            && repo_root.join("packaging/rpm/verbot.spec").is_file(),
        "Missing packaging/arch/PKGBUILD or packaging/rpm/verbot.spec"
    );
    println!(
        "  [OK] scripts/package.sh & packaging specs verified (archive={}, SHA256SUMS, PKGBUILD, verbot.spec)",
        tarball_path.file_name().unwrap_or_default().to_string_lossy()
    );

    // 35. Verify `curl | bash` piped remote installer mode (`cat install.sh | bash -s -- --from-archive ...`)
    //     where `BASH_SOURCE[0]` is unbound and installation runs from an arbitrary temporary directory.
    let curl_stage_prefix = tmp_test_root.join("curl_pipe_prefix");
    let curl_stage_config = tmp_test_root.join("curl_pipe_config");
    let curl_empty_cwd = tmp_test_root.join("curl_empty_cwd");
    std::fs::create_dir_all(&curl_empty_cwd)?;
    let install_script_bytes = std::fs::read(repo_root.join("install.sh"))?;

    let mut curl_pipe_child = std::process::Command::new("bash")
        .current_dir(&curl_empty_cwd)
        .args([
            "-s",
            "--",
            "--from-archive",
            &tarball_path.to_string_lossy(),
            "--no-modify-path",
            "--prefix",
            &curl_stage_prefix.to_string_lossy(),
            "--config-dir",
            &curl_stage_config.to_string_lossy(),
        ])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()
        .context("Failed to spawn bash for piped install.sh test")?;

    if let Some(mut stdin_pipe) = curl_pipe_child.stdin.take() {
        use std::io::Write;
        stdin_pipe.write_all(&install_script_bytes)?;
    }
    let curl_pipe_out = curl_pipe_child.wait_with_output()?;
    ensure!(
        curl_pipe_out.status.success(),
        "Piped `cat install.sh | bash -s -- --from-archive` failed: {}",
        String::from_utf8_lossy(&curl_pipe_out.stderr)
    );
    ensure!(
        curl_stage_prefix.join("bin/verbot").is_file()
            && curl_stage_prefix.join("bin/verbot-tui").is_file()
            && curl_stage_prefix
                .join("share/verbot/scripts/dispatch-agy.sh")
                .is_file()
            && curl_stage_prefix
                .join("share/icons/hicolor/scalable/apps/verbot.svg")
                .is_file()
            && curl_stage_prefix
                .join("share/applications/verbot.desktop")
                .is_file()
            && curl_stage_config.join("config.toml").is_file()
            && curl_stage_config.join("theme.toml").is_file()
            && curl_stage_config.join("models.toml").is_file()
            && curl_stage_config
                .join("templates/GeminiAgentPromptTemplate.md")
                .is_file(),
        "Piped `curl | bash` installer failed to populate all target binaries, scripts, icons, or configs"
    );
    println!("  [OK] Piped `curl | bash` remote installation (`bash -s -- --from-archive`) verified without BASH_SOURCE[0]");

    // 36. Verify GitHub Actions CI/Release Workflows (`.github/workflows/ci.yml`, `.github/workflows/release.yml`)
    //     and `README.md` instructional completeness.
    for wf_rel in [".github/workflows/ci.yml", ".github/workflows/release.yml"] {
        let wf_path = repo_root.join(wf_rel);
        let wf_raw = std::fs::read_to_string(&wf_path)
            .with_context(|| format!("Missing workflow file {}", wf_path.display()))?;
        let wf_val: serde_yaml::Value = serde_yaml::from_str(&wf_raw)
            .with_context(|| format!("Invalid YAML syntax in {}", wf_rel))?;
        ensure!(
            wf_val.get("jobs").is_some(),
            "Workflow {} missing 'jobs' key",
            wf_rel
        );
    }
    let readme_raw = std::fs::read_to_string(repo_root.join("README.md"))
        .context("Missing README.md")?;
    for required_doc_section in [
        "curl -fsSL https://raw.githubusercontent.com/InfoMoose/VerBot/main/install.sh | bash",
        "Complete CLI Flags & Modifiers Reference",
        "Global & Editor Keybindings Table",
        "Settings Page (`Alt+S`) & Extending Verbocity",
        "Packaging & Release Automation Guide",
        "--ingest-model",
        "scripts/package.sh",
    ] {
        ensure!(
            readme_raw.contains(required_doc_section),
            "README.md missing required instructional section or reference: '{}'",
            required_doc_section
        );
    }
    println!("  [OK] GitHub Actions workflows (ci.yml, release.yml) YAML validated and README.md instructional manual verified");

    let _ = std::fs::remove_dir_all(&tmp_test_root);

    println!("==> [Verbocity Self-Test] ALL STEP 1, STEP 2, STEP 3, STEP 4, STEP 5, STEP 6, STEP 7, STEP 8 & STEP 9 CHECKS PASSED.");
    Ok(())
}
