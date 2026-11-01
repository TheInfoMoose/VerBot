#!/usr/bin/env bash
# ==============================================================================
# Verbocity (`verbot`) — Output & File Dispatcher (`scripts/dispatch-output.sh`)
# ==============================================================================
# Handles non-agy dispatch actions emitted by `verbot-tui`:
#   - `stdout`: Prints the rendered prompt to stdout and optionally copies it
#               to the system clipboard (`wl-copy` / `xclip` / `xsel`) when
#               available and not disabled via `VERBOT_NO_CLIPBOARD=1`.
#   - `save-md`: Saves the rendered prompt to a Markdown file in the user's
#                current working directory (`verbot-prompt.md` by default).
# ==============================================================================

set -euo pipefail

MANIFEST_PATH=""
NO_CLIPBOARD="${VERBOT_NO_CLIPBOARD:-0}"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --no-clipboard)
            NO_CLIPBOARD=1
            shift
            ;;
        --manifest)
            MANIFEST_PATH="${2:-}"
            shift 2
            ;;
        *)
            if [[ -z "$MANIFEST_PATH" ]]; then
                MANIFEST_PATH="$1"
            fi
            shift
            ;;
    esac
done

if [[ -z "$MANIFEST_PATH" || ! -f "$MANIFEST_PATH" ]]; then
    echo "Error [dispatch-output]: Missing or invalid DispatchManifest path: '${MANIFEST_PATH}'" >&2
    exit 1
fi

extract_json_field() {
    local file="$1"
    local field="$2"
    if command -v jq >/dev/null 2>&1; then
        jq -r ".${field} // empty" "$file"
    elif command -v python3 >/dev/null 2>&1; then
        python3 -c 'import json, sys; data = json.load(open(sys.argv[1], encoding="utf-8")); val = data.get(sys.argv[2]); print("" if val is None else val)' "$file" "$field"
    else
        echo "Error [dispatch-output]: Neither 'jq' nor 'python3' is available to parse DispatchManifest." >&2
        exit 1
    fi
}

ACTION="$(extract_json_field "$MANIFEST_PATH" "action")"
PROMPT="$(extract_json_field "$MANIFEST_PATH" "rendered_prompt")"
OUTPUT_MD_PATH="$(extract_json_field "$MANIFEST_PATH" "output_md_path")"
TARGET_CWD="$(extract_json_field "$MANIFEST_PATH" "cwd")"

if [[ -n "$TARGET_CWD" && -d "$TARGET_CWD" ]]; then
    cd "$TARGET_CWD"
fi

copy_to_clipboard_if_available() {
    local content="$1"
    if [[ "$NO_CLIPBOARD" == "1" || "$NO_CLIPBOARD" == "true" ]]; then
        return 1
    fi
    if [[ -n "${WAYLAND_DISPLAY:-}" ]] && command -v wl-copy >/dev/null 2>&1; then
        printf '%s' "$content" | wl-copy 2>/dev/null && return 0
    fi
    if [[ -n "${DISPLAY:-}" ]] && command -v xclip >/dev/null 2>&1; then
        printf '%s' "$content" | xclip -selection clipboard 2>/dev/null && return 0
    fi
    if [[ -n "${DISPLAY:-}" ]] && command -v xsel >/dev/null 2>&1; then
        printf '%s' "$content" | xsel --clipboard --input 2>/dev/null && return 0
    fi
    return 1
}

case "$ACTION" in
    save-md|save|md)
        if [[ -z "$OUTPUT_MD_PATH" ]]; then
            OUTPUT_MD_PATH="$(pwd)/verbot-prompt.md"
        fi
        mkdir -p "$(dirname "$OUTPUT_MD_PATH")"
        printf '%s\n' "$PROMPT" > "$OUTPUT_MD_PATH"
        echo "Saved rendered prompt to ${OUTPUT_MD_PATH}"
        ;;
    stdout|clipboard|*)
        printf '%s\n' "$PROMPT"
        if [[ -t 1 ]]; then
            if copy_to_clipboard_if_available "$PROMPT"; then
                echo "==> [Verbocity] Copied rendered prompt to system clipboard." >&2
            fi
        fi
        ;;
esac
