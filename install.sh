#!/usr/bin/env bash
# ==============================================================================
# Verbocity (`verbot`) — Local, Portable Archive & Remote `curl | bash` Installer
# ==============================================================================
# Supports three installation modes:
#   1. Local Checkout Mode (`./install.sh`):
#      Builds (via `cargo build --release`) and installs directly from the local
#      working tree or unpacked portable release archive.
#   2. Remote Prebuilt Release Mode (`curl -fsSL .../install.sh | bash`):
#      When piped into `bash` (no local checkout) and `--from-source` is not set,
#      detects host OS/architecture (`uname -s` / `uname -m`), downloads the
#      matching release tarball (`verbot-v<VERSION>-<ARCH>-unknown-linux-gnu.tar.gz`)
#      and `SHA256SUMS` from GitHub Releases, verifies SHA-256 integrity, and
#      installs binaries, dispatch scripts, SVG icon, COSMIC `.desktop` launcher,
#      and default configs into `~/.local` and `~/.config/verbot`.
#   3. Remote Source Fallback Mode (`curl -fsSL .../install.sh | bash -s -- --from-source`):
#      If no prebuilt release binary matches (or `--from-source` is passed),
#      downloads the source tarball (`https://github.com/${VERBOT_REPO}/archive/...`)
#      or shallow-clones via `git clone --depth 1` into an ephemeral `mktemp -d`
#      directory, runs `cargo build --release`, installs all components, and
#      cleans up on exit.
# ==============================================================================

set -euo pipefail

VERBOT_REPO="${VERBOT_REPO:-InfoMoose/VerBot}"
VERBOT_VERSION="${VERBOT_VERSION:-latest}"
VERBOT_GIT_REF="${VERBOT_GIT_REF:-main}"
VERBOT_GIT_URL="${VERBOT_GIT_URL:-https://github.com/${VERBOT_REPO}.git}"
VERBOT_RELEASE_BASE_URL="${VERBOT_RELEASE_BASE_URL:-https://github.com/${VERBOT_REPO}/releases}"

SKIP_BUILD=0
DRY_RUN=0
SYSTEM_SYMLINK=0
NO_MODIFY_PATH=0
FROM_SOURCE=0
FROM_ARCHIVE=""

PREFIX_DIR="${VERBOT_PREFIX:-${HOME}/.local}"
BIN_DIR="${VERBOT_BIN_DIR:-${PREFIX_DIR}/bin}"
DATA_DIR="${VERBOT_DATA_DIR:-${PREFIX_DIR}/share}"
CONFIG_DIR="${VERBOT_CONFIG_DIR:-${XDG_CONFIG_HOME:-${HOME}/.config}/verbot}"
SHELL_HOME="${VERBOT_SHELL_RC_DIR:-${HOME}}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build)
            SKIP_BUILD=1
            shift
            ;;
        --dry-run)
            DRY_RUN=1
            shift
            ;;
        --system)
            SYSTEM_SYMLINK=1
            shift
            ;;
        --no-modify-path)
            NO_MODIFY_PATH=1
            shift
            ;;
        --from-source)
            FROM_SOURCE=1
            shift
            ;;
        --from-archive)
            FROM_ARCHIVE="${2:-}"
            shift 2
            ;;
        --repo)
            VERBOT_REPO="${2:-}"
            VERBOT_GIT_URL="https://github.com/${VERBOT_REPO}.git"
            VERBOT_RELEASE_BASE_URL="https://github.com/${VERBOT_REPO}/releases"
            shift 2
            ;;
        --version)
            VERBOT_VERSION="${2:-}"
            shift 2
            ;;
        --prefix)
            PREFIX_DIR="$2"
            BIN_DIR="${PREFIX_DIR}/bin"
            DATA_DIR="${PREFIX_DIR}/share"
            shift 2
            ;;
        --config-dir)
            CONFIG_DIR="$2"
            shift 2
            ;;
        --shell-rc-dir)
            SHELL_HOME="$2"
            shift 2
            ;;
        -h|--help)
            cat <<'EOF'
Usage: ./install.sh [OPTIONS]
   or: curl -fsSL https://raw.githubusercontent.com/InfoMoose/VerBot/main/install.sh | bash
   or: curl -fsSL https://raw.githubusercontent.com/InfoMoose/VerBot/main/install.sh | bash -s -- [OPTIONS]

Options:
  --skip-build           Use existing target/release/verbot-tui or bundled binary without invoking cargo
  --from-source          Force remote source clone/tarball download and `cargo build --release`
  --from-archive <FILE>  Install from a local or pre-downloaded release `.tar.gz` archive
  --repo <OWNER/REPO>    GitHub repository slug for remote release/source downloads (default: InfoMoose/VerBot)
  --version <TAG>        Release version tag to install (default: latest, e.g. v0.1.0 or 0.1.0)
  --dry-run              Print planned installation steps without writing files
  --prefix <DIR>         Install prefix for bin/ and share/ (default: ~/.local)
  --config-dir <DIR>     Config directory for config.toml, theme.toml, models.toml, and templates/ (default: ~/.config/verbot)
  --shell-rc-dir <DIR>   Target home directory for .bashrc / .zshrc / .profile PATH updates (default: $HOME)
  --system               Also symlink verbot and verbot-tui into /usr/local/bin if writable
  --no-modify-path       Skip automatic shell profile PATH updates
  -h, --help             Show this help message
EOF
            exit 0
            ;;
        *)
            echo "Error [install.sh]: Unknown argument '$1'" >&2
            exit 1
            ;;
    esac
done

SCRIPTS_DEST="${DATA_DIR}/verbot/scripts"
ICON_DEST_DIR="${DATA_DIR}/icons/hicolor/scalable/apps"
APPS_DEST_DIR="${DATA_DIR}/applications"
TEMPLATES_DEST_DIR="${CONFIG_DIR}/templates"
CONFIG_TOML_DEST="${CONFIG_DIR}/config.toml"
THEME_TOML_DEST="${CONFIG_DIR}/theme.toml"
MODELS_TOML_DEST="${CONFIG_DIR}/models.toml"

# Safely determine script directory when invoked from a file vs piped via `curl | bash`
SCRIPT_SOURCE="${BASH_SOURCE[0]:-}"
LOCAL_CANDIDATE_ROOT=""
if [[ -n "$SCRIPT_SOURCE" && -f "$SCRIPT_SOURCE" ]]; then
    LOCAL_CANDIDATE_ROOT="$(cd "$(dirname "$SCRIPT_SOURCE")" && pwd)"
fi

is_valid_source_or_bundle_dir() {
    local d="$1"
    [[ -n "$d" && -d "$d" && -f "${d}/bin/verbot" ]] && (
        [[ -f "${d}/Cargo.toml" && -d "${d}/src" ]] ||
        [[ -x "${d}/bin/verbot-tui" && -d "${d}/share/verbot/scripts" ]]
    )
}

INSTALL_MODE="remote"
REPO_ROOT=""
if [[ -n "$FROM_ARCHIVE" ]]; then
    INSTALL_MODE="archive"
elif [[ "$FROM_SOURCE" -eq 0 ]] && is_valid_source_or_bundle_dir "$LOCAL_CANDIDATE_ROOT"; then
    INSTALL_MODE="local"
    REPO_ROOT="$LOCAL_CANDIDATE_ROOT"
elif [[ "$FROM_SOURCE" -eq 0 ]] && is_valid_source_or_bundle_dir "$(pwd)"; then
    INSTALL_MODE="local"
    REPO_ROOT="$(pwd)"
fi

echo "==> [Verbocity Installer] Mode: ${INSTALL_MODE}"
echo "==> [Verbocity Installer] Target directories:"
echo "    Binaries:    ${BIN_DIR} (verbot, verbot-tui)"
echo "    Scripts:     ${SCRIPTS_DEST}"
echo "    Icon:        ${ICON_DEST_DIR}/verbot.svg"
echo "    Desktop:     ${APPS_DEST_DIR}/verbot.desktop"
echo "    Config:      ${CONFIG_DIR} (config.toml, theme.toml, models.toml)"
echo "    Shell PATH:  ${SHELL_HOME}/{.bashrc,.zshrc,.profile}"

if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "==> [Verbocity Installer] Dry-run complete (no files modified)."
    exit 0
fi

TMP_WORK_DIR=""
cleanup_tmp() {
    if [[ -n "$TMP_WORK_DIR" && -d "$TMP_WORK_DIR" ]]; then
        rm -rf "$TMP_WORK_DIR"
    fi
}
trap cleanup_tmp EXIT

detect_target_arch() {
    local arch
    arch="$(uname -m 2>/dev/null || echo "x86_64")"
    case "$arch" in
        x86_64|amd64)
            echo "x86_64"
            ;;
        aarch64|arm64)
            echo "aarch64"
            ;;
        *)
            echo "$arch"
            ;;
    esac
}

verify_sha256_if_available() {
    local archive_file="$1"
    local sums_file="$2"
    if [[ ! -s "$sums_file" ]]; then
        return 0
    fi
    local base_name
    base_name="$(basename "$archive_file")"
    local expected
    expected="$(grep -E "[[:space:]]${base_name}\$" "$sums_file" | awk '{print $1}' | head -n 1 || true)"
    if [[ -z "$expected" ]]; then
        expected="$(awk '{print $1}' "$sums_file" | head -n 1 || true)"
    fi
    if [[ -z "$expected" ]]; then
        return 0
    fi
    if command -v sha256sum >/dev/null 2>&1; then
        local actual
        actual="$(sha256sum "$archive_file" | awk '{print $1}')"
        if [[ "$actual" != "$expected" ]]; then
            echo "Error [install.sh]: SHA-256 mismatch for ${base_name} (expected ${expected}, got ${actual})" >&2
            return 1
        fi
        echo "  [OK] Verified SHA-256 checksum for ${base_name}"
    fi
    return 0
}

unpack_archive_to_repo_root() {
    local archive_path="$1"
    local extract_dir="$2"
    mkdir -p "$extract_dir"
    tar -xzf "$archive_path" -C "$extract_dir"
    local top_dir
    top_dir="$(find "$extract_dir" -mindepth 1 -maxdepth 1 -type d | head -n 1 || true)"
    if [[ -n "$top_dir" ]] && (is_valid_source_or_bundle_dir "$top_dir" || [[ -f "${top_dir}/Cargo.toml" ]]); then
        REPO_ROOT="$top_dir"
    else
        REPO_ROOT="$extract_dir"
    fi
}

download_remote_release_or_source() {
    TMP_WORK_DIR="$(mktemp -d "${TMPDIR:-/tmp}/verbot-install.XXXXXX")"

    if [[ -n "$FROM_ARCHIVE" ]]; then
        if [[ ! -f "$FROM_ARCHIVE" ]]; then
            echo "Error [install.sh]: Archive not found at '${FROM_ARCHIVE}'" >&2
            exit 1
        fi
        echo "==> [Verbocity Installer] Extracting archive ${FROM_ARCHIVE}..."
        if [[ -f "${FROM_ARCHIVE}.sha256" ]]; then
            verify_sha256_if_available "$FROM_ARCHIVE" "${FROM_ARCHIVE}.sha256"
        elif [[ -f "$(dirname "$FROM_ARCHIVE")/SHA256SUMS" ]]; then
            verify_sha256_if_available "$FROM_ARCHIVE" "$(dirname "$FROM_ARCHIVE")/SHA256SUMS"
        fi
        unpack_archive_to_repo_root "$FROM_ARCHIVE" "${TMP_WORK_DIR}/unpacked"
        return 0
    fi

    local arch
    arch="$(detect_target_arch)"
    local os_name
    os_name="$(uname -s 2>/dev/null | tr '[:upper:]' '[:lower:]' || echo "linux")"

    # 1. Try prebuilt release archive when `--from-source` is not set and OS is Linux
    if [[ "$FROM_SOURCE" -eq 0 && "$os_name" == "linux" ]] && command -v curl >/dev/null 2>&1; then
        local resolved_tag="$VERBOT_VERSION"
        if [[ "$resolved_tag" == "latest" ]]; then
            resolved_tag="$(curl -fsSLI -o /dev/null -w '%{url_effective}' "${VERBOT_RELEASE_BASE_URL}/latest" 2>/dev/null | awk -F'/' '{print $NF}' || true)"
        fi
        if [[ -n "$resolved_tag" && "$resolved_tag" != "latest" && "$resolved_tag" != "releases" ]]; then
            local ver_no_v="${resolved_tag#v}"
            local tarball_name="verbot-v${ver_no_v}-${arch}-unknown-linux-gnu.tar.gz"
            local tarball_url="${VERBOT_RELEASE_BASE_URL}/download/${resolved_tag}/${tarball_name}"
            local sums_url="${VERBOT_RELEASE_BASE_URL}/download/${resolved_tag}/SHA256SUMS"
            local dl_tar="${TMP_WORK_DIR}/${tarball_name}"
            local dl_sums="${TMP_WORK_DIR}/SHA256SUMS"
            echo "==> [Verbocity Installer] Attempting prebuilt release download: ${tarball_url}"
            if curl -fsSL "$tarball_url" -o "$dl_tar" 2>/dev/null; then
                curl -fsSL "$sums_url" -o "$dl_sums" 2>/dev/null || true
                verify_sha256_if_available "$dl_tar" "$dl_sums"
                unpack_archive_to_repo_root "$dl_tar" "${TMP_WORK_DIR}/release"
                SKIP_BUILD=1
                return 0
            fi
            echo "==> [Verbocity Installer] Prebuilt binary not found for ${resolved_tag} (${arch}); falling back to source build..."
        fi
    fi

    # 2. Fallback: Download source tarball or shallow git clone into TMP_WORK_DIR
    local src_extract="${TMP_WORK_DIR}/src"
    mkdir -p "$src_extract"
    local tarball_ref_url="https://github.com/${VERBOT_REPO}/archive/refs/heads/${VERBOT_GIT_REF}.tar.gz"
    if [[ "$VERBOT_VERSION" != "latest" ]]; then
        local tag_ref="v${VERBOT_VERSION#v}"
        tarball_ref_url="https://github.com/${VERBOT_REPO}/archive/refs/tags/${tag_ref}.tar.gz"
    fi

    if command -v curl >/dev/null 2>&1 && curl -fsSL "$tarball_ref_url" -o "${TMP_WORK_DIR}/source.tar.gz" 2>/dev/null; then
        echo "==> [Verbocity Installer] Downloaded source archive from ${tarball_ref_url}"
        unpack_archive_to_repo_root "${TMP_WORK_DIR}/source.tar.gz" "$src_extract"
        return 0
    elif command -v git >/dev/null 2>&1; then
        echo "==> [Verbocity Installer] Cloning ${VERBOT_GIT_URL} (${VERBOT_GIT_REF})..."
        git clone --depth 1 --branch "$VERBOT_GIT_REF" "$VERBOT_GIT_URL" "${src_extract}/repo"
        REPO_ROOT="${src_extract}/repo"
        return 0
    else
        echo "Error [install.sh]: Neither 'curl' nor 'git' succeeded in fetching ${VERBOT_REPO}." >&2
        exit 1
    fi
}

if [[ "$INSTALL_MODE" != "local" ]]; then
    download_remote_release_or_source
fi

# 1. Resolve or build `verbot-tui` binary
RELEASE_BIN="${VERBOT_TUI_BIN:-}"
if [[ -z "$RELEASE_BIN" ]]; then
    if [[ -x "${REPO_ROOT}/bin/verbot-tui" ]]; then
        RELEASE_BIN="${REPO_ROOT}/bin/verbot-tui"
    else
        RELEASE_BIN="${REPO_ROOT}/target/release/verbot-tui"
    fi
fi

if [[ "$SKIP_BUILD" -eq 0 && ! -x "${REPO_ROOT}/bin/verbot-tui" ]]; then
    if [[ ! -x "$RELEASE_BIN" || -z "${VERBOT_TUI_BIN:-}" ]]; then
        if ! command -v cargo >/dev/null 2>&1; then
            echo "Error [install.sh]: 'cargo' is required to build from source. Install Rust (https://rustup.rs) or use a prebuilt release archive." >&2
            exit 1
        fi
        echo "==> [Verbocity Installer] Building release binary (cargo build --release)..."
        (cd "$REPO_ROOT" && cargo build --release)
        RELEASE_BIN="${REPO_ROOT}/target/release/verbot-tui"
    fi
fi

if [[ ! -x "$RELEASE_BIN" && -x "${REPO_ROOT}/bin/verbot-tui" ]]; then
    RELEASE_BIN="${REPO_ROOT}/bin/verbot-tui"
elif [[ ! -x "$RELEASE_BIN" && -x "${REPO_ROOT}/target/debug/verbot-tui" ]]; then
    RELEASE_BIN="${REPO_ROOT}/target/debug/verbot-tui"
fi

if [[ ! -x "$RELEASE_BIN" ]]; then
    echo "Error [install.sh]: Binary not found at ${RELEASE_BIN}" >&2
    exit 1
fi

# 2. Install `verbot` and `verbot-tui` into BIN_DIR (and optional /usr/local/bin symlink)
mkdir -p "$BIN_DIR"
install -m 0755 "$RELEASE_BIN" "${BIN_DIR}/verbot-tui"
install -m 0755 "${REPO_ROOT}/bin/verbot" "${BIN_DIR}/verbot"
echo "  [OK] Installed ${BIN_DIR}/verbot and ${BIN_DIR}/verbot-tui"

if [[ "$SYSTEM_SYMLINK" -eq 1 && -d "/usr/local/bin" && -w "/usr/local/bin" ]]; then
    ln -sf "${BIN_DIR}/verbot" "/usr/local/bin/verbot"
    ln -sf "${BIN_DIR}/verbot-tui" "/usr/local/bin/verbot-tui"
    echo "  [OK] Created system symlinks in /usr/local/bin/{verbot,verbot-tui}"
fi

# 3. Ensure BIN_DIR is on PATH across bash, zsh, POSIX profile, and fish
ensure_posix_rc_path() {
    local rc_file="$1"
    local path_expr="$2"
    if [[ ! -f "$rc_file" ]]; then
        touch "$rc_file"
    fi
    if ! grep -Fq "# Verbocity (verbot) PATH" "$rc_file" 2>/dev/null; then
        cat >> "$rc_file" <<EOF

# Verbocity (verbot) PATH
export PATH="${path_expr}:\$PATH"
EOF
        echo "  [OK] Added Verbocity PATH entry to ${rc_file}"
    else
        echo "  [OK] Verbocity PATH entry already present in ${rc_file}"
    fi
}

ensure_fish_rc_path() {
    local fish_cfg="$1"
    local path_expr="$2"
    if [[ ! -f "$fish_cfg" ]]; then
        return 0
    fi
    if ! grep -Fq "# Verbocity (verbot) PATH" "$fish_cfg" 2>/dev/null; then
        cat >> "$fish_cfg" <<EOF

# Verbocity (verbot) PATH
fish_add_path -g "${path_expr}"
EOF
        echo "  [OK] Added Verbocity PATH entry to ${fish_cfg}"
    else
        echo "  [OK] Verbocity PATH entry already present in ${fish_cfg}"
    fi
}

if [[ "$NO_MODIFY_PATH" -eq 0 ]]; then
    mkdir -p "$SHELL_HOME"
    if [[ "$BIN_DIR" == "${HOME}/.local/bin" ]]; then
        EXPORT_BIN_EXPR="\$HOME/.local/bin"
    else
        EXPORT_BIN_EXPR="$BIN_DIR"
    fi
    ensure_posix_rc_path "${SHELL_HOME}/.bashrc" "$EXPORT_BIN_EXPR"
    ensure_posix_rc_path "${SHELL_HOME}/.zshrc" "$EXPORT_BIN_EXPR"
    ensure_posix_rc_path "${SHELL_HOME}/.profile" "$EXPORT_BIN_EXPR"
    if [[ -f "${SHELL_HOME}/.config/fish/config.fish" ]] || command -v fish >/dev/null 2>&1; then
        mkdir -p "${SHELL_HOME}/.config/fish"
        touch "${SHELL_HOME}/.config/fish/config.fish"
        ensure_fish_rc_path "${SHELL_HOME}/.config/fish/config.fish" "$BIN_DIR"
    fi
fi

# 4. Install modular dispatch scripts (supports both repo layout `scripts/` and bundle layout `share/verbot/scripts/`)
mkdir -p "$SCRIPTS_DEST"
if [[ -f "${REPO_ROOT}/scripts/dispatch-agy.sh" ]]; then
    install -m 0755 "${REPO_ROOT}/scripts/dispatch-agy.sh" "${SCRIPTS_DEST}/dispatch-agy.sh"
    install -m 0755 "${REPO_ROOT}/scripts/dispatch-output.sh" "${SCRIPTS_DEST}/dispatch-output.sh"
elif [[ -f "${REPO_ROOT}/share/verbot/scripts/dispatch-agy.sh" ]]; then
    install -m 0755 "${REPO_ROOT}/share/verbot/scripts/dispatch-agy.sh" "${SCRIPTS_DEST}/dispatch-agy.sh"
    install -m 0755 "${REPO_ROOT}/share/verbot/scripts/dispatch-output.sh" "${SCRIPTS_DEST}/dispatch-output.sh"
fi
echo "  [OK] Installed dispatch scripts to ${SCRIPTS_DEST}/"

# 5. Install COSMIC / freedesktop icon and desktop launcher
mkdir -p "$ICON_DEST_DIR" "$APPS_DEST_DIR"
if [[ -f "${REPO_ROOT}/assets/verbot-logo.svg" ]]; then
    install -m 0644 "${REPO_ROOT}/assets/verbot-logo.svg" "${ICON_DEST_DIR}/verbot.svg"
elif [[ -f "${REPO_ROOT}/share/icons/hicolor/scalable/apps/verbot.svg" ]]; then
    install -m 0644 "${REPO_ROOT}/share/icons/hicolor/scalable/apps/verbot.svg" "${ICON_DEST_DIR}/verbot.svg"
fi

if [[ -f "${REPO_ROOT}/assets/verbot.desktop" ]]; then
    install -m 0644 "${REPO_ROOT}/assets/verbot.desktop" "${APPS_DEST_DIR}/verbot.desktop"
elif [[ -f "${REPO_ROOT}/share/applications/verbot.desktop" ]]; then
    install -m 0644 "${REPO_ROOT}/share/applications/verbot.desktop" "${APPS_DEST_DIR}/verbot.desktop"
fi
echo "  [OK] Installed icon (${ICON_DEST_DIR}/verbot.svg) and desktop launcher (${APPS_DEST_DIR}/verbot.desktop)"

if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$APPS_DEST_DIR" >/dev/null 2>&1 || true
fi

# 6. Install default templates, `config.toml`, `theme.toml`, and `models.toml` into `~/.config/verbot/`
mkdir -p "$TEMPLATES_DEST_DIR"
for tpl_dir in "${REPO_ROOT}/templates" "${REPO_ROOT}/share/verbot/templates"; do
    if [[ -f "${tpl_dir}/GeminiAgentPromptTemplate.md" ]]; then
        install -m 0644 "${tpl_dir}/GeminiAgentPromptTemplate.md" "${TEMPLATES_DEST_DIR}/GeminiAgentPromptTemplate.md"
    fi
    if [[ -f "${tpl_dir}/GeminiAgentPromptTemplate.toml" ]]; then
        install -m 0644 "${tpl_dir}/GeminiAgentPromptTemplate.toml" "${TEMPLATES_DEST_DIR}/GeminiAgentPromptTemplate.toml"
    fi
done

if [[ ! -f "$CONFIG_TOML_DEST" ]]; then
    cat > "$CONFIG_TOML_DEST" <<'EOF'
# Verbocity (`verbot`) Global Configuration
default_theme_preset = "NovOS Cosmic Dark"
default_model_family = "Gemini"
default_sub_model = "Argon"
default_effort = "medium"
default_slash_command = "plan"
show_preview_on_startup = true
shell_formatting_policy = "PromptOnSpecialChars"
workspace_storage_local = false

[agy_flags]
mode = "None"
sandbox = false
continue_session = false
dangerously_skip_permissions = false
add_dirs = []
EOF
    chmod 0644 "$CONFIG_TOML_DEST"
    echo "  [OK] Created default global config at ${CONFIG_TOML_DEST}"
else
    echo "  [OK] Preserved existing global config at ${CONFIG_TOML_DEST}"
fi

if [[ ! -f "$THEME_TOML_DEST" ]]; then
    cat > "$THEME_TOML_DEST" <<'EOF'
# Verbocity (`verbot`) Theme Configuration
# Presets: "NovOS Cosmic Dark", "COSMIC System Auto", "NovOS Solar Light", "Terminal ANSI"
preset = "NovOS Cosmic Dark"
name = "NovOS Cosmic Dark"
is_dark = true
use_ansi = false

# Default NovOS Cosmic Nova Orange & Deep Obsidian Palette
bg_primary = "#0B0C10"
bg_deep = "#07080B"
surface_primary = "#121521"
surface_secondary = "#0F121C"
border_slate = "#2A314A"
accent_primary = "#FF6B1A"
accent_secondary = "#FF9E44"
accent_tertiary = "#FFB347"
highlight_core = "#FFF3E0"
text_primary = "#D8E2FF"
title_inactive = "#FF9E44"
badge_inactive = "#9BA8CC"
placeholder_muted = "#5A6588"
EOF
    chmod 0644 "$THEME_TOML_DEST"
    echo "  [OK] Created default NovOS Cosmic Nova Orange theme at ${THEME_TOML_DEST}"
else
    echo "  [OK] Preserved existing user theme config at ${THEME_TOML_DEST}"
fi

if [[ ! -f "$MODELS_TOML_DEST" ]]; then
    cat > "$MODELS_TOML_DEST" <<'EOF'
# Verbocity (`verbot`) Model Catalog Configuration
# Add or ingest custom sub-models via `verbot --ingest-model <file.toml>` or Settings Page (`Alt+S` -> Tab 2)

[[profiles]]
family = "Gemini"
short_name = "Argon"
display_name = "Gemini 4 Argon"
agy_base_id = "gemini-4-argon"
available_tiers = ["low", "medium", "high"]
default_effort = "Medium"
format_style = "GeminiConcise"

[[profiles]]
family = "Gemini"
short_name = "Flash"
display_name = "Gemini 3.8 Flash"
agy_base_id = "gemini-3.8-flash"
available_tiers = ["low", "medium", "high"]
default_effort = "High"
format_style = "GeminiConcise"

[[profiles]]
family = "Claude"
short_name = "Opus"
display_name = "Claude Opus 5.5"
agy_base_id = "claude-opus-5-5"
available_tiers = ["low", "medium", "high"]
default_effort = "High"
format_style = "ClaudeXml"

[[profiles]]
family = "Claude"
short_name = "Sonnet"
display_name = "Claude Sonnet 5.5"
agy_base_id = "claude-sonnet-5-5"
available_tiers = ["low", "medium", "high"]
default_effort = "Medium"
format_style = "ClaudeXml"

[[profiles]]
family = "Gpt"
short_name = "GPT-OSS 120B"
display_name = "GPT-OSS 120B"
agy_base_id = "gpt-oss-120b"
available_tiers = ["medium"]
default_effort = "Medium"
format_style = "GptMarkdownSections"
EOF
    chmod 0644 "$MODELS_TOML_DEST"
    echo "  [OK] Created default model catalog at ${MODELS_TOML_DEST}"
else
    echo "  [OK] Preserved existing model catalog at ${MODELS_TOML_DEST}"
fi

echo "==> [Verbocity Installer] Installation complete. Run 'verbot' from any directory."
