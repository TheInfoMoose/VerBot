# Verbocity (`verbot`)

<p align="center">
  <img src="assets/verbot-logo.svg" alt="Verbocity Logo" width="140" />
</p>

**Verbocity** (invoked on the terminal as `verbot`) is an interactive, multi-pane terminal prompt template engine and Antigravity (`agy`) CLI dispatcher with configurable style/theme presets and automatic desktop theme integration.

It enables engineers, technicians, and power users to compose concise, high-density structured prompts (`Objective`, `Context/Logs`, `Constraints`, `Output`), dynamically format them for **Gemini**, **Claude**, or **GPT** model families without losing entered text, inspect a live shell-safe `agy` command preview, and dispatch directly into an in-app **Terminal Execution Output Pane** (`agy -p`) or an interactive **`agy -i`** handoff without closing Verbocity.

---

## Table of Contents

1. [Overview](#1-overview)
2. [Installation](#2-installation)
   - [One-Line Remote `curl | bash` Installer](#one-line-remote-curl--bash-installer)
   - [Local Repository Build & Install](#local-repository-build--install)
   - [System Packages (`.deb`, Portable `.tar.gz`, Arch `PKGBUILD`, RPM)](#system-packages-deb-portable-targz-arch-pkgbuild-rpm)
3. [Instructional Usage Guide & CLI Reference](#3-instructional-usage-guide--cli-reference)
   - [Quick Start & Common Workflows](#quick-start--common-workflows)
   - [Piped `stdin` Log & Command Output Capture](#piped-stdin-log--command-output-capture)
   - [Headless & Scripted Automation](#headless--scripted-automation)
   - [Complete CLI Flags & Modifiers Reference](#complete-cli-flags--modifiers-reference)
4. [Interactive TUI Walkthrough & Keybindings](#4-interactive-tui-walkthrough--keybindings)
   - [Interface Layout](#interface-layout)
   - [Global & Editor Keybindings Table](#global--editor-keybindings-table)
   - [Fuzzy File Picker (`@` / `Ctrl+F`) & `.md` Plan Approval Gate](#fuzzy-file-picker--ctrlf--md-plan-approval-gate)
   - [Persistent Execution Loop & In-App Terminal Output Pane (`Ctrl+O`)](#persistent-execution-loop--in-app-terminal-output-pane-ctrlo)
5. [Settings Page (`Alt+S`) & Extending Verbocity](#5-settings-page-alts--extending-verbocity)
   - [Tab 1: Theme & Contrast (`~/.config/verbot/theme.toml`)](#tab-1-theme--contrast-configverbotthemetoml)
   - [Tab 2: Models & TOML Ingestion (`~/.config/verbot/models.toml`)](#tab-2-models--toml-ingestion-configverbotmodelstoml)
   - [Tab 3: Templates & Startup Defaults (`verbot ingest`)](#tab-3-templates--startup-defaults-verbot-ingest)
   - [Tab 4: Execution Flags, Shell Formatting & Workspace Storage](#tab-4-execution-flags-shell-formatting--workspace-storage)
6. [Packaging & Release Automation Guide](#6-packaging--release-automation-guide)

---

## 1. Overview

- **Application Name**: **Verbocity**
- **CLI Command**: `verbot` (backed by the compiled Rust core `verbot-tui` and modular dispatch scripts `dispatch-agy.sh` / `dispatch-output.sh`)
- **Decoupled Multi-Model Prompt Engine**: Maintains canonical prompt field values (`Objective`, `Context/Logs`, `Constraints`, `Output`, plus custom template fields) in memory while dynamically rendering output into model-specific formats (`GeminiConcise`, `ClaudeXml`, `GptMarkdownSections`, or custom templates).
- **Persistent In-App Execution & Interactive Handoff**: Streams `agy -p` output directly into an in-app scrollable terminal pane (`Ctrl+O`) and cleanly suspends/resumes `/dev/tty` for interactive `agy -i` sessions without losing your prompt state.
- **Configurable Style & Theme Options**: Supports live theme preset cycling (`Ctrl+T`), automatic desktop dark/light & accent color detection, `Terminal ANSI` passthrough, and custom hex color token overrides persisted in `~/.config/verbot/theme.toml`.

---

## 2. Installation

### One-Line Remote `curl | bash` Installer

Install the latest prebuilt release (or automatically fall back to building from source if no prebuilt archive matches your architecture):

```bash
curl -fsSL https://raw.githubusercontent.com/TheInfoMoose/VerBot/main/install.sh | bash
```

Pass installer options after `bash -s --`:

```bash
# Force building from source via cargo:
curl -fsSL https://raw.githubusercontent.com/TheInfoMoose/VerBot/main/install.sh | bash -s -- --from-source

# Install a specific release tag into a custom prefix and symlink into /usr/local/bin:
curl -fsSL https://raw.githubusercontent.com/TheInfoMoose/VerBot/main/install.sh | bash -s -- --version v0.1.0 --prefix "$HOME/.local" --system
```

### Local Repository Build & Install

Clone the repository and run `./install.sh`:

```bash
git clone https://github.com/TheInfoMoose/VerBot.git
cd VerBot
./install.sh
```

What `./install.sh` sets up:
1. Builds `target/release/verbot-tui` via `cargo build --release` (unless `--skip-build` or a prebuilt bundle is used).
2. Installs `verbot` and `verbot-tui` into `~/.local/bin` (and optionally symlinks `/usr/local/bin/{verbot,verbot-tui}` with `--system`).
3. Idempotently ensures `export PATH="$HOME/.local/bin:$PATH"` is present in `~/.bashrc`, `~/.zshrc`, `~/.profile` (and `fish_add_path` in `~/.config/fish/config.fish`).
4. Installs modular dispatch scripts into `~/.local/share/verbot/scripts/`.
5. Installs the desktop icon (`~/.local/share/icons/hicolor/scalable/apps/verbot.svg`) and desktop launcher (`~/.local/share/applications/verbot.desktop`).
6. Initializes default `config.toml`, `theme.toml`, `models.toml`, and `templates/GeminiAgentPromptTemplate.{md,toml}` inside `~/.config/verbot/` without overwriting existing user customizations.

### System Packages (`.deb`, Portable `.tar.gz`, Arch `PKGBUILD`, RPM)

- **Debian / Ubuntu (`.deb`)**:
  ```bash
  ./scripts/package.sh
  sudo dpkg -i dist/verbot_0.1.0_amd64.deb
  ```
- **Portable Tarball (`.tar.gz`)**:
  ```bash
  tar -xzf dist/verbot-v0.1.0-x86_64-unknown-linux-gnu.tar.gz
  ./verbot-v0.1.0-x86_64-unknown-linux-gnu/install.sh --skip-build
  ```
- **Arch Linux (`packaging/arch/PKGBUILD`)**:
  ```bash
  cd packaging/arch && makepkg -si
  ```
- **Fedora / RHEL / openSUSE (`packaging/rpm/verbot.spec`)**:
  ```bash
  rpmbuild -ba packaging/rpm/verbot.spec
  ```

---

## 3. Instructional Usage Guide & CLI Reference

### Quick Start & Common Workflows

```bash
# 1. Launch interactive Verbocity TUI in your current working directory:
verbot

# 2. Launch with /plan pre-selected, Opus high effort, and initial Objective text:
verbot -c plan -m Opus -e high Audit boot service ordering and initramfs hooks

# 3. Pre-select `agy -p` (Print mode) with --mode plan and --sandbox enabled:
verbot -p -c boost --mode plan --sandbox Refactor state serialization in src/state.rs

# 4. Launch directly into the full-screen Settings Page (Alt+S):
verbot -s
```

### Piped `stdin` Log & Command Output Capture

Pipe any command output or log stream directly into `verbot`. Verbocity captures `stdin` into the **`Context/Logs`** editor field and reconnects the interactive TUI to `/dev/tty`:

```bash
# Pipe recent systemd journal failures into Verbocity's Context/Logs pane:
journalctl -n 50 --no-pager | verbot -c plan Diagnose service startup failure

# Pipe failing cargo test output into Verbocity:
cargo test 2>&1 | verbot -c goal Fix failing unit tests
```

### Headless & Scripted Automation

Use `--headless` for non-interactive prompt rendering, CI checks, or direct scripted dispatch:

```bash
# Render a structured Gemini Argon prompt to stdout:
verbot --headless -c plan -m gemini-4-argon-medium -e high \
  --objective "Isolate display manager race condition" \
  --constraints "Read-only diagnostic mode; do not modify files" \
  --output "Diagnose only" \
  --action stdout

# Save rendered prompt directly to a Markdown file:
verbot --headless -c goal --action save-md --save-md ./sprint-task.md \
  "Implement automated release packaging"
```

### Complete CLI Flags & Modifiers Reference

| Flag / Modifier | Short | Description |
| :--- | :--- | :--- |
| `--interactive` | `-i` | Pre-select (or in `--headless` mode dispatch) Action 1: `Run agy -i (Interactive)` |
| `--print` | `-p` | Pre-select (or in `--headless` mode dispatch) Action 2: `Run agy -p (Print)` |
| `--settings` | `-s` | Open Verbocity directly on the full-screen **Settings Page (`Alt+S`)** |
| `--command <CMD>` | `-c` | Set the active `agy` slash command (`plan`, `boost`, `goal`, `grill-me`, `learn`, `compact`, `schedule`, `automation`, `browser`, `plugin`, `teamwork-preview`, or `none`) |
| `--model <MODEL>` | `-m` | Select model family, short name (`Argon`, `Flash`, `Opus`, `Sonnet`, `GPT-OSS 120B`), or full `agy` model ID (`gemini-4-argon-high`) |
| `--effort <LEVEL>` | `-e` | Set reasoning/thinking effort (`low`, `medium`, `high`, `xhigh`, `max`) |
| `--mode <MODE>` | | Pass `--mode` (`auto`, `plan`, `accept-edits`, `none`) to `agy` |
| `--sandbox` | | Enable `--sandbox` flag when invoking `agy` |
| `--continue` | | Pass `--continue` to `agy` to resume the most recent conversation |
| `--ingest-model <FILE>` | | Validate and import a custom model `.toml` or `.yaml` file into `~/.config/verbot/models.toml` |
| `--template <FILE>` | `-t` | Load a specific `.md`, `.toml`, or `.yaml` prompt template file |
| `--theme <PRESET>` | | Override active style/theme preset (dark, light, system auto, or `Terminal ANSI`) |
| `--local` | | Store workspace state in project-local `./.verbot/state.toml` instead of XDG data cache |
| `--objective <TEXT>` | | Populate the `Objective` field |
| `--context <TEXT>` | | Populate or prepend to the `Context/Logs` field |
| `--constraints <TEXT>` | | Populate the `Constraints` field (automatically merged with pinned directory constraints) |
| `--output <TEXT>` | | Populate the `Output` field |
| `--headless` | | Run non-interactively and emit the rendered prompt or dispatch manifest immediately |
| `--action <ACTION>` | | Action for `--headless` mode (`agy-interactive`, `agy-print`, `stdout`, `save-md`) |
| `--save-md <PATH>` | | Target file path when saving Markdown (`verbot-prompt.md` by default) |
| `--self-test` | | Execute the full automated diagnostic and headless TUI verification suite |
| `[OBJECTIVE]...` | | Trailing positional words/sentences are automatically joined into the `Objective` field |
| `ingest <PATH>` | | Subcommand: validate and import a template into `~/.config/verbot/templates/` (supports `--local`, `--name`, `--dry-run`) |

---

## 4. Interactive TUI Walkthrough & Keybindings

### Interface Layout

1. **Top Header**: Displays the responsive `VERBOCITY` header bar scaled to the active terminal dimensions.
2. **Control Bar**: Quick selectors for `Family` (`Gemini`, `Claude`, `GPT`), `Sub-Model (Alt+M)`, `Effort (Alt+E)`, `Command`, and `Settings (Alt+S)`.
3. **4 Soft-Wrapping Multiline Field Editors**:
   - `Objective`: Primary task or symptom description.
   - `Context/Logs`: Relevant file paths (insertable via `@` or `Ctrl+F`) and piped terminal logs.
   - `Constraints`: Architectural guardrails and per-directory pinned rules (`Ctrl+K`).
   - `Output`: Expected deliverable with quick-choice cycling (`Alt+C`: `Diagnose only`, `Update <file>.md plan`, `Execute Step X`).
4. **Live Prompt Preview (`Ctrl+P`) & Terminal Execution Output Pane (`Ctrl+O`)**:
   - Real-time model-formatted prompt preview (`GeminiConcise` bullets, `ClaudeXml` tags, or `GptMarkdownSections` headers), word/token conciseness estimator, and scrollable live output from executed `agy` runs.
5. **Action Bar & `agy` Command Footer**:
   - Displays the exact POSIX-quoted `agy` CLI command (`agy --model <MODEL> --effort <EFFORT> [FLAGS...] <-p|-i> "<PROMPT>"`) and lets you trigger actions `1..4` without closing Verbocity.

### Global & Editor Keybindings Table

| Shortcut | Scope | Action |
| :--- | :--- | :--- |
| `Tab` / `Shift+Tab` | Global | Cycle focus across Control Bar, Field Editors (`Objective` -> `Context/Logs` -> `Constraints` -> `Output`), Terminal Output Pane, and Action Bar |
| `Alt+S` | Global | Open / close the full-screen **Settings Page** (preserving all prompt text) |
| `Alt+M` | Global | Cycle Sub-Model (`Argon`, `Flash`, `Opus`, `Sonnet`, `GPT-OSS 120B`, plus custom models) |
| `Alt+E` | Global | Cycle Reasoning / Thinking Effort (`low`, `medium`, `high`, `xhigh`, `max`) |
| `Ctrl+T` | Global | Cycle active Theme / Style Preset live (dark, system auto, light, `Terminal ANSI`) |
| `Ctrl+P` | Global | Toggle collapsible **Live Prompt Preview** pane |
| `Ctrl+O` | Global | Toggle **Terminal Execution Output Pane** (`FocusPane::TerminalOutput`) |
| `@` or `Ctrl+F` | Field Editor | Open **CWD Fuzzy File Picker Modal** to filter and insert workspace file paths |
| `Alt+C` | Field Editor | Cycle template quick-choices for the focused field (e.g., `Output` choices) |
| `Ctrl+K` | Constraints / Global | Pin or unpin the current constraint for the active working directory |
| `Ctrl+H` | Global | Cycle through recent prompt history for the current workspace |
| `Ctrl+S` | Global | Immediately save the rendered prompt to `verbot-prompt.md` (or `--save-md` path) |
| `Alt+1` .. `Alt+4` | Global | Immediately execute Action 1 (`agy -i`), Action 2 (`agy -p`), Action 3 (`Stdout/Clipboard`), or Action 4 (`Save .md`) |
| `Alt+U` | Global | Open quick **Model Catalog Sync & Add Custom Sub-Model Modal** (`Ctrl+R` syncs from `agy models`) |
| `Ctrl+Q` | Global | Quit Verbocity |

### Fuzzy File Picker (`@` / `Ctrl+F`) & `.md` Plan Approval Gate

- Pressing `@` or `Ctrl+F` inside any field opens a `.gitignore`-aware fuzzy file picker rooted in your current working directory. Type to filter, use `Up/Down` to select, and press `Enter` to insert the relative path at the cursor.
- When cycling `Output` choices (`Alt+C`) to **`Update <file>.md plan`**, Verbocity stages an explicit approval banner (`Enter` / `Alt+Y` to open the `.md` plan file picker, or `Esc` / `Alt+N` to keep the literal `<file>.md` token) so the modal never interrupts unexpectedly.

### Persistent Execution Loop & In-App Terminal Output Pane (`Ctrl+O`)

Verbocity **does not close** when you execute an action—allowing you to iterate on prompts, inspect output, and re-run commands in a single session:
- **Action 1 (`Run agy -i Interactive`)**: Temporarily suspends the Ratatui alternate screen, hands `/dev/tty` cleanly to the interactive `agy` session (`agy --model <ID> --effort <EFFORT> [FLAGS...] -i "<PROMPT>"`) in your working directory, waits for `agy` to exit, and immediately resumes Verbocity with all fields and history intact.
- **Action 2 (`Run agy -p Print`)**: Opens the in-app **Terminal Execution Output Pane (`Ctrl+O`)** and streams `stdout`/`stderr` from `agy --model <ID> --effort <EFFORT> [FLAGS...] -p "<PROMPT>"` live into a scrollable buffer (`Up/Down`/`PgUp/PgDn`/`End`, `Ctrl+L` to clear).
- **Action 3 (`Output to Stdout / Clipboard`)**: Copies the rendered prompt to your Wayland/X11 clipboard (`wl-copy`, `xclip`, or `xsel`) and logs it in the Terminal Output Pane.
- **Action 4 (`Save to .md File`)**: Writes the rendered prompt to `verbot-prompt.md` (or custom path) and records confirmation in the Terminal Output Pane.

---

## 5. Settings Page (`Alt+S`) & Extending Verbocity

Press **`Alt+S`** (or launch with `verbot -s`) to open the 4-tab Settings Page. Press `1..4` (or `Alt+1..4` / `Left/Right`) to switch tabs and `Esc` to return to the prompt editor.

### Tab 1: Theme & Contrast (`~/.config/verbot/theme.toml`)

Select a style preset (dark, system auto-detected, light, or `Terminal ANSI`) or customize individual hex color tokens (`accent_primary`, `accent_secondary`, `text_primary`, `title_inactive`, `border_slate`, `placeholder_muted`) and press `Enter` to apply and persist to `~/.config/verbot/theme.toml`.

### Tab 2: Models & TOML Ingestion (`~/.config/verbot/models.toml`)

You can extend Verbocity's model catalog in three ways:
1. **Sync from `agy models` (`Ctrl+R`)**: Queries your installed `agy models` CLI and merges discovered models automatically.
2. **Ingest External `.toml` / `.yaml` File**: Enter a file path in Settings Tab 2 (or run `verbot --ingest-model ./my-models.toml`).
3. **Interactive Custom Sub-Model Form**: Fill out `Family`, `Short Name`, `Display Name`, `Base agy ID`, `Available Tiers`, and `Default Effort` directly in Tab 2.

Example single-profile or multi-profile model TOML file (`my-models.toml`):

```toml
[[profiles]]
family = "Gemini"
short_name = "Hyperion"
display_name = "Gemini 4 Hyperion"
agy_base_id = "gemini-4-hyperion"
available_tiers = ["low", "medium", "high"]
default_effort = "High"
format_style = "GeminiConcise"

[[profiles]]
family = "Claude"
short_name = "Haiku"
display_name = "Claude Haiku 5.5"
agy_base_id = "claude-haiku-5-5"
available_tiers = ["low", "medium", "high"]
default_effort = "Medium"
format_style = "ClaudeXml"
```

### Tab 3: Templates & Startup Defaults (`verbot ingest`)

Import custom Markdown or TOML templates globally (`~/.config/verbot/templates/`) or per-project (`./.verbot/templates/`):

```bash
# Preview template parsing and generated companion .toml metadata:
verbot ingest ./templates/GeminiAgentPromptTemplate.md --dry-run

# Import a template into ~/.config/verbot/templates/:
verbot ingest ./my-team-template.md --name team-prompt
```

Tab 3 also configures startup defaults (`default_model_family`, `default_sub_model`, `default_effort`, `default_slash_command`, `show_preview_on_startup`) in `~/.config/verbot/config.toml`.

### Tab 4: Execution Flags, Shell Formatting & Workspace Storage

- **Default `agy` CLI Flags**: Configure persistent defaults for `--mode` (`None`, `auto`, `plan`, `accept-edits`), `--sandbox`, and `--continue`.
- **Shell Metacharacter Formatting Policy (`src/formatting.rs`)**:
  - `PromptOnSpecialChars` *(Default)*: Inspects prompt fields for shell-sensitive characters (`"`, `'`, `$`, `` ` ``, `*`, `(`, `)`, `;`, `|`, `&`, newlines) and opens a confirmation modal on dispatch to POSIX-quote/escape the prompt safely.
  - `AutoEscapeSilently`: Automatically wraps and escapes all arguments using POSIX single-quote escaping without prompting.
  - `RawLiteral`: Emits unescaped text.
- **Workspace Storage Mode**: Defaults to clean **XDG Data Cache** (`~/.local/share/verbot/workspaces/`) so project directories are never polluted with `.verbot/` unless toggled to **Project-Local (`./.verbot/state.toml`)** or `--local` is passed.

---

## 6. Packaging & Release Automation Guide

### Local Packaging (`scripts/package.sh`)

Build all distribution packages locally into `./dist`:

```bash
# Compile release binary and generate .tar.gz, .deb, and SHA256SUMS:
./scripts/package.sh

# Package using an already-compiled target/release/verbot-tui binary:
./scripts/package.sh --skip-build --out-dir ./dist
```

Generated artifacts in `dist/`:
- `verbot-v<VERSION>-<ARCH>-unknown-linux-gnu.tar.gz` (portable bundle compatible with `install.sh --from-archive`)
- `verbot-v<VERSION>-<ARCH>-unknown-linux-gnu.tar.gz.sha256`
- `verbot_<VERSION>_<DEB_ARCH>.deb` (Debian/Ubuntu package installing to `/usr/bin` and `/usr/share`)
- `SHA256SUMS`

### Automated GitHub Actions Workflows

- **Continuous Integration (`.github/workflows/ci.yml`)**: Runs on every push and pull request to `main`, verifying `bash -n` across all shell scripts, `cargo test`, `cargo run -- --self-test`, and `./scripts/package.sh --skip-build`.
- **Release Publishing (`.github/workflows/release.yml`)**: Pushing a version tag (`git tag v0.1.0 && git push origin v0.1.0`) or triggering `workflow_dispatch` runs the full verification gate, builds the `.tar.gz`, `.deb`, `install.sh`, and `SHA256SUMS` artifacts, and publishes them to GitHub Releases for instant `curl | bash` installation.
