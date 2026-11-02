#!/usr/bin/env bash
# ==============================================================================
# Verbocity (`verbot`) — agy Dispatcher (`scripts/dispatch-agy.sh`)
# ==============================================================================
# Reads a structured JSON DispatchManifest written by `verbot-tui`, resolves the
# `agy` executable (`command -v agy` or `~/.local/bin/agy`), constructs the
# invocation (`agy --model "$MODEL_ID" --effort "$EFFORT" [EXTRA_FLAGS...] -i "$PROMPT"` or
# `... -p "$PROMPT"`), and execs it in the user's current working directory.
#
# Environment / CLI overrides for automated testing:
#   --dry-run                Print resolved agy invocation without executing agy
#   VERBOT_AGY_DRY_RUN=1     Same as --dry-run
#   VERBOT_AGY_BIN=<path>    Override path to the `agy` executable
# ==============================================================================

set -euo pipefail

DRY_RUN="${VERBOT_AGY_DRY_RUN:-0}"
MANIFEST_PATH=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --dry-run)
            DRY_RUN=1
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
    echo "Error [dispatch-agy]: Missing or invalid DispatchManifest path: '${MANIFEST_PATH}'" >&2
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
        echo "Error [dispatch-agy]: Neither 'jq' nor 'python3' is available to parse DispatchManifest." >&2
        exit 1
    fi
}

extract_json_array() {
    local file="$1"
    local field="$2"
    if command -v jq >/dev/null 2>&1; then
        jq -r ".${field}[]? // empty" "$file"
    elif command -v python3 >/dev/null 2>&1; then
        python3 -c 'import json, sys; data = json.load(open(sys.argv[1], encoding="utf-8")); arr = data.get(sys.argv[2]) or []; print("\n".join(str(x) for x in arr))' "$file" "$field"
    fi
}

ACTION="$(extract_json_field "$MANIFEST_PATH" "action")"
MODEL_ID="$(extract_json_field "$MANIFEST_PATH" "model_id")"
EFFORT="$(extract_json_field "$MANIFEST_PATH" "effort")"
PROMPT="$(extract_json_field "$MANIFEST_PATH" "rendered_prompt")"
TARGET_CWD="$(extract_json_field "$MANIFEST_PATH" "cwd")"
PREVIEW="$(extract_json_field "$MANIFEST_PATH" "agy_command_preview")"
SHELL_SAFE_CMD="$(extract_json_field "$MANIFEST_PATH" "shell_safe_command")"

EXTRA_FLAGS=()
while IFS= read -r flag_item; do
    if [[ -n "$flag_item" ]]; then
        EXTRA_FLAGS+=("$flag_item")
    fi
done < <(extract_json_array "$MANIFEST_PATH" "agy_extra_flags")

if [[ -z "$MODEL_ID" ]]; then
    MODEL_ID="gemini-4-argon-medium"
fi
if [[ -z "$EFFORT" ]]; then
    EFFORT="medium"
fi

MODE_FLAG="-i"
case "$ACTION" in
    agy-print|print|-p)
        MODE_FLAG="-p"
        ;;
    agy-interactive|interactive|-i|*)
        MODE_FLAG="-i"
        ;;
esac

resolve_agy_bin() {
    if [[ -n "${VERBOT_AGY_BIN:-}" ]]; then
        echo "$VERBOT_AGY_BIN"
        return 0
    fi
    if command -v agy >/dev/null 2>&1; then
        command -v agy
        return 0
    fi
    if [[ -x "${HOME}/.local/bin/agy" ]]; then
        echo "${HOME}/.local/bin/agy"
        return 0
    fi
    return 1
}

if [[ -n "$TARGET_CWD" && -d "$TARGET_CWD" ]]; then
    cd "$TARGET_CWD"
fi

if [[ "$DRY_RUN" == "1" || "$DRY_RUN" == "true" ]]; then
    AGY_BIN="$(resolve_agy_bin || echo "agy")"
    echo "==> [Verbocity Dry-Run] CWD: $(pwd)"
    echo "==> [Verbocity Dry-Run] Command: ${AGY_BIN} --model ${MODEL_ID} --effort ${EFFORT} ${EXTRA_FLAGS[*]} ${MODE_FLAG} <PROMPT>"
    if [[ -n "$PREVIEW" ]]; then
        echo "==> [Verbocity Dry-Run] Preview: ${PREVIEW}"
    fi
    printf '%s\n' "$PROMPT"
    exit 0
fi

if ! AGY_BIN="$(resolve_agy_bin)"; then
    echo "Error [dispatch-agy]: Could not locate 'agy' on PATH or at ~/.local/bin/agy." >&2
    echo "Constructed command preview: ${PREVIEW}" >&2
    echo "--- Rendered Prompt ---" >&2
    printf '%s\n' "$PROMPT"
    exit 127
fi

exec "$AGY_BIN" --model "$MODEL_ID" --effort "$EFFORT" "${EXTRA_FLAGS[@]}" "$MODE_FLAG" "$PROMPT"
