use crate::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use unicode_width::UnicodeWidthStr;

/// Compact single-line speaker-eared boxy robot emblem.
pub const COMPACT_ROBOT_MASCOT: &str = "◁(( [◉_◉] ))▷";

/// Application display name and CLI command name.
pub const APP_NAME: &str = "VERBOCITY";
pub const CLI_NAME: &str = "verbot";

/// Minimum inner display width and terminal height for the Full 7-Row Banner.
pub const FULL_BANNER_MIN_WIDTH: u16 = 92;
pub const FULL_BANNER_MIN_TERMINAL_HEIGHT: u16 = 38;

/// Minimum inner display width and terminal height for the Medium 3-Row Robot + Wordmark Banner.
pub const MEDIUM_BANNER_MIN_WIDTH: u16 = 60;
pub const MEDIUM_BANNER_MIN_TERMINAL_HEIGHT: u16 = 32;

/// Fixed, non-warping branding banner tiers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BannerTier {
    /// 7-row ASCII block wordmark + multi-line speaker-eared robot head (`inner_width >= 92 && terminal_height >= 38`).
    Full7Row,
    /// 3-row speaker-eared robot head + wordmark (`inner_width >= 60 && terminal_height >= 32`).
    Medium3Row,
    /// 1-row compact emblem `◁(( [◉_◉] ))▷ VERBOCITY (verbot)` (smaller heights/widths).
    Compact1Row,
}

impl BannerTier {
    pub const fn line_count(self) -> u16 {
        match self {
            Self::Full7Row => 7,
            Self::Medium3Row => 3,
            Self::Compact1Row => 1,
        }
    }
}

/// Multi-line cartoon boxy robot head with speaker-cone ears (`◁(( ... ))▷`) and antenna (`▄█▄`).
pub const ROBOT_HEAD_ASCII: [&str; 6] = [
    "     ▄█▄     ",
    "  ╭───────╮  ",
    "◁((│ ◉   ◉ │))▷",
    "◁((│  ═══  │))▷",
    "  ╰──┬─┬──╯  ",
    "  (( [◉_◉] ))",
];

/// Multi-line block lettering for `VERBOCITY`.
pub const VERBOCITY_BLOCK_ASCII: [&str; 6] = [
    "██╗   ██╗███████╗██████╗ ██████╗  ██████╗  ██████╗██╗████████╗██╗   ██╗",
    "██║   ██║██╔════╝██╔══██╗██╔══██╗██╔═══██╗██╔════╝██║╚══██╔══╝╚██╗ ██╔╝",
    "██║   ██║█████╗  ██████╔╝██████╔╝██║   ██║██║     ██║   ██║    ╚████╔╝ ",
    "╚██╗ ██╔╝██╔══╝  ██╔══██╗██╔══██╗██║   ██║██║     ██║   ██║     ╚██╔╝  ",
    " ╚████╔╝ ███████╗██║  ██║██████╔╝╚██████╔╝╚██████╗██║   ██║      ██║   ",
    "  ╚═══╝  ╚══════╝╚═╝  ╚═╝╚═════╝  ╚═════╝  ╚═════╝╚═╝   ╚═╝      ╚═╝   ",
];

/// Selects the non-warping [`BannerTier`] for a given `inner_width` and `terminal_height`.
pub fn select_banner_tier(inner_width: u16, terminal_height: u16) -> BannerTier {
    if inner_width >= FULL_BANNER_MIN_WIDTH && terminal_height >= FULL_BANNER_MIN_TERMINAL_HEIGHT {
        BannerTier::Full7Row
    } else if inner_width >= MEDIUM_BANNER_MIN_WIDTH
        && terminal_height >= MEDIUM_BANNER_MIN_TERMINAL_HEIGHT
    {
        BannerTier::Medium3Row
    } else {
        BannerTier::Compact1Row
    }
}

/// Measures the exact display-column width of a Ratatui [`Line`] using `unicode-width`.
pub fn line_display_width(line: &Line<'_>) -> usize {
    line.spans
        .iter()
        .map(|s| UnicodeWidthStr::width(s.content.as_ref()))
        .sum()
}

/// Returns the height in terminal rows required by the responsive header banner for a given width.
pub fn banner_height(width: u16) -> u16 {
    select_banner_tier(width, FULL_BANNER_MIN_TERMINAL_HEIGHT).line_count()
}

/// Returns the height in terminal rows required by the header banner for `(inner_width, terminal_height)`.
pub fn banner_height_for_terminal(inner_width: u16, terminal_height: u16) -> u16 {
    select_banner_tier(inner_width, terminal_height).line_count()
}

/// Renders the branding header as plain text (used for CLI `--self-test`, logs, and snapshot verification).
pub fn render_banner_plain(width: u16) -> String {
    render_banner_plain_for_terminal(width, FULL_BANNER_MIN_TERMINAL_HEIGHT)
}

/// Renders the branding header as plain text for `(inner_width, terminal_height)`.
pub fn render_banner_plain_for_terminal(inner_width: u16, terminal_height: u16) -> String {
    match select_banner_tier(inner_width, terminal_height) {
        BannerTier::Full7Row => {
            let mut out = Vec::with_capacity(7);
            for i in 0..6 {
                out.push(format!(
                    " {}   {}",
                    ROBOT_HEAD_ASCII[i], VERBOCITY_BLOCK_ASCII[i]
                ));
            }
            out.push(format!(
                " {}  VERBOCITY ({}) :: Interactive LLM Prompt Template Engine & agy Dispatcher",
                COMPACT_ROBOT_MASCOT, CLI_NAME
            ));
            out.join("\n")
        }
        BannerTier::Medium3Row => vec![
            "   ╭───────╮    VERBOCITY (verbot)".to_string(),
            " ◁((│ ◉   ◉ │))▷ LLM Prompt Engine & agy Dispatcher".to_string(),
            "   ╰──═══──╯    (( [◉_◉] )) NovOS Cosmic Nova Orange".to_string(),
        ]
        .join("\n"),
        BannerTier::Compact1Row => {
            format!("{} {} ({})", COMPACT_ROBOT_MASCOT, APP_NAME, CLI_NAME)
        }
    }
}

/// Renders the responsive branding banner into styled Ratatui [`Line`]s using the active [`Theme`].
pub fn render_banner_lines(width: u16, theme: &Theme) -> Vec<Line<'static>> {
    render_banner_lines_for_terminal(width, FULL_BANNER_MIN_TERMINAL_HEIGHT, theme)
}

/// Renders the non-warping 3-tier branding banner for `(inner_width, terminal_height)`.
pub fn render_banner_lines_for_terminal(
    inner_width: u16,
    terminal_height: u16,
    theme: &Theme,
) -> Vec<Line<'static>> {
    let bg = theme.surface_color();

    let style_antenna = Style::default()
        .fg(theme.highlight_color())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let style_speaker = Style::default()
        .fg(theme.secondary_accent_color())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let style_chassis = Style::default()
        .fg(theme.accent_color())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let style_eyes = Style::default()
        .fg(theme.highlight_color())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let style_mouth = Style::default()
        .fg(theme.tertiary_accent_color())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let style_wordmark_top = Style::default()
        .fg(theme.highlight_color())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let style_wordmark_mid = Style::default()
        .fg(theme.accent_color())
        .bg(bg)
        .add_modifier(Modifier::BOLD);
    let style_wordmark_bot = Style::default()
        .fg(theme.secondary_accent_color())
        .bg(bg);
    let style_subtitle = Style::default().fg(theme.text_color()).bg(bg);

    match select_banner_tier(inner_width, terminal_height) {
        BannerTier::Full7Row => vec![
            Line::from(vec![
                Span::styled("      ", style_subtitle),
                Span::styled("▄█▄", style_antenna),
                Span::styled("        ", style_subtitle),
                Span::styled(VERBOCITY_BLOCK_ASCII[0], style_wordmark_top),
            ]),
            Line::from(vec![
                Span::styled("   ╭───────╮     ", style_chassis),
                Span::styled(VERBOCITY_BLOCK_ASCII[1], style_wordmark_mid),
            ]),
            Line::from(vec![
                Span::styled(" ◁((", style_speaker),
                Span::styled("│ ", style_chassis),
                Span::styled("◉   ◉", style_eyes),
                Span::styled(" │", style_chassis),
                Span::styled("))▷  ", style_speaker),
                Span::styled(VERBOCITY_BLOCK_ASCII[2], style_wordmark_mid),
            ]),
            Line::from(vec![
                Span::styled(" ◁((", style_speaker),
                Span::styled("│  ", style_chassis),
                Span::styled("═══", style_mouth),
                Span::styled("  │", style_chassis),
                Span::styled("))▷  ", style_speaker),
                Span::styled(VERBOCITY_BLOCK_ASCII[3], style_wordmark_mid),
            ]),
            Line::from(vec![
                Span::styled("   ╰──┬─┬──╯     ", style_chassis),
                Span::styled(VERBOCITY_BLOCK_ASCII[4], style_wordmark_bot),
            ]),
            Line::from(vec![
                Span::styled("   ", style_subtitle),
                Span::styled("(( ", style_speaker),
                Span::styled("[◉_◉]", style_eyes),
                Span::styled(" ))", style_speaker),
                Span::styled("    ", style_subtitle),
                Span::styled(VERBOCITY_BLOCK_ASCII[5], style_wordmark_bot),
            ]),
            Line::from(vec![
                Span::styled(" ", style_subtitle),
                Span::styled(COMPACT_ROBOT_MASCOT, style_speaker),
                Span::styled("  ", style_subtitle),
                Span::styled(APP_NAME, style_wordmark_top),
                Span::styled(
                    format!(" ({}) — Interactive LLM Prompt Engine & agy Dispatcher", CLI_NAME),
                    style_subtitle,
                ),
            ]),
        ],
        BannerTier::Medium3Row => vec![
            Line::from(vec![
                Span::styled("   ╭───────╮    ", style_chassis),
                Span::styled("VERBOCITY", style_wordmark_top),
                Span::styled(" (verbot)", style_speaker),
            ]),
            Line::from(vec![
                Span::styled(" ◁((", style_speaker),
                Span::styled("│ ", style_chassis),
                Span::styled("◉   ◉", style_eyes),
                Span::styled(" │", style_chassis),
                Span::styled("))▷ ", style_speaker),
                Span::styled("LLM Prompt Engine & agy Dispatcher", style_subtitle),
            ]),
            Line::from(vec![
                Span::styled("   ╰──═══──╯    ", style_chassis),
                Span::styled("(( [◉_◉] )) ", style_speaker),
                Span::styled(format!("Theme: {}", theme.name), style_mouth),
            ]),
        ],
        BannerTier::Compact1Row => vec![Line::from(vec![
            Span::styled(" ", style_subtitle),
            Span::styled(COMPACT_ROBOT_MASCOT, style_speaker),
            Span::styled(" ", style_subtitle),
            Span::styled(APP_NAME, style_wordmark_top),
            Span::styled(format!(" ({})", CLI_NAME), style_mouth),
        ])],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_branding_banners_include_robot_and_verbocity() {
        let wide = render_banner_plain(110);
        assert!(wide.contains("VERBOCITY"));
        assert!(wide.contains("(( [◉_◉] ))"));
        assert!(wide.contains("◁((│ ◉   ◉ │))▷"));

        let med = render_banner_plain(72);
        assert!(med.contains("VERBOCITY"));
        assert!(med.contains("(( [◉_◉] ))"));

        let compact = render_banner_plain(48);
        assert!(compact.contains("VERBOCITY"));
        assert!(compact.contains("(( [◉_◉] ))"));
    }

    #[test]
    fn test_styled_banner_line_counts_and_non_warping_widths() {
        let theme = Theme::novos_cosmic_dark();

        // Tier 1: Full 7-Row (150x45 -> inner_width=148 >= 92, height=45 >= 38)
        let full_lines = render_banner_lines_for_terminal(148, 45, &theme);
        assert_eq!(full_lines.len(), 7);
        for line in &full_lines {
            assert!(
                line_display_width(line) <= 92,
                "Full banner line exceeded 92 cols: {}",
                line_display_width(line)
            );
        }

        // Tier 2: Medium 3-Row (100x36 -> inner_width=98 >= 60, height=36 in 32..38)
        let med_lines = render_banner_lines_for_terminal(98, 36, &theme);
        assert_eq!(med_lines.len(), 3);
        for line in &med_lines {
            assert!(
                line_display_width(line) <= 60,
                "Medium banner line exceeded 60 cols: {}",
                line_display_width(line)
            );
        }

        // Tier 3: Compact 1-Row (75x30 -> inner_width=73, height=30 < 32)
        let compact_lines = render_banner_lines_for_terminal(73, 30, &theme);
        assert_eq!(compact_lines.len(), 1);
        assert!(line_display_width(&compact_lines[0]) <= 60);
    }
}
