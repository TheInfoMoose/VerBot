use crate::models::{AgyLaunchMode, EffortLevel, sanitize_field_value};
use crate::template::PromptValues;
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// Shell-sensitive characters (excluding `\n` and `\r`, which are tracked via `has_multiline`).
pub const SHELL_SENSITIVE_CHARS: [char; 19] = [
    '"', '\'', '$', '`', '*', '?', '(', ')', '[', ']', '{', '}', ';', '&', '|', '<', '>', '\\',
    '!',
];

/// Policy governing how Verbocity handles shell-sensitive characters and multi-line prompts
/// when dispatching to `agy` or generating terminal commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShellFormattingPolicy {
    /// Default: prompts the user to auto-escape/quote when unescaped shell metacharacters are detected.
    PromptOnSpecialChars,
    /// Automatically POSIX-quotes and escapes the prompt without showing a confirmation modal.
    AutoEscapeSilently,
    /// Leaves input unescaped/raw in previews (still passes clean OS argv when spawning directly).
    RawLiteral,
}

impl Default for ShellFormattingPolicy {
    fn default() -> Self {
        Self::PromptOnSpecialChars
    }
}

impl ShellFormattingPolicy {
    pub const ALL: [ShellFormattingPolicy; 3] = [
        Self::PromptOnSpecialChars,
        Self::AutoEscapeSilently,
        Self::RawLiteral,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::PromptOnSpecialChars => "PromptOnSpecialChars",
            Self::AutoEscapeSilently => "AutoEscapeSilently",
            Self::RawLiteral => "RawLiteral",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::PromptOnSpecialChars => "PromptOnSpecialChars (Ask on metacharacters)",
            Self::AutoEscapeSilently => "AutoEscapeSilently (Always POSIX-quote)",
            Self::RawLiteral => "RawLiteral (No escaping)",
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "autoescapesilently" | "auto-escape" | "auto_escape" | "auto" | "always" => {
                Self::AutoEscapeSilently
            }
            "rawliteral" | "raw-literal" | "raw_literal" | "raw" | "literal" | "none" => {
                Self::RawLiteral
            }
            _ => Self::PromptOnSpecialChars,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::PromptOnSpecialChars => Self::AutoEscapeSilently,
            Self::AutoEscapeSilently => Self::RawLiteral,
            Self::RawLiteral => Self::PromptOnSpecialChars,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::PromptOnSpecialChars => Self::RawLiteral,
            Self::AutoEscapeSilently => Self::PromptOnSpecialChars,
            Self::RawLiteral => Self::AutoEscapeSilently,
        }
    }
}

/// Optional `agy --mode` execution mode setting.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AgyExecutionMode {
    None,
    Auto,
    Plan,
    AcceptEdits,
}

impl Default for AgyExecutionMode {
    fn default() -> Self {
        Self::None
    }
}

impl AgyExecutionMode {
    pub const ALL: [AgyExecutionMode; 4] = [
        Self::None,
        Self::Auto,
        Self::Plan,
        Self::AcceptEdits,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Auto => "auto",
            Self::Plan => "plan",
            Self::AcceptEdits => "accept-edits",
        }
    }

    pub fn cli_value(self) -> Option<&'static str> {
        match self {
            Self::None => None,
            Self::Auto => Some("auto"),
            Self::Plan => Some("plan"),
            Self::AcceptEdits => Some("accept-edits"),
        }
    }

    pub fn from_str_loose(s: &str) -> Self {
        match s.trim().to_ascii_lowercase().as_str() {
            "auto" => Self::Auto,
            "plan" => Self::Plan,
            "accept-edits" | "acceptedits" | "accept_edits" => Self::AcceptEdits,
            _ => Self::None,
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::None => Self::Auto,
            Self::Auto => Self::Plan,
            Self::Plan => Self::AcceptEdits,
            Self::AcceptEdits => Self::None,
        }
    }

    pub fn prev(self) -> Self {
        match self {
            Self::None => Self::AcceptEdits,
            Self::Auto => Self::None,
            Self::Plan => Self::Auto,
            Self::AcceptEdits => Self::Plan,
        }
    }
}

/// Configurable default `agy` CLI flags persisted in `config.toml` and editable on the Settings Page.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgyCliFlags {
    #[serde(default)]
    pub mode: AgyExecutionMode,
    #[serde(default)]
    pub sandbox: bool,
    #[serde(default)]
    pub continue_session: bool,
    #[serde(default)]
    pub dangerously_skip_permissions: bool,
    #[serde(default)]
    pub add_dirs: Vec<String>,
}

impl AgyCliFlags {
    pub fn has_any(&self) -> bool {
        self.mode != AgyExecutionMode::None
            || self.sandbox
            || self.continue_session
            || self.dangerously_skip_permissions
            || self.add_dirs.iter().any(|d| !d.trim().is_empty())
    }

    /// Emits the configured flags as CLI arguments (`--mode <m>`, `--sandbox`, `--continue`, etc.).
    pub fn to_cli_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(m) = self.mode.cli_value() {
            args.push("--mode".to_string());
            args.push(m.to_string());
        }
        if self.sandbox {
            args.push("--sandbox".to_string());
        }
        if self.continue_session {
            args.push("--continue".to_string());
        }
        if self.dangerously_skip_permissions {
            args.push("--dangerously-skip-permissions".to_string());
        }
        for dir in &self.add_dirs {
            let trimmed = dir.trim();
            if !trimmed.is_empty() {
                args.push("--add-dir".to_string());
                args.push(trimmed.to_string());
            }
        }
        args
    }

    pub fn formatted_add_dirs(&self) -> String {
        self.add_dirs.join(", ")
    }

    pub fn set_add_dirs_from_str(&mut self, raw: &str) {
        self.add_dirs = raw
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(ToString::to_string)
            .collect();
    }
}

/// Report describing shell-sensitive metacharacters and line breaks found in user input.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ShellSafetyReport {
    pub has_special_chars: bool,
    pub has_multiline: bool,
    pub detected_chars: Vec<char>,
    pub affected_fields: Vec<String>,
}

impl ShellSafetyReport {
    pub fn needs_escaping_or_quoting(&self) -> bool {
        self.has_special_chars || self.has_multiline
    }

    pub fn summary(&self) -> String {
        if !self.needs_escaping_or_quoting() {
            return "Clean (no shell metacharacters)".to_string();
        }
        let mut tokens: Vec<String> = self
            .detected_chars
            .iter()
            .map(|c| format!("'{}'", c))
            .collect();
        if self.has_multiline {
            tokens.push("newlines (\\n)".to_string());
        }
        let fields_note = if self.affected_fields.is_empty() {
            String::new()
        } else {
            format!(" in [{}]", self.affected_fields.join(", "))
        };
        format!("Detected {}{}", tokens.join(", "), fields_note)
    }
}

/// Inspects a single string for shell-sensitive characters (`"`, `'`, `$`, `` ` ``, `*`, `?`,
/// `(`, `)`, `[`, `]`, `{`, `}`, `;`, `&`, `|`, `<`, `>`, `\`, `!`) and multi-line breaks.
pub fn inspect_shell_safety(text: &str) -> ShellSafetyReport {
    let mut detected_chars: Vec<char> = Vec::new();
    let mut has_multiline = false;

    let trimmed = text.trim();
    if trimmed.contains('\n') || trimmed.contains('\r') {
        has_multiline = true;
    }

    for ch in trimmed.chars() {
        if SHELL_SENSITIVE_CHARS.contains(&ch) && !detected_chars.contains(&ch) {
            detected_chars.push(ch);
        }
    }

    ShellSafetyReport {
        has_special_chars: !detected_chars.is_empty(),
        has_multiline,
        detected_chars,
        affected_fields: Vec::new(),
    }
}

/// Inspects all canonical and custom fields in [`PromptValues`] for shell-sensitive characters
/// and multi-line breaks.
pub fn inspect_prompt_values_safety(values: &PromptValues) -> ShellSafetyReport {
    let mut combined = ShellSafetyReport::default();

    let field_pairs: [(&str, Option<&str>); 4] = [
        ("Objective", sanitize_field_value(&values.objective)),
        ("Context/Logs", sanitize_field_value(&values.context_logs)),
        ("Constraints", sanitize_field_value(&values.constraints)),
        ("Output", sanitize_field_value(&values.output)),
    ];

    for (label, opt_val) in field_pairs {
        let Some(val) = opt_val else {
            continue;
        };
        let rep = inspect_shell_safety(val);
        if rep.needs_escaping_or_quoting() {
            combined.affected_fields.push(label.to_string());
        }
        if rep.has_multiline {
            combined.has_multiline = true;
        }
        for ch in rep.detected_chars {
            if !combined.detected_chars.contains(&ch) {
                combined.detected_chars.push(ch);
            }
        }
    }

    for (k, v) in &values.custom_fields {
        if let Some(val) = sanitize_field_value(v) {
            let rep = inspect_shell_safety(val);
            if rep.needs_escaping_or_quoting() {
                combined.affected_fields.push(k.clone());
            }
            if rep.has_multiline {
                combined.has_multiline = true;
            }
            for ch in rep.detected_chars {
                if !combined.detected_chars.contains(&ch) {
                    combined.detected_chars.push(ch);
                }
            }
        }
    }

    combined.has_special_chars = !combined.detected_chars.is_empty();
    combined
}

/// Deterministically wraps `input` in POSIX single quotes `'...'`, escaping any interior `'`
/// as `'\''`. This guarantees 100% literal preservation across `sh`, `bash`, and `zsh` for
/// multi-line text, bullets (`*`, `-`), parentheses `( )`, quotes `"`, backticks `` ` ``,
/// slashes `/`, and `$VARS`.
pub fn posix_single_quote(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    out.push('\'');
    for ch in input.chars() {
        if ch == '\'' {
            out.push_str("'\\''");
        } else {
            out.push(ch);
        }
    }
    out.push('\'');
    out
}

/// Quotes a CLI token in POSIX single quotes only if it is empty or contains characters outside
/// the safe bareword set (`a-zA-Z0-9_./:-`).
pub fn posix_quote_token_if_needed(token: &str) -> String {
    if !token.is_empty()
        && token
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '/' | ':' | '-'))
    {
        token.to_string()
    } else {
        posix_single_quote(token)
    }
}

/// Escapes a string for safe inclusion inside double quotes `"..."` (`\\`, `\"`, `\$`, `` \` ``, `\!`).
pub fn escape_for_double_quotes(input: &str) -> String {
    let mut out = String::with_capacity(input.len() + 8);
    for ch in input.chars() {
        match ch {
            '\\' => out.push_str("\\\\"),
            '"' => out.push_str("\\\""),
            '$' => out.push_str("\\$"),
            '`' => out.push_str("\\`"),
            '!' => out.push_str("\\!"),
            _ => out.push(ch),
        }
    }
    out
}

/// Builds both the raw OS `argv` vector and the POSIX-quoted `shell_safe_command` string for an
/// `agy` invocation.
///
/// Places `--model`, `--effort`, and extra flags before `-p` / `-i` so that `-p` / `-i`
/// immediately precedes its prompt argument (`agy --model <ID> --effort <EFFORT> [EXTRA...] <-p|-i> <PROMPT>`).
pub fn build_shell_safe_agy_invocation(
    mode: AgyLaunchMode,
    model_id: &str,
    effort: EffortLevel,
    extra_flags: &AgyCliFlags,
    prompt: &str,
    _policy: ShellFormattingPolicy,
) -> (Vec<String>, String) {
    let mut argv: Vec<String> = vec![
        "agy".to_string(),
        "--model".to_string(),
        model_id.to_string(),
        "--effort".to_string(),
        effort.as_str().to_string(),
    ];
    argv.extend(extra_flags.to_cli_args());
    argv.push(mode.flag().to_string());
    argv.push(prompt.to_string());

    let mut shell_parts: Vec<String> = Vec::with_capacity(argv.len());
    for (idx, arg) in argv.iter().enumerate() {
        if idx + 1 == argv.len() {
            // Always POSIX single-quote the prompt argument so multi-word / multi-line / special-char
            // prompts are 100% shell-parseable and never break on bullets, parentheses, or quotes.
            shell_parts.push(posix_single_quote(arg));
        } else {
            shell_parts.push(posix_quote_token_if_needed(arg));
        }
    }

    (argv, shell_parts.join(" "))
}

/// Builds the single-line `agy` command preview for the TUI footer, including any configured
/// extra `agy` flags (`--mode`, `--sandbox`, etc.) and a safely escaped prompt preview.
pub fn build_agy_footer_preview_with_flags(
    mode: AgyLaunchMode,
    model_id: &str,
    effort: EffortLevel,
    extra_flags: &AgyCliFlags,
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
    let escaped = escape_for_double_quotes(&truncated);

    let extra_args = extra_flags.to_cli_args();
    let extra_str = if extra_args.is_empty() {
        String::new()
    } else {
        let quoted_extras: Vec<String> = extra_args
            .iter()
            .map(|a| posix_quote_token_if_needed(a))
            .collect();
        format!(" {}", quoted_extras.join(" "))
    };

    format!(
        "agy --model {} --effort {}{} {} \"{}\"",
        model_id,
        effort.as_str(),
        extra_str,
        mode.flag(),
        escaped
    )
}

/// Parses a POSIX shell command string into individual words (`shlex` equivalent), validating
/// that all single and double quotes and backslash escapes are balanced and well-formed.
pub fn parse_posix_shell_words(cmd: &str) -> Result<Vec<String>> {
    let mut words: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut in_word = false;
    let mut chars = cmd.chars().peekable();

    while let Some(ch) = chars.next() {
        match ch {
            '\'' => {
                in_word = true;
                let mut closed = false;
                for next_ch in chars.by_ref() {
                    if next_ch == '\'' {
                        closed = true;
                        break;
                    }
                    current.push(next_ch);
                }
                if !closed {
                    return Err(anyhow!("Unclosed single quote in shell command"));
                }
            }
            '"' => {
                in_word = true;
                let mut closed = false;
                while let Some(next_ch) = chars.next() {
                    if next_ch == '"' {
                        closed = true;
                        break;
                    } else if next_ch == '\\' {
                        if let Some(&peek_ch) = chars.peek() {
                            if matches!(peek_ch, '"' | '\\' | '$' | '`' | '\n') {
                                current.push(chars.next().unwrap());
                            } else {
                                current.push('\\');
                            }
                        } else {
                            return Err(anyhow!("Trailing backslash inside double quotes"));
                        }
                    } else {
                        current.push(next_ch);
                    }
                }
                if !closed {
                    return Err(anyhow!("Unclosed double quote in shell command"));
                }
            }
            '\\' => {
                in_word = true;
                if let Some(escaped_ch) = chars.next() {
                    current.push(escaped_ch);
                } else {
                    return Err(anyhow!("Trailing unescaped backslash in shell command"));
                }
            }
            c if c.is_whitespace() => {
                if in_word {
                    words.push(std::mem::take(&mut current));
                    in_word = false;
                }
            }
            c if matches!(c, ';' | '|' | '&' | '<' | '>' | '(' | ')' | '`' | '$') => {
                return Err(anyhow!(
                    "Unescaped shell metacharacter '{}' outside quotes in command",
                    c
                ));
            }
            other => {
                in_word = true;
                current.push(other);
            }
        }
    }

    if in_word {
        words.push(current);
    }

    Ok(words)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_inspect_complex_multiline_bullet_prompt_with_parens_quotes_and_slashes() {
        let mut values = PromptValues::new();
        values.slash_command = Some("plan".to_string());
        values.objective =
            "Please add the following features:\n* Create a Settings Page (Alt+S) and move theming/models into it\n* Allow model .toml ingestion in Settings\n* Automatically format text (quotes \"...\", single '...', $VARS, `code`, and /paths) for terminal execution".to_string();
        values.context_logs = "src/settings.rs, src/formatting.rs".to_string();
        values.constraints = "No regressions; preserve text on (Alt+S / Esc)".to_string();
        values.output = "Update STEPPED_IMPLEMENTATION_PLAN.md plan".to_string();

        let report = inspect_prompt_values_safety(&values);
        assert!(report.needs_escaping_or_quoting());
        assert!(report.has_special_chars);
        assert!(report.has_multiline);
        for expected_ch in ['*', '(', ')', '"', '\'', '$', '`', ';'] {
            assert!(
                report.detected_chars.contains(&expected_ch),
                "Expected '{}' to be detected in {:?}",
                expected_ch,
                report.detected_chars
            );
        }
        assert!(report.affected_fields.contains(&"Objective".to_string()));
        assert!(report.affected_fields.contains(&"Constraints".to_string()));
    }

    #[test]
    fn test_posix_single_quote_and_shell_word_roundtrip() {
        let complex_prompt = "/plan Please add the following features:\n* Create a Settings Page (Alt+S) and move theming + model addition into it\n* Allow model .toml file ingestion in Settings\n* Handle \"double quotes\", 'single quotes', $HOME, `backticks`, and (parentheses)!";
        let mut flags = AgyCliFlags {
            mode: AgyExecutionMode::Plan,
            sandbox: true,
            continue_session: false,
            dangerously_skip_permissions: false,
            add_dirs: vec!["/tmp/extra-dir".to_string()],
        };

        let (argv, shell_cmd) = build_shell_safe_agy_invocation(
            AgyLaunchMode::Interactive,
            "gemini-4-argon-medium",
            EffortLevel::High,
            &flags,
            complex_prompt,
            ShellFormattingPolicy::PromptOnSpecialChars,
        );

        let parsed_words =
            parse_posix_shell_words(&shell_cmd).expect("shell_cmd must be 100% POSIX parseable");
        assert_eq!(parsed_words, argv);
        assert_eq!(parsed_words.last().unwrap(), complex_prompt);

        // Verify footer preview includes configured flags and escaped double quotes
        flags.add_dirs.clear();
        let preview = build_agy_footer_preview_with_flags(
            AgyLaunchMode::Interactive,
            "gemini-4-argon-medium",
            EffortLevel::High,
            &flags,
            complex_prompt,
        );
        assert!(preview.starts_with(
            "agy --model gemini-4-argon-medium --effort high --mode plan --sandbox -i \""
        ));
    }
}
