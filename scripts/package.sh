#!/usr/bin/env bash
# ==============================================================================
# Verbocity (`verbot`) — Automated Release & Distribution Packager (`scripts/package.sh`)
# ==============================================================================
# Builds deterministic distribution artifacts in `dist/` (or `--out-dir <DIR>`):
#   1. Portable Release Tarball:
#      `verbot-v<VERSION>-<ARCH>-unknown-linux-gnu.tar.gz`
#      Contains `bin/{verbot,verbot-tui}`, `share/verbot/scripts/dispatch-{agy,output}.sh`,
#      `share/icons/hicolor/scalable/apps/verbot.svg`, `share/applications/verbot.desktop`,
#      `share/verbot/templates/GeminiAgentPromptTemplate.{md,toml}`, `install.sh`, and `README.md`.
#   2. Debian Binary Package (`.deb`):
#      `verbot_<VERSION>_<DEB_ARCH>.deb` (built via `dpkg-deb --build` when available)
#      Installs into `/usr/bin` and `/usr/share`.
#   3. SHA-256 Checksum Manifest:
#      `SHA256SUMS` covering all generated archives/packages.
# ==============================================================================

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"

SKIP_BUILD=0
OUT_DIR="${REPO_ROOT}/dist"
TARGET_ARCH=""
VERSION_OVERRIDE=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --skip-build)
            SKIP_BUILD=1
            shift
            ;;
        --out-dir)
            OUT_DIR="$2"
            shift 2
            ;;
        --arch)
            TARGET_ARCH="$2"
            shift 2
            ;;
        --version)
            VERSION_OVERRIDE="$2"
            shift 2
            ;;
        -h|--help)
            cat <<'EOF'
Usage: ./scripts/package.sh [OPTIONS]

Options:
  --skip-build       Package existing compiled binary without running `cargo build --release`
  --out-dir <DIR>    Output directory for release artifacts (default: ./dist)
  --arch <ARCH>      Override target architecture label (default: uname -m)
  --version <VER>    Override version string (default: read from Cargo.toml)
  -h, --help         Show this help message
EOF
            exit 0
            ;;
        *)
            echo "Error [package.sh]: Unknown argument '$1'" >&2
            exit 1
            ;;
    esac
done

extract_cargo_version() {
    awk -F'"' '/^version[[:space:]]*=[[:space:]]*"/ { print $2; exit }' "${REPO_ROOT}/Cargo.toml"
}

VERSION="${VERSION_OVERRIDE:-$(extract_cargo_version)}"
VERSION="${VERSION#v.}"
VERSION="${VERSION#v}"
if [[ -z "$VERSION" ]]; then
    VERSION="0.1.0"
fi

if [[ -z "$TARGET_ARCH" ]]; then
    TARGET_ARCH="$(uname -m 2>/dev/null || echo "x86_64")"
fi

case "$TARGET_ARCH" in
    x86_64|amd64)
        RUST_ARCH="x86_64"
        DEB_ARCH="amd64"
        ;;
    aarch64|arm64)
        RUST_ARCH="aarch64"
        DEB_ARCH="arm64"
        ;;
    *)
        RUST_ARCH="$TARGET_ARCH"
        DEB_ARCH="$TARGET_ARCH"
        ;;
esac

RELEASE_BIN="${VERBOT_TUI_BIN:-${REPO_ROOT}/target/release/verbot-tui}"
if [[ "$SKIP_BUILD" -eq 0 ]]; then
    echo "==> [Verbocity Packager] Building release binary (cargo build --release)..."
    (cd "$REPO_ROOT" && cargo build --release)
    RELEASE_BIN="${REPO_ROOT}/target/release/verbot-tui"
fi

if [[ ! -x "$RELEASE_BIN" && -x "${REPO_ROOT}/target/debug/verbot-tui" ]]; then
    RELEASE_BIN="${REPO_ROOT}/target/debug/verbot-tui"
fi

if [[ ! -x "$RELEASE_BIN" ]]; then
    echo "Error [package.sh]: Compiled binary not found at ${RELEASE_BIN}. Run 'cargo build --release' first." >&2
    exit 1
fi

mkdir -p "$OUT_DIR"
OUT_DIR="$(cd "$OUT_DIR" && pwd)"

STAGE_ROOT="$(mktemp -d "${TMPDIR:-/tmp}/verbot-pkg.XXXXXX")"
cleanup() {
    rm -rf "$STAGE_ROOT"
}
trap cleanup EXIT

# ------------------------------------------------------------------------------
# 1. Build Portable Tarball (`verbot-v<VERSION>-<ARCH>-unknown-linux-gnu.tar.gz`)
# ------------------------------------------------------------------------------
TAR_STEM="verbot-v${VERSION}-${RUST_ARCH}-unknown-linux-gnu"
TAR_STAGE="${STAGE_ROOT}/${TAR_STEM}"

mkdir -p \
    "${TAR_STAGE}/bin" \
    "${TAR_STAGE}/share/verbot/scripts" \
    "${TAR_STAGE}/share/verbot/templates" \
    "${TAR_STAGE}/share/icons/hicolor/scalable/apps" \
    "${TAR_STAGE}/share/applications"

install -m 0755 "${REPO_ROOT}/bin/verbot" "${TAR_STAGE}/bin/verbot"
install -m 0755 "$RELEASE_BIN" "${TAR_STAGE}/bin/verbot-tui"
install -m 0755 "${REPO_ROOT}/scripts/dispatch-agy.sh" "${TAR_STAGE}/share/verbot/scripts/dispatch-agy.sh"
install -m 0755 "${REPO_ROOT}/scripts/dispatch-output.sh" "${TAR_STAGE}/share/verbot/scripts/dispatch-output.sh"
install -m 0644 "${REPO_ROOT}/assets/verbot-logo.svg" "${TAR_STAGE}/share/icons/hicolor/scalable/apps/verbot.svg"
install -m 0644 "${REPO_ROOT}/assets/verbot.desktop" "${TAR_STAGE}/share/applications/verbot.desktop"
install -m 0644 "${REPO_ROOT}/templates/GeminiAgentPromptTemplate.md" "${TAR_STAGE}/share/verbot/templates/GeminiAgentPromptTemplate.md"
install -m 0644 "${REPO_ROOT}/templates/GeminiAgentPromptTemplate.toml" "${TAR_STAGE}/share/verbot/templates/GeminiAgentPromptTemplate.toml"
install -m 0755 "${REPO_ROOT}/install.sh" "${TAR_STAGE}/install.sh"
install -m 0644 "${REPO_ROOT}/README.md" "${TAR_STAGE}/README.md"

TARBALL_PATH="${OUT_DIR}/${TAR_STEM}.tar.gz"
tar -C "$STAGE_ROOT" -czf "$TARBALL_PATH" "$TAR_STEM"
echo "  [OK] Built portable release archive: ${TARBALL_PATH}"

# ------------------------------------------------------------------------------
# 2. Build Debian Package (`verbot_<VERSION>_<DEB_ARCH>.deb`) if dpkg-deb exists
# ------------------------------------------------------------------------------
DEB_FILENAME="verbot_${VERSION}_${DEB_ARCH}.deb"
DEB_PATH="${OUT_DIR}/${DEB_FILENAME}"
if command -v dpkg-deb >/dev/null 2>&1; then
    DEB_STAGE="${STAGE_ROOT}/deb_root"
    mkdir -p \
        "${DEB_STAGE}/DEBIAN" \
        "${DEB_STAGE}/usr/bin" \
        "${DEB_STAGE}/usr/share/verbot/scripts" \
        "${DEB_STAGE}/usr/share/verbot/templates" \
        "${DEB_STAGE}/usr/share/icons/hicolor/scalable/apps" \
        "${DEB_STAGE}/usr/share/applications" \
        "${DEB_STAGE}/usr/share/doc/verbot"

    install -m 0755 "${REPO_ROOT}/bin/verbot" "${DEB_STAGE}/usr/bin/verbot"
    install -m 0755 "$RELEASE_BIN" "${DEB_STAGE}/usr/bin/verbot-tui"
    install -m 0755 "${REPO_ROOT}/scripts/dispatch-agy.sh" "${DEB_STAGE}/usr/share/verbot/scripts/dispatch-agy.sh"
    install -m 0755 "${REPO_ROOT}/scripts/dispatch-output.sh" "${DEB_STAGE}/usr/share/verbot/scripts/dispatch-output.sh"
    install -m 0644 "${REPO_ROOT}/assets/verbot-logo.svg" "${DEB_STAGE}/usr/share/icons/hicolor/scalable/apps/verbot.svg"
    install -m 0644 "${REPO_ROOT}/assets/verbot.desktop" "${DEB_STAGE}/usr/share/applications/verbot.desktop"
    install -m 0644 "${REPO_ROOT}/templates/GeminiAgentPromptTemplate.md" "${DEB_STAGE}/usr/share/verbot/templates/GeminiAgentPromptTemplate.md"
    install -m 0644 "${REPO_ROOT}/templates/GeminiAgentPromptTemplate.toml" "${DEB_STAGE}/usr/share/verbot/templates/GeminiAgentPromptTemplate.toml"
    install -m 0644 "${REPO_ROOT}/README.md" "${DEB_STAGE}/usr/share/doc/verbot/README.md"

    cat > "${DEB_STAGE}/DEBIAN/control" <<EOF
Package: verbot
Version: ${VERSION}
Section: utils
Priority: optional
Architecture: ${DEB_ARCH}
Depends: bash, libc6
Maintainer: Verbocity Maintainers <maintainers@novos.dev>
Description: Verbocity (verbot) — Interactive terminal LLM prompt template engine and agy dispatcher
 Multi-pane Ratatui terminal prompt builder, template engine, and Antigravity (agy)
 CLI dispatcher styled in the NovOS Cosmic Nova Orange & COSMIC DE palette.
EOF
    chmod 0644 "${DEB_STAGE}/DEBIAN/control"
    dpkg-deb --root-owner-group --build "$DEB_STAGE" "$DEB_PATH" >/dev/null
    echo "  [OK] Built Debian package:           ${DEB_PATH}"
fi

# ------------------------------------------------------------------------------
# 3. Generate `SHA256SUMS`
# ------------------------------------------------------------------------------
if command -v sha256sum >/dev/null 2>&1; then
    (
        cd "$OUT_DIR"
        ARTIFACTS=("$(basename "$TARBALL_PATH")")
        if [[ -f "$DEB_PATH" ]]; then
            ARTIFACTS+=("$(basename "$DEB_PATH")")
        fi
        sha256sum "${ARTIFACTS[@]}" > SHA256SUMS
        sha256sum "$(basename "$TARBALL_PATH")" > "$(basename "$TARBALL_PATH").sha256"
    )
    echo "  [OK] Generated checksum manifest:    ${OUT_DIR}/SHA256SUMS"
fi

echo "==> [Verbocity Packager] Packaging complete in ${OUT_DIR}"
