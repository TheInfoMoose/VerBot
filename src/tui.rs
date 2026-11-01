use crate::app::{
    App, ControlItem, DispatchAction, DispatchManifest, FieldEditor, FocusPane, ModelModalField,
};
use crate::branding::{
    render_banner_lines_for_terminal, select_banner_tier, BannerTier, APP_NAME, CLI_NAME,
    COMPACT_ROBOT_MASCOT,
};
use crate::settings::SettingsTab;
use crate::theme::RgbColor;
use anyhow::{Context, Result};
use crossterm::event::{self, Event, KeyEventKind};
use crossterm::execute;
use crossterm::terminal::{
    disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen,
};
use ratatui::backend::{CrosstermBackend, TestBackend};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, List, ListItem, Paragraph, Wrap};
use ratatui::{Frame, Terminal};
use std::io;
use std::time::Duration;
use unicode_width::UnicodeWidthStr;

/// Runs the interactive multi-pane Ratatui interface on `/dev/tty` (or `stdout`) until the user
/// explicitly quits (`Ctrl+Q`). Executing actions (`agy -i`, `agy -p`, `Stdout / Clipboard`, `Save .md`)
/// keeps Verbocity open.
pub fn run_interactive_tui(app: &mut App) -> Result<Option<DispatchManifest>> {
    enable_raw_mode().context("Failed to enable terminal raw mode")?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen).context("Failed to enter alternate screen")?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend).context("Failed to initialize Ratatui terminal")?;

    let loop_res = run_event_loop(&mut terminal, app);

    let _ = disable_raw_mode();
    let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
    let _ = terminal.show_cursor();

    loop_res?;
    Ok(app.confirmed_manifest.clone())
}

fn run_event_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    app: &mut App,
) -> Result<()> {
    while !app.should_quit {
        if let Some(manifest) = app.pending_interactive_handoff.take() {
            // Temporarily leave Ratatui alternate screen and raw mode so `agy -i` gets `/dev/tty` cleanly
            let _ = disable_raw_mode();
            let _ = execute!(terminal.backend_mut(), LeaveAlternateScreen);
            let _ = terminal.show_cursor();

            let _ = app.run_interactive_handoff_child(&manifest);

            // Re-enter Ratatui alternate screen and raw mode, resuming Verbocity with all state intact
            let _ = enable_raw_mode();
            let _ = execute!(terminal.backend_mut(), EnterAlternateScreen);
            let _ = terminal.hide_cursor();
            let _ = terminal.clear();
        }

        app.refresh_editor_themes();
        terminal.draw(|frame| render_ui(frame, app))?;

        if event::poll(Duration::from_millis(100))? {
            if let Event::Key(key) = event::read()? {
                if key.kind == KeyEventKind::Press {
                    app.handle_key_event(key)?;
                }
            }
        }
    }
    Ok(())
}

/// Renders a single frame of `App` headlessly into a `ratatui::backend::TestBackend` buffer.
pub fn render_headless_buffer(app: &mut App, width: u16, height: u16) -> Result<Buffer> {
    app.refresh_editor_themes();
    let backend = TestBackend::new(width, height);
    let mut terminal = Terminal::new(backend)?;
    terminal.draw(|frame| render_ui(frame, app))?;
    Ok(terminal.backend().buffer().clone())
}

/// Converts a `ratatui::buffer::Buffer` into a newline-separated plain-text string for inspection.
pub fn buffer_to_string(buf: &Buffer) -> String {
    let area = buf.area;
    let mut lines = Vec::with_capacity(area.height as usize);
    for y in area.top()..area.bottom() {
        let mut row = String::with_capacity(area.width as usize);
        for x in area.left()..area.right() {
            row.push_str(buf[(x, y)].symbol());
        }
        lines.push(row.trim_end().to_string());
    }
    lines.join("\n")
}

/// Minimum supported terminal dimensions before displaying the compact resize guard notice.
/// Enforces sentence-length minimum width (`75` cols) and height (`28` rows) so all 4 field editors
/// always receive `>= 75` cols width and `>= 6` rows height (`4` inner text rows + `2` border rows).
pub const MIN_TERMINAL_WIDTH: u16 = 75;
pub const MIN_TERMINAL_HEIGHT: u16 = 28;

/// Minimum dimensions guaranteed for every field editor box (`Objective`, `Context/Logs`, `Constraints`, `Output`).
pub const MIN_FIELD_EDITOR_WIDTH: u16 = 75;
pub const MIN_FIELD_EDITOR_HEIGHT: u16 = 6;
pub const MIN_FIELD_INNER_TEXT_ROWS: u16 = 4;

/// Minimum terminal width required to show the Live Prompt Preview side-by-side without shrinking
/// the field editor column below `MIN_FIELD_EDITOR_WIDTH` (`75` columns).
pub const SIDE_BY_SIDE_PREVIEW_MIN_WIDTH: u16 = 140;

/// Computes the top-level `[Header, ControlBar, Body, Footer]` layout rectangles, guaranteeing
/// that `Body` always receives at least `4 * MIN_FIELD_EDITOR_HEIGHT` (`24`) rows when `area >= 75x28`.
pub fn compute_root_chunks(area: Rect, app: &App) -> [Rect; 4] {
    let editor_count = (app.editors.len().max(1)) as u16;
    let min_body_rows = editor_count * MIN_FIELD_EDITOR_HEIGHT;
    let inner_w = area.width.saturating_sub(2);
    let tier = select_banner_tier(inner_w, area.height);

    let (header_rows, control_rows, footer_rows) =
        if area.height >= 38 && tier == BannerTier::Full7Row {
            let h_rows = 9u16; // 7 banner lines + 2 border rows
            let c_lines = build_wrapped_control_lines(app, inner_w).len() as u16;
            let c_rows = (c_lines + 2).min(5);
            let avail_rem = area.height.saturating_sub(min_body_rows + h_rows + c_rows);
            let f_rows = avail_rem.clamp(3, 5);
            (h_rows, c_rows, f_rows)
        } else if area.height >= 32 && tier == BannerTier::Medium3Row {
            let h_rows = 5u16; // 3 banner lines + 2 border rows
            let avail_chrome = area.height.saturating_sub(min_body_rows + h_rows);
            let c_lines = build_wrapped_control_lines(app, inner_w).len() as u16;
            let c_rows = if avail_chrome >= c_lines + 5 {
                c_lines + 2
            } else {
                (c_lines + 1).min(avail_chrome.saturating_sub(3).max(2))
            };
            let f_rows = avail_chrome.saturating_sub(c_rows).clamp(3, 5);
            (h_rows, c_rows, f_rows)
        } else {
            let h_rows = 1u16; // 1-row compact non-warping emblem
            let avail_chrome = area.height.saturating_sub(min_body_rows + h_rows);
            let c_lines = build_wrapped_control_lines(app, area.width).len() as u16;
            let c_rows = c_lines.min(avail_chrome.saturating_sub(1).max(2));
            let f_rows = avail_chrome.saturating_sub(c_rows).max(1);
            (h_rows, c_rows, f_rows)
        };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_rows),
            Constraint::Length(control_rows),
            Constraint::Min(min_body_rows),
            Constraint::Length(footer_rows),
        ])
        .split(area);

    [chunks[0], chunks[1], chunks[2], chunks[3]]
}

/// Computes the `(editors_rect, optional_aux_rect)` split inside `body_area`, enforcing that
/// `editors_rect.width >= 75` and `editors_rect.height >= 24`.
pub fn compute_body_rects(body_area: Rect, app: &App) -> (Rect, Option<Rect>) {
    let editor_count = (app.editors.len().max(1)) as u16;
    let min_body_rows = editor_count * MIN_FIELD_EDITOR_HEIGHT;
    let show_aux = app.show_preview || app.terminal_output.is_open;

    if show_aux {
        if body_area.width >= SIDE_BY_SIDE_PREVIEW_MIN_WIDTH {
            let editor_w = ((body_area.width as u32 * 56) / 100) as u16;
            let editor_w = editor_w
                .max(MIN_FIELD_EDITOR_WIDTH)
                .min(body_area.width.saturating_sub(30));
            let preview_w = body_area.width.saturating_sub(editor_w);
            let chunks = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Length(editor_w), Constraint::Length(preview_w)])
                .split(body_area);
            return (chunks[0], Some(chunks[1]));
        } else if body_area.height >= min_body_rows + 6 {
            let preview_h = body_area
                .height
                .saturating_sub(min_body_rows)
                .min(12)
                .max(6);
            let editor_h = body_area.height.saturating_sub(preview_h);
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(editor_h), Constraint::Length(preview_h)])
                .split(body_area);
            return (chunks[0], Some(chunks[1]));
        }
    }

    (body_area, None)
}

/// Computes the individual [`Rect`] for each of the field editors inside `editor_column_area`.
/// Guarantees every field editor receives at least `MIN_FIELD_EDITOR_HEIGHT` (`6`) rows, and
/// prioritizes allocating extra rows to editors with layered choice/pinned sub-headers (`Output`, `Constraints`).
pub fn compute_field_editor_rects(editor_column_area: Rect, app: &App) -> Vec<Rect> {
    let count = app.editors.len();
    if count == 0 {
        return Vec::new();
    }
    let n = count as u16;
    let base = (editor_column_area.height / n).max(1);
    let mut heights = vec![base; count];
    let mut rem = editor_column_area.height.saturating_sub(base * n);

    // Prioritize giving extra rows to fields with choices (`Output`) or pinned constraints first
    for idx in (0..count).rev() {
        if rem == 0 {
            break;
        }
        if !app.editors[idx].spec.choices.is_empty() || app.editors[idx].spec.id == "constraints" {
            heights[idx] += 1;
            rem -= 1;
        }
    }
    let mut i = 0;
    while rem > 0 {
        heights[i % count] += 1;
        rem -= 1;
        i += 1;
    }

    let constraints: Vec<Constraint> = heights.into_iter().map(Constraint::Length).collect();
    Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(editor_column_area)
        .iter()
        .copied()
        .collect()
}

/// Helper for tests and diagnostics: computes the exact [`Rect`] of every field editor for a
/// terminal of size `(width, height)`.
pub fn compute_field_editor_rects_for_terminal(width: u16, height: u16, app: &App) -> Vec<Rect> {
    let root = Rect::new(0, 0, width, height);
    let chunks = compute_root_chunks(root, app);
    let (editor_col, _) = compute_body_rects(chunks[2], app);
    compute_field_editor_rects(editor_col, app)
}

/// Renders the complete multi-pane Verbocity (`verbot`) interface.
pub fn render_ui(frame: &mut Frame, app: &App) {
    let area = frame.area();
    let theme = &app.theme;

    // Paint root background in Deep Obsidian (`#0B0C10` in NovOS Cosmic Dark)
    let bg_block = Block::default().style(theme.base_style());
    frame.render_widget(bg_block, area);

    // Minimum window size guard (`75x28`)
    if area.width < MIN_TERMINAL_WIDTH || area.height < MIN_TERMINAL_HEIGHT {
        render_min_size_guard(frame, area, app);
        return;
    }

    if app.settings_page.is_open {
        render_settings_page(frame, area, app);
        if app.file_picker.is_open {
            render_file_picker_modal(frame, area, app);
        }
        return;
    }

    let root_chunks = compute_root_chunks(area, app);

    render_header_banner(frame, root_chunks[0], app);
    render_control_bar(frame, root_chunks[1], app);
    render_body_panes(frame, root_chunks[2], app);
    render_footer(frame, root_chunks[3], app);

    if app.pending_plan_picker_approval.is_some() {
        render_plan_picker_approval_banner(frame, area, app);
    }

    if app.file_picker.is_open {
        render_file_picker_modal(frame, area, app);
    }

    if app.model_modal.is_open {
        render_model_manager_modal(frame, area, app);
    }

    if app.shell_quote_modal.is_open {
        render_shell_quote_modal(frame, area, app);
    }
}

fn render_min_size_guard(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused_style())
        .title(Span::styled(
            " VERBOCITY (verbot) — Window Too Small ",
            theme.title_focused_style(),
        ))
        .style(theme.surface_style());

    let lines = vec![
        Line::from(Span::styled(
            format!(
                "Minimum size: {}x{} (current: {}x{})",
                MIN_TERMINAL_WIDTH, MIN_TERMINAL_HEIGHT, area.width, area.height
            ),
            Style::default()
                .fg(theme.accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            "Resize terminal window (>= 75 cols x 28 rows) or press Ctrl+Q to quit.",
            theme.surface_style(),
        )),
    ];

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(theme.surface_style());
    frame.render_widget(para, area);
}

fn render_header_banner(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let total_terminal_height = frame.area().height;

    if area.height <= 1 {
        let bg = theme.surface_color();
        let line = Line::from(vec![
            Span::styled(" ", theme.surface_style()),
            Span::styled(
                COMPACT_ROBOT_MASCOT,
                Style::default()
                    .fg(theme.secondary_accent_color())
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(" ", theme.surface_style()),
            Span::styled(
                APP_NAME,
                Style::default()
                    .fg(theme.highlight_color())
                    .bg(bg)
                    .add_modifier(Modifier::BOLD),
            ),
            Span::styled(
                format!(" ({}) — Theme: {} [Ctrl+T]", CLI_NAME, theme.name),
                Style::default().fg(theme.tertiary_accent_color()).bg(bg),
            ),
        ]);
        // Never attach `.wrap(...)` so branding banner never warps or shears
        let paragraph = Paragraph::new(vec![line]).style(theme.surface_style());
        frame.render_widget(paragraph, area);
        return;
    }

    let inner_width = area.width.saturating_sub(2);
    let banner_lines =
        render_banner_lines_for_terminal(inner_width, total_terminal_height, theme);
    let borders = if area.height <= 2 {
        Borders::TOP
    } else {
        Borders::ALL
    };

    let block = Block::default()
        .borders(borders)
        .border_style(Style::default().fg(theme.accent_color()).bg(theme.surface_color()))
        .title(Span::styled(
            format!(
                " VERBOCITY (verbot) — Theme: {} [Ctrl+T] | Template: {} ",
                theme.name, app.template.name
            ),
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme.surface_style());

    // Never attach `.wrap(...)` on the branding header so ASCII art never warps
    let paragraph = Paragraph::new(banner_lines)
        .block(block)
        .style(theme.surface_style());
    frame.render_widget(paragraph, area);
}

/// Builds width-aware wrapped [`Line`]s for the top Control Bar so that no selector or value is
/// ever cut off at any terminal width `>= 75`.
pub fn build_wrapped_control_lines(app: &App, avail_width: u16) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let profile = app.active_profile();
    let cmd_str = app
        .values
        .formatted_slash_command()
        .unwrap_or_else(|| "(none)".to_string());

    let compact_pills = avail_width < 110;
    let settings_val = if avail_width >= 115 {
        format!(
            "Theme: {} | {} Models",
            theme.name,
            app.catalog.profiles.len()
        )
    } else if avail_width >= 90 {
        format!("{} | {}m", theme.name, app.catalog.profiles.len())
    } else {
        "Open".to_string()
    };

    let items: [(ControlItem, &str, String); 5] = if compact_pills {
        [
            (
                ControlItem::ModelFamily,
                "Family",
                app.selected_family.as_str().to_string(),
            ),
            (
                ControlItem::SubModel,
                "Sub-Model (Alt+M)",
                format!("{} [{}]", profile.short_name, app.active_agy_model_id()),
            ),
            (
                ControlItem::Effort,
                "Effort",
                app.selected_effort.as_str().to_string(),
            ),
            (ControlItem::SlashCommand, "Command", cmd_str),
            (ControlItem::Settings, "Settings (Alt+S)", settings_val),
        ]
    } else {
        [
            (
                ControlItem::ModelFamily,
                "Family",
                app.selected_family.as_str().to_string(),
            ),
            (
                ControlItem::SubModel,
                "Sub-Model (Alt+M)",
                format!("{} [{}]", profile.short_name, app.active_agy_model_id()),
            ),
            (
                ControlItem::Effort,
                "Effort (Alt+E)",
                app.selected_effort.as_str().to_string(),
            ),
            (ControlItem::SlashCommand, "Command", cmd_str),
            (ControlItem::Settings, "Settings (Alt+S)", settings_val),
        ]
    };

    let max_cols = (avail_width as usize).max(40);
    let sep_str = if compact_pills { " │ " } else { " │ " };
    let sep_w = UnicodeWidthStr::width(sep_str);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> = vec![Span::styled(" ", theme.surface_style())];
    let mut current_w: usize = 1;

    for (item_kind, label, val) in items {
        let is_selected = app.focus == FocusPane::ControlBar(item_kind);
        let label_text = format!("{}: ", label);
        let pill_text = if compact_pills {
            format!("◄{}►", val)
        } else {
            format!(" ◄ {} ► ", val)
        };
        let item_w =
            UnicodeWidthStr::width(label_text.as_str()) + UnicodeWidthStr::width(pill_text.as_str());

        let has_items_on_line = current_spans.len() > 1;
        if has_items_on_line && current_w + sep_w + item_w > max_cols {
            lines.push(Line::from(current_spans));
            current_spans = vec![Span::styled(" ", theme.surface_style())];
            current_w = 1;
        } else if has_items_on_line {
            current_spans.push(Span::styled(
                sep_str,
                Style::default()
                    .fg(theme.border_color())
                    .bg(theme.surface_color()),
            ));
            current_w += sep_w;
        }

        let label_style = Style::default()
            .fg(theme.secondary_accent_color())
            .bg(theme.surface_color());
        let val_style = if is_selected {
            Style::default()
                .fg(theme.bg_color())
                .bg(theme.accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD)
        };

        current_spans.push(Span::styled(label_text, label_style));
        current_spans.push(Span::styled(pill_text, val_style));
        current_w += item_w;
    }

    if current_spans.len() > 1 {
        lines.push(Line::from(current_spans));
    }

    lines
}

fn render_control_bar(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let is_bar_focused = matches!(app.focus, FocusPane::ControlBar(_));

    let border_style = if is_bar_focused {
        theme.border_focused_style()
    } else {
        theme.border_inactive_style()
    };

    let lines_no_border = build_wrapped_control_lines(app, area.width);
    let lines_with_border = build_wrapped_control_lines(app, area.width.saturating_sub(2));

    let (borders, lines) = if area.height >= lines_with_border.len() as u16 + 2 {
        (Borders::ALL, lines_with_border)
    } else if area.height >= lines_no_border.len() as u16 + 1 {
        (Borders::TOP, lines_no_border)
    } else {
        (Borders::NONE, lines_no_border)
    };

    if borders == Borders::NONE {
        let paragraph = Paragraph::new(lines).style(theme.surface_style());
        frame.render_widget(paragraph, area);
        return;
    }

    let title_style = if is_bar_focused {
        theme.title_focused_style()
    } else {
        theme.title_inactive_style()
    };

    let block = Block::default()
        .borders(borders)
        .border_style(border_style)
        .title(Span::styled(
            " Model, Effort, Command, Theme & Catalog Selectors (Tab / Arrows / Alt+M / Alt+U / Ctrl+T) ",
            title_style,
        ))
        .style(theme.surface_style());

    let paragraph = Paragraph::new(lines)
        .block(block)
        .style(theme.surface_style());
    frame.render_widget(paragraph, area);
}

fn render_body_panes(frame: &mut Frame, area: Rect, app: &App) {
    let (editor_area, aux_area) = compute_body_rects(area, app);
    render_field_editors(frame, editor_area, app);
    if let Some(aux_rect) = aux_area {
        match (app.show_preview, app.terminal_output.is_open) {
            (true, true) => {
                if aux_rect.height >= 12 {
                    let preview_h = (aux_rect.height * 45) / 100;
                    let term_h = aux_rect.height.saturating_sub(preview_h);
                    let chunks = Layout::default()
                        .direction(Direction::Vertical)
                        .constraints([Constraint::Length(preview_h), Constraint::Length(term_h)])
                        .split(aux_rect);
                    render_live_preview(frame, chunks[0], app);
                    render_terminal_output_pane(frame, chunks[1], app);
                } else {
                    render_terminal_output_pane(frame, aux_rect, app);
                }
            }
            (false, true) => {
                render_terminal_output_pane(frame, aux_rect, app);
            }
            (true, false) => {
                render_live_preview(frame, aux_rect, app);
            }
            (false, false) => {}
        }
    } else if app.terminal_output.is_open {
        render_terminal_output_modal(frame, area, app);
    }
}

/// Breaks logical editor lines into width-aware soft-wrapped visual lines and projects the logical
/// `(cursor_row, cursor_col)` onto `(visual_cursor_row, visual_cursor_col)`.
pub fn wrap_editor_lines_with_cursor(
    lines: &[String],
    cursor: (usize, usize),
    max_width: usize,
) -> (Vec<String>, usize, usize) {
    let eff_width = max_width.max(1);
    let mut visual_lines: Vec<String> = Vec::new();
    let mut visual_cursor_row = 0usize;
    let mut visual_cursor_col = 0usize;
    let (cursor_row, cursor_col) = cursor;

    if lines.is_empty() {
        visual_lines.push(String::new());
        return (visual_lines, 0, 0);
    }

    for (row_idx, line) in lines.iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if chars.is_empty() {
            let v_row = visual_lines.len();
            visual_lines.push(String::new());
            if row_idx == cursor_row {
                visual_cursor_row = v_row;
                visual_cursor_col = 0;
            }
            continue;
        }

        let mut pos = 0usize;
        let mut matched_cursor = false;

        while pos < chars.len() {
            let remaining = chars.len() - pos;
            let seg_end = if remaining <= eff_width {
                chars.len()
            } else {
                // Prefer wrapping at the last whitespace within `eff_width` when available
                let window = &chars[pos..pos + eff_width];
                if let Some(rel_ws) = window.iter().rposition(|c| c.is_whitespace()) {
                    if rel_ws > 0 {
                        pos + rel_ws + 1
                    } else {
                        pos + eff_width
                    }
                } else {
                    pos + eff_width
                }
            };

            let v_row = visual_lines.len();
            let seg_str: String = chars[pos..seg_end].iter().collect();
            visual_lines.push(seg_str);

            if row_idx == cursor_row && !matched_cursor {
                if cursor_col >= pos && cursor_col < seg_end {
                    visual_cursor_row = v_row;
                    visual_cursor_col = (cursor_col - pos).min(eff_width.saturating_sub(1));
                    matched_cursor = true;
                } else if cursor_col == seg_end && seg_end == chars.len() {
                    let seg_len = seg_end - pos;
                    if seg_len < eff_width {
                        visual_cursor_row = v_row;
                        visual_cursor_col = seg_len;
                    } else {
                        let next_v_row = visual_lines.len();
                        visual_lines.push(String::new());
                        visual_cursor_row = next_v_row;
                        visual_cursor_col = 0;
                    }
                    matched_cursor = true;
                }
            }

            pos = seg_end;
        }

        if row_idx == cursor_row && !matched_cursor {
            visual_cursor_row = visual_lines.len().saturating_sub(1);
            let last_len = visual_lines
                .last()
                .map(|s| s.chars().count())
                .unwrap_or(0);
            visual_cursor_col = last_len.min(eff_width.saturating_sub(1));
        }
    }

    (visual_lines, visual_cursor_row, visual_cursor_col)
}

fn render_field_editors(frame: &mut Frame, area: Rect, app: &App) {
    if app.editors.is_empty() {
        return;
    }
    let chunks = compute_field_editor_rects(area, app);

    let pinned_summary = if app.workspace_state.pinned_constraints.is_empty() {
        None
    } else {
        Some(app.workspace_state.pinned_constraints.join(", "))
    };

    for (idx, editor) in app.editors.iter().enumerate() {
        if let Some(&pane_area) = chunks.get(idx) {
            let is_focused = app.focus == FocusPane::FieldEditor(idx)
                && !app.file_picker.is_open
                && !app.model_modal.is_open;
            let badge = if editor.spec.id == "constraints" {
                pinned_summary.as_deref()
            } else {
                None
            };
            render_wrapped_field_editor(frame, pane_area, editor, &app.theme, is_focused, badge);
        }
    }
}

fn render_wrapped_field_editor(
    frame: &mut Frame,
    pane_area: Rect,
    editor: &FieldEditor,
    theme: &crate::theme::Theme,
    focused: bool,
    pinned_badge: Option<&str>,
) {
    if pane_area.width == 0 || pane_area.height == 0 {
        return;
    }

    let surface_style = theme.surface_style();
    let border_style = if focused {
        theme.border_focused_style()
    } else {
        theme.border_inactive_style()
    };
    let title_style = if focused {
        theme.title_focused_style()
    } else {
        theme.title_inactive_style()
    };
    let badge_style = if focused {
        theme.badge_focused_style()
    } else {
        theme.badge_inactive_style()
    };

    let primary_label = format!(" {} ", editor.spec.label);
    let files_badge = if focused {
        "(@/Ctrl+F: Files) "
    } else {
        "[@/Ctrl+F] "
    };

    let choices_badge_opt = if !editor.spec.choices.is_empty() {
        Some(format!(
            "[Choices (Alt+C): {}]",
            editor.spec.choices.join(" | ")
        ))
    } else {
        None
    };

    let pinned_badge_opt = pinned_badge
        .filter(|b| !b.is_empty())
        .map(|b| format!("[Pinned (Ctrl+K): {}]", b));

    let avail_border_cols = pane_area.width.saturating_sub(4) as usize;
    let mut used_border_cols =
        UnicodeWidthStr::width(primary_label.as_str()) + UnicodeWidthStr::width(files_badge);

    let mut title_spans: Vec<Span<'static>> = vec![
        Span::styled(primary_label, title_style),
        Span::styled(files_badge.to_string(), badge_style),
    ];

    let mut secondary_subheader_items: Vec<String> = Vec::new();

    if let Some(choices_badge) = choices_badge_opt {
        let w = UnicodeWidthStr::width(choices_badge.as_str()) + 1;
        if used_border_cols + w <= avail_border_cols {
            title_spans.push(Span::styled(format!("{} ", choices_badge), badge_style));
            used_border_cols += w;
        } else {
            secondary_subheader_items.push(choices_badge);
        }
    }

    if let Some(pinned_str) = pinned_badge_opt {
        let w = UnicodeWidthStr::width(pinned_str.as_str()) + 1;
        if used_border_cols + w <= avail_border_cols && secondary_subheader_items.is_empty() {
            title_spans.push(Span::styled(format!("{} ", pinned_str), badge_style));
        } else {
            secondary_subheader_items.push(pinned_str);
        }
    }

    let borders = if pane_area.height <= 2 {
        Borders::TOP
    } else {
        Borders::ALL
    };

    let block = Block::default()
        .borders(borders)
        .border_style(border_style)
        .title(Line::from(title_spans))
        .style(surface_style);

    let mut inner = block.inner(pane_area);
    frame.render_widget(block, pane_area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    // Render dedicated secondary sub-header bar inside the top of the field box when options
    // or pinned constraints exceed a single header line so they layer/wrap cleanly instead of truncating.
    if !secondary_subheader_items.is_empty() && inner.height > 1 {
        let combined_sub = secondary_subheader_items.join(" ");
        let (wrapped_sub, _, _) =
            wrap_editor_lines_with_cursor(&[combined_sub], (0, 0), inner.width as usize);
        let sub_rows = (wrapped_sub.len() as u16).min(inner.height.saturating_sub(1)).max(1);
        let sub_rect = Rect::new(inner.x, inner.y, inner.width, sub_rows);
        let sub_lines: Vec<Line<'static>> = wrapped_sub
            .into_iter()
            .take(sub_rows as usize)
            .map(|s| {
                if let Some((prefix, rest)) = s.split_once(':') {
                    Line::from(vec![
                        Span::styled(format!("{}:", prefix), theme.badge_inactive_style()),
                        Span::styled(rest.to_string(), theme.badge_focused_style()),
                    ])
                } else {
                    Line::from(Span::styled(s, badge_style))
                }
            })
            .collect();
        frame.render_widget(Paragraph::new(sub_lines).style(surface_style), sub_rect);
        inner = Rect::new(
            inner.x,
            inner.y + sub_rows,
            inner.width,
            inner.height.saturating_sub(sub_rows),
        );
    }

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let raw_lines = editor.textarea.lines();
    let is_empty = raw_lines.is_empty() || (raw_lines.len() == 1 && raw_lines[0].is_empty());

    let cursor_style = Style::default()
        .fg(theme.bg_color())
        .bg(theme.highlight_color())
        .add_modifier(Modifier::BOLD);

    if is_empty {
        let placeholder_str = format!("<{}>", editor.spec.placeholder);
        let placeholder_vec = vec![placeholder_str];
        let (wrapped_ph, _, _) =
            wrap_editor_lines_with_cursor(&placeholder_vec, (0, 0), inner.width as usize);
        let ph_style = Style::default()
            .fg(theme.placeholder_color())
            .bg(theme.surface_color());

        let mut rendered_lines: Vec<Line> = Vec::with_capacity(inner.height as usize);
        for (row_i, line_str) in wrapped_ph.iter().take(inner.height as usize).enumerate() {
            if focused && row_i == 0 {
                let mut chars = line_str.chars();
                let first_ch = chars.next().unwrap_or(' ');
                let rest: String = chars.collect();
                rendered_lines.push(Line::from(vec![
                    Span::styled(first_ch.to_string(), cursor_style),
                    Span::styled(rest, ph_style),
                ]));
            } else {
                rendered_lines.push(Line::from(Span::styled(line_str.clone(), ph_style)));
            }
        }
        let para = Paragraph::new(rendered_lines).style(surface_style);
        frame.render_widget(para, inner);
        return;
    }

    let cursor = editor.textarea.cursor();
    let (visual_lines, v_cursor_row, v_cursor_col) =
        wrap_editor_lines_with_cursor(raw_lines, cursor, inner.width as usize);

    let view_height = inner.height as usize;
    let scroll_top = if v_cursor_row >= view_height {
        v_cursor_row + 1 - view_height
    } else {
        0
    };

    let mut rendered_lines: Vec<Line> = Vec::with_capacity(view_height);
    for (vis_idx, line_str) in visual_lines
        .iter()
        .enumerate()
        .skip(scroll_top)
        .take(view_height)
    {
        if focused && vis_idx == v_cursor_row {
            let chars: Vec<char> = line_str.chars().collect();
            let before: String = chars.iter().take(v_cursor_col).collect();
            let cursor_ch = chars.get(v_cursor_col).copied().unwrap_or(' ');
            let after: String = if v_cursor_col < chars.len() {
                chars.iter().skip(v_cursor_col + 1).collect()
            } else {
                String::new()
            };
            rendered_lines.push(Line::from(vec![
                Span::styled(before, surface_style),
                Span::styled(cursor_ch.to_string(), cursor_style),
                Span::styled(after, surface_style),
            ]));
        } else {
            rendered_lines.push(Line::from(Span::styled(line_str.clone(), surface_style)));
        }
    }

    let para = Paragraph::new(rendered_lines).style(surface_style);
    frame.render_widget(para, inner);
}

fn render_live_preview(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let profile = app.active_profile();
    let rendered = app.rendered_prompt();
    let metrics = app.prompt_metrics();

    let title = format!(
        " Live Prompt Preview [Ctrl+P] — {} ({}) | {}w / ~{}t ",
        profile.display_name,
        app.active_agy_model_id(),
        metrics.words,
        metrics.estimated_tokens
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(
            Style::default()
                .fg(theme.secondary_accent_color())
                .bg(theme.surface_color()),
        )
        .title(Span::styled(
            title,
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ))
        .style(theme.surface_style());

    let styled_lines: Vec<Line> = rendered
        .lines()
        .map(|line| {
            if line.starts_with('/') {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(theme.accent_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD),
                ))
            } else if line.starts_with('<') || line.starts_with("###") {
                Line::from(Span::styled(
                    line.to_string(),
                    Style::default()
                        .fg(theme.secondary_accent_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD),
                ))
            } else if let Some((prefix, rest)) = line.split_once(':') {
                if prefix.starts_with("- ") {
                    Line::from(vec![
                        Span::styled(
                            format!("{}:", prefix),
                            Style::default()
                                .fg(theme.tertiary_accent_color())
                                .bg(theme.surface_color())
                                .add_modifier(Modifier::BOLD),
                        ),
                        Span::styled(rest.to_string(), theme.surface_style()),
                    ])
                } else {
                    Line::from(Span::styled(line.to_string(), theme.surface_style()))
                }
            } else {
                Line::from(Span::styled(line.to_string(), theme.surface_style()))
            }
        })
        .collect();

    let paragraph = Paragraph::new(styled_lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(theme.surface_style());

    frame.render_widget(paragraph, area);
}

fn render_terminal_output_pane(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let is_focused = app.focus == FocusPane::TerminalOutput;
    let border_style = if is_focused {
        theme.border_focused_style()
    } else {
        theme.border_inactive_style()
    };
    let title_style = if is_focused {
        theme.title_focused_style()
    } else {
        theme.title_inactive_style()
    };

    let exit_badge = match app.terminal_output.last_exit_code {
        Some(0) => " [exit: 0 OK]",
        Some(code) => &format!(" [exit: {}]", code),
        None => "",
    };

    let title = format!(
        " Terminal Execution Output Pane [Ctrl+O] — {} lines{} (Up/Down/PgUp/PgDn: Scroll | Ctrl+L: Clear) ",
        app.terminal_output.lines.len(),
        exit_badge
    );

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(border_style)
        .title(Span::styled(title, title_style))
        .style(theme.surface_style());

    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.width == 0 || inner.height == 0 {
        return;
    }

    let view_h = inner.height as usize;
    let total = app.terminal_output.lines.len();
    let start_idx = if total <= view_h {
        0
    } else {
        app.terminal_output
            .scroll_offset
            .min(total.saturating_sub(view_h))
    };

    let visible_lines: Vec<Line> = app
        .terminal_output
        .lines
        .iter()
        .skip(start_idx)
        .take(view_h)
        .map(|l| {
            if l.starts_with("$ ") || l.starts_with("==> ") {
                Line::from(Span::styled(
                    l.clone(),
                    Style::default()
                        .fg(theme.accent_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD),
                ))
            } else if l.starts_with("[stderr]") || l.starts_with("[Error") {
                Line::from(Span::styled(
                    l.clone(),
                    Style::default()
                        .fg(theme.secondary_accent_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD),
                ))
            } else {
                Line::from(Span::styled(l.clone(), theme.surface_style()))
            }
        })
        .collect();

    let para = Paragraph::new(visible_lines)
        .wrap(Wrap { trim: false })
        .style(theme.surface_style());
    frame.render_widget(para, inner);
}

fn render_terminal_output_modal(frame: &mut Frame, root_area: Rect, app: &App) {
    let popup_area = centered_rect(88, 70, root_area);
    frame.render_widget(Clear, popup_area);
    render_terminal_output_pane(frame, popup_area, app);
}

/// Builds width-aware wrapped [`Line`]s for the selectable Action Bar (`1..4`) so that no
/// action option is ever cut off at any terminal width `>= 75`.
pub fn build_wrapped_action_lines(app: &App, avail_width: u16) -> Vec<Line<'static>> {
    let theme = &app.theme;
    let max_cols = (avail_width as usize).max(40);
    let prefix_first = if avail_width < 110 {
        " Actions: "
    } else {
        " Actions (Alt+1..4 / Enter): "
    };
    let prefix_cont = "          ";

    let prefix_style = Style::default()
        .fg(theme.secondary_accent_color())
        .bg(theme.surface_color())
        .add_modifier(Modifier::BOLD);

    let mut lines: Vec<Line<'static>> = Vec::new();
    let mut current_spans: Vec<Span<'static>> =
        vec![Span::styled(prefix_first.to_string(), prefix_style)];
    let mut current_w = UnicodeWidthStr::width(prefix_first);

    for act in DispatchAction::ALL {
        let is_selected = act == app.selected_action;
        let btn_text = if avail_width < 110 {
            format!("[{}]", act.label())
        } else {
            format!(" [{}] ", act.label())
        };
        let btn_w = UnicodeWidthStr::width(btn_text.as_str());
        let has_btns = current_spans.len() > 1;

        if has_btns && current_w + 1 + btn_w > max_cols {
            lines.push(Line::from(current_spans));
            current_spans = vec![Span::styled(prefix_cont.to_string(), prefix_style)];
            current_w = UnicodeWidthStr::width(prefix_cont);
        } else if has_btns {
            current_spans.push(Span::styled(" ", theme.surface_style()));
            current_w += 1;
        }

        let style = if is_selected {
            Style::default()
                .fg(theme.bg_color())
                .bg(theme.accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.text_color())
                .bg(theme.surface_color())
        };
        current_spans.push(Span::styled(btn_text, style));
        current_w += btn_w;
    }

    if current_spans.len() > 1 {
        lines.push(Line::from(current_spans));
    }

    lines
}

fn render_footer(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let is_action_focused = app.focus == FocusPane::ActionBar;
    let border_style = if is_action_focused {
        theme.border_focused_style()
    } else {
        theme.border_inactive_style()
    };

    let action_lines_no_border = build_wrapped_action_lines(app, area.width);
    let action_lines_with_border = build_wrapped_action_lines(app, area.width.saturating_sub(2));
    let min_content_rows_with_border = 1 + action_lines_with_border.len() as u16;
    let min_content_rows_no_border = 1 + action_lines_no_border.len() as u16;

    let (borders, avail_w, action_lines) =
        if area.height >= min_content_rows_with_border + 2 {
            (
                Borders::ALL,
                area.width.saturating_sub(2),
                action_lines_with_border,
            )
        } else if area.height >= min_content_rows_no_border + 1 {
            (Borders::TOP, area.width, action_lines_no_border)
        } else {
            (Borders::NONE, area.width, action_lines_no_border)
        };

    let metrics = app.prompt_metrics();
    let agy_cmd_full = app.agy_command_preview();
    let stats_str = format!(" │ {}w / ~{}t", metrics.words, metrics.estimated_tokens);
    let max_cmd_chars = (avail_w as usize)
        .saturating_sub(10 + UnicodeWidthStr::width(stats_str.as_str()))
        .max(28);
    let agy_cmd_display = if agy_cmd_full.chars().count() > max_cmd_chars {
        let prefix: String = agy_cmd_full
            .chars()
            .take(max_cmd_chars.saturating_sub(3))
            .collect();
        format!("{}...", prefix)
    } else {
        agy_cmd_full
    };

    let cmd_line = Line::from(vec![
        Span::styled(
            " agy CLI: ",
            Style::default()
                .fg(theme.secondary_accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            agy_cmd_display,
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color()),
        ),
        Span::styled(
            stats_str,
            Style::default()
                .fg(theme.tertiary_accent_color())
                .bg(theme.surface_color()),
        ),
    ]);

    let status_line = Line::from(vec![
        Span::styled(
            " Status: ",
            Style::default()
                .fg(theme.accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(app.status_message.clone(), theme.surface_style()),
    ]);

    let mut footer_lines = vec![cmd_line];
    footer_lines.extend(action_lines);
    let inner_rows = match borders {
        Borders::ALL => area.height.saturating_sub(2) as usize,
        Borders::TOP => area.height.saturating_sub(1) as usize,
        _ => area.height as usize,
    };
    if footer_lines.len() < inner_rows {
        footer_lines.push(status_line);
    }

    if borders == Borders::NONE {
        let footer_para = Paragraph::new(footer_lines).style(theme.surface_style());
        frame.render_widget(footer_para, area);
        return;
    }

    let block = Block::default()
        .borders(borders)
        .border_style(border_style)
        .title(Span::styled(
            " Dispatch & Status Footer (Ctrl+Enter: Run Action | Ctrl+O: Output Pane | Ctrl+S: Save .md | Ctrl+P: Preview | Ctrl+Q: Quit) ",
            if is_action_focused {
                theme.title_focused_style()
            } else {
                theme.title_inactive_style()
            },
        ))
        .style(theme.surface_style());

    let footer_para = Paragraph::new(footer_lines)
        .block(block)
        .style(theme.surface_style());
    frame.render_widget(footer_para, area);
}

fn render_plan_picker_approval_banner(frame: &mut Frame, root_area: Rect, app: &App) {
    let theme = &app.theme;
    let popup_area = centered_rect(76, 22, root_area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused_style())
        .title(Span::styled(
            " Confirm Opening .md Plan File Picker ",
            theme.title_focused_style(),
        ))
        .style(theme.surface_style());

    let lines = vec![
        Line::from(Span::styled(
            " Choice 'Update <file>.md plan' selected.",
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            " Approve opening .md file picker? [Enter / Alt+Y: Open Picker | Esc / Alt+N: Keep literal]",
            Style::default()
                .fg(theme.accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        )),
    ];

    let para = Paragraph::new(lines)
        .block(block)
        .wrap(Wrap { trim: false })
        .style(theme.surface_style());
    frame.render_widget(para, popup_area);
}

fn render_model_manager_modal(frame: &mut Frame, root_area: Rect, app: &App) {
    let theme = &app.theme;
    let popup_area = centered_rect(78, 72, root_area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused_style())
        .title(Span::styled(
            " Model Catalog Sync & Add Custom Sub-Model (Alt+U) ",
            theme.title_focused_style(),
        ))
        .style(theme.surface_style());

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let modal = &app.model_modal;
    let mut lines: Vec<Line> = Vec::new();

    lines.push(Line::from(vec![
        Span::styled(
            " [Ctrl+R / Alt+R] ",
            Style::default()
                .fg(theme.bg_color())
                .bg(theme.accent_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!(
                " Sync Available Models from `agy models` (Active Catalog: {} profiles)",
                app.catalog.profiles.len()
            ),
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        " ────────────────────────────────────────────────────────────────────────",
        Style::default()
            .fg(theme.border_color())
            .bg(theme.surface_color()),
    )));

    let field_values = [
        (
            ModelModalField::Family,
            format!("◄ {} ►", modal.family.as_str()),
        ),
        (ModelModalField::ShortName, modal.short_name.clone()),
        (ModelModalField::DisplayName, modal.display_name.clone()),
        (ModelModalField::AgyBaseId, modal.agy_base_id.clone()),
        (
            ModelModalField::DefaultEffort,
            format!("◄ {} ►", modal.default_effort.as_str()),
        ),
    ];

    for (field_kind, val) in field_values {
        let is_focused = modal.focused_field == field_kind;
        let prefix = if is_focused { " ▸ " } else { "   " };
        let label_style = if is_focused {
            Style::default()
                .fg(theme.accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.secondary_accent_color())
                .bg(theme.surface_color())
        };
        let val_display = if is_focused
            && matches!(
                field_kind,
                ModelModalField::ShortName
                    | ModelModalField::DisplayName
                    | ModelModalField::AgyBaseId
            ) {
            format!("{}█", val)
        } else if val.is_empty() {
            "<auto-derived if blank>".to_string()
        } else {
            val
        };
        let val_style = if is_focused {
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.text_color())
                .bg(theme.surface_color())
        };

        lines.push(Line::from(vec![
            Span::styled(format!("{}{:<34}: ", prefix, field_kind.label()), label_style),
            Span::styled(val_display, val_style),
        ]));
    }

    lines.push(Line::from(Span::styled(
        " ────────────────────────────────────────────────────────────────────────",
        Style::default()
            .fg(theme.border_color())
            .bg(theme.surface_color()),
    )));
    lines.push(Line::from(vec![
        Span::styled(
            " Status: ",
            Style::default()
                .fg(theme.accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(modal.status_note.clone(), theme.surface_style()),
    ]));

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(theme.surface_style());
    frame.render_widget(para, inner);
}

fn render_shell_quote_modal(frame: &mut Frame, root_area: Rect, app: &App) {
    let theme = &app.theme;
    let popup_area = centered_rect(82, 56, root_area);
    frame.render_widget(Clear, popup_area);

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused_style())
        .title(Span::styled(
            " Shell Metacharacter & Quote Confirmation (PromptOnSpecialChars) ",
            theme.title_focused_style(),
        ))
        .style(theme.surface_style());

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let modal = &app.shell_quote_modal;
    let mut lines: Vec<Line> = Vec::new();
    lines.push(Line::from(vec![
        Span::styled(
            " Warning: ",
            Style::default()
                .fg(theme.accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            modal.safety_report.summary(),
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        " Unescaped shell characters (*, (), \"\", '', $, `, newlines) can break terminal command parsing.",
        theme.surface_style(),
    )));
    lines.push(Line::from(Span::styled(
        " ────────────────────────────────────────────────────────────────────────────",
        Style::default()
            .fg(theme.border_color())
            .bg(theme.surface_color()),
    )));
    lines.push(Line::from(vec![
        Span::styled(
            " POSIX-Safe Command Preview: ",
            Style::default()
                .fg(theme.secondary_accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            modal.shell_safe_preview.lines().next().unwrap_or(""),
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color()),
        ),
    ]));
    lines.push(Line::from(Span::styled(
        " ────────────────────────────────────────────────────────────────────────────",
        Style::default()
            .fg(theme.border_color())
            .bg(theme.surface_color()),
    )));
    lines.push(Line::from(Span::styled(
        " [Enter / Alt+E: Auto-Escape & Quote for Terminal Execution (Recommended)] ",
        Style::default()
            .fg(theme.bg_color())
            .bg(theme.accent_color())
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        " [Alt+A: Always Auto-Escape (Save to Settings)]",
        Style::default()
            .fg(theme.secondary_accent_color())
            .bg(theme.surface_color())
            .add_modifier(Modifier::BOLD),
    )));
    lines.push(Line::from(Span::styled(
        " [Alt+R / Esc: Edit / Add Quotes Manually]",
        Style::default()
            .fg(theme.tertiary_accent_color())
            .bg(theme.surface_color()),
    )));

    let para = Paragraph::new(lines)
        .wrap(Wrap { trim: false })
        .style(theme.surface_style());
    frame.render_widget(para, inner);
}

fn render_settings_page(frame: &mut Frame, area: Rect, app: &App) {
    let theme = &app.theme;
    let header_rows = if area.height >= 38 {
        9u16
    } else if area.height >= 32 {
        5u16
    } else {
        1u16
    };

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(header_rows),
            Constraint::Length(3),
            Constraint::Min(14),
            Constraint::Length(3),
        ])
        .split(area);

    render_header_banner(frame, chunks[0], app);

    // Tab bar across the 4 Settings sections
    let mut tab_spans: Vec<Span> = vec![Span::styled(" ", theme.surface_style())];
    let compact_tabs = area.width < 105;
    for tab in SettingsTab::ALL {
        let is_active = app.settings_page.active_tab == tab;
        let label = if compact_tabs {
            format!(" {} ", tab.short_title())
        } else {
            format!(" {} ", tab.title())
        };
        let style = if is_active {
            Style::default()
                .fg(theme.bg_color())
                .bg(theme.accent_color())
                .add_modifier(Modifier::BOLD)
        } else {
            Style::default()
                .fg(theme.secondary_accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD)
        };
        tab_spans.push(Span::styled(label, style));
        tab_spans.push(Span::styled(
            " │ ",
            Style::default()
                .fg(theme.border_color())
                .bg(theme.surface_color()),
        ));
    }

    let tab_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused_style())
        .title(Span::styled(
            " VERBOCITY SETTINGS PAGE (Alt+S) — [1..4 / Ctrl+Left/Right: Switch Tab | Esc/Alt+S: Return to Prompt] ",
            theme.title_focused_style(),
        ))
        .style(theme.surface_style());
    frame.render_widget(
        Paragraph::new(Line::from(tab_spans))
            .block(tab_block)
            .style(theme.surface_style()),
        chunks[1],
    );

    // Active Tab Body
    let body_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_inactive_style())
        .title(Span::styled(
            format!(
                " {} — Up/Down/Tab: Navigate | Left/Right/Enter: Modify | Ctrl+S: Save ",
                app.settings_page.active_tab.title()
            ),
            theme.title_inactive_style(),
        ))
        .style(theme.surface_style());
    let body_inner = body_block.inner(chunks[2]);
    frame.render_widget(body_block, chunks[2]);

    let mut lines: Vec<Line> = Vec::new();
    let sel_row = app.settings_page.selected_row;

    match app.settings_page.active_tab {
        SettingsTab::ThemeAndContrast => {
            let rows: [(usize, &str, String, Option<RgbColor>); 8] = [
                (
                    0,
                    "Active Theme Preset (Left/Right)",
                    format!("◄ {} ►", theme.name),
                    None,
                ),
                (
                    1,
                    "Primary Accent (accent_primary)",
                    app.settings_page.hex_accent_primary.clone(),
                    RgbColor::from_hex(&app.settings_page.hex_accent_primary).ok(),
                ),
                (
                    2,
                    "Secondary Accent (accent_secondary)",
                    app.settings_page.hex_accent_secondary.clone(),
                    RgbColor::from_hex(&app.settings_page.hex_accent_secondary).ok(),
                ),
                (
                    3,
                    "Primary Text (text_primary)",
                    app.settings_page.hex_text_primary.clone(),
                    RgbColor::from_hex(&app.settings_page.hex_text_primary).ok(),
                ),
                (
                    4,
                    "Unfocused Field Header (title_inactive)",
                    app.settings_page.hex_title_inactive.clone(),
                    RgbColor::from_hex(&app.settings_page.hex_title_inactive).ok(),
                ),
                (
                    5,
                    "Inactive Border Slate (border_slate)",
                    app.settings_page.hex_border_slate.clone(),
                    RgbColor::from_hex(&app.settings_page.hex_border_slate).ok(),
                ),
                (
                    6,
                    "Muted Placeholder (placeholder_muted)",
                    app.settings_page.hex_placeholder_muted.clone(),
                    RgbColor::from_hex(&app.settings_page.hex_placeholder_muted).ok(),
                ),
                (
                    7,
                    "Persist Theme Configuration",
                    "[Save Theme & Color Overrides to ~/.config/verbot/theme.toml (Enter / Ctrl+S)]"
                        .to_string(),
                    None,
                ),
            ];
            for (idx, label, val, swatch) in rows {
                let focused = sel_row == idx;
                let prefix = if focused { " ▸ " } else { "   " };
                let label_style = if focused {
                    Style::default()
                        .fg(theme.accent_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme.secondary_accent_color())
                        .bg(theme.surface_color())
                };
                let val_display = if focused && (1..=6).contains(&idx) {
                    format!("{}█", val)
                } else {
                    val
                };
                let val_style = if focused {
                    Style::default()
                        .fg(theme.highlight_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.surface_style()
                };
                let mut row_spans = vec![
                    Span::styled(format!("{}{:<42}: ", prefix, label), label_style),
                    Span::styled(val_display, val_style),
                ];
                if let Some(rgb) = swatch {
                    row_spans.push(Span::styled("  ", theme.surface_style()));
                    row_spans.push(Span::styled(
                        " ██ ",
                        Style::default().fg(rgb.to_ratatui()).bg(theme.surface_color()),
                    ));
                }
                lines.push(Line::from(row_spans));
            }
        }
        SettingsTab::ModelsAndToml => {
            let ingest_val = if app.settings_page.model_ingest_path.is_empty() {
                "<path to model .toml/.yaml or press Ctrl+I / Enter to browse>".to_string()
            } else {
                app.settings_page.model_ingest_path.clone()
            };
            let rows: [(usize, &str, String); 9] = [
                (
                    0,
                    "Ingest Model .toml / .yaml File (Ctrl+I)",
                    ingest_val,
                ),
                (
                    1,
                    "Sync from `agy models` CLI (Ctrl+R)",
                    format!(
                        "[Run `agy models` Discovery & Merge ({} active profiles)]",
                        app.catalog.profiles.len()
                    ),
                ),
                (
                    2,
                    "Custom Model Family (Left/Right)",
                    format!("◄ {} ►", app.settings_page.custom_family.as_str()),
                ),
                (
                    3,
                    "Custom Short Name",
                    app.settings_page.custom_short_name.clone(),
                ),
                (
                    4,
                    "Custom Display Name",
                    app.settings_page.custom_display_name.clone(),
                ),
                (
                    5,
                    "Custom Base agy ID",
                    app.settings_page.custom_agy_base_id.clone(),
                ),
                (
                    6,
                    "Available Tiers (comma-separated)",
                    app.settings_page.custom_available_tiers.clone(),
                ),
                (
                    7,
                    "Default Effort Level (Left/Right)",
                    format!("◄ {} ►", app.settings_page.custom_default_effort.as_str()),
                ),
                (
                    8,
                    "Save Custom Sub-Model",
                    "[Add / Update Sub-Model & Persist to ~/.config/verbot/models.toml (Enter)]"
                        .to_string(),
                ),
            ];
            for (idx, label, val) in rows {
                let focused = sel_row == idx;
                let prefix = if focused { " ▸ " } else { "   " };
                let label_style = if focused {
                    Style::default()
                        .fg(theme.accent_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme.secondary_accent_color())
                        .bg(theme.surface_color())
                };
                let is_text = matches!(idx, 0 | 3 | 4 | 5 | 6);
                let val_display = if focused && is_text {
                    format!("{}█", val)
                } else if val.is_empty() {
                    "<auto-derived if blank>".to_string()
                } else {
                    val
                };
                let val_style = if focused {
                    Style::default()
                        .fg(theme.highlight_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.surface_style()
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{}{:<42}: ", prefix, label), label_style),
                    Span::styled(val_display, val_style),
                ]));
            }
        }
        SettingsTab::TemplatesAndDefaults => {
            let tmpl_ingest_val = if app.settings_page.template_ingest_path.is_empty() {
                "<path to template .md/.toml or press Ctrl+T / Enter to browse>".to_string()
            } else {
                app.settings_page.template_ingest_path.clone()
            };
            let dest_label = if app.settings_page.template_ingest_local {
                "◄ Project-Local (./.verbot/templates/) ►"
            } else {
                "◄ Global (~/.config/verbot/templates/) ►"
            };
            let def_cmd = app
                .global_config
                .default_slash_command
                .as_deref()
                .map(|c| format!("/{}", c.trim_start_matches('/')))
                .unwrap_or_else(|| "(none)".to_string());
            let rows: [(usize, &str, String); 8] = [
                (
                    0,
                    "Active Prompt Template (Left/Right)",
                    format!("◄ {} ►", app.template.name),
                ),
                (
                    1,
                    "Ingest Template .md / .toml (Ctrl+T)",
                    tmpl_ingest_val,
                ),
                (
                    2,
                    "Template Ingest Destination",
                    dest_label.to_string(),
                ),
                (
                    3,
                    "Startup Default Model Family",
                    format!("◄ {} ►", app.global_config.default_model_family),
                ),
                (
                    4,
                    "Startup Default Sub-Model",
                    format!("◄ {} ►", app.global_config.default_sub_model),
                ),
                (
                    5,
                    "Startup Default Effort Level",
                    format!("◄ {} ►", app.global_config.default_effort),
                ),
                (
                    6,
                    "Startup Default Slash Command",
                    format!("◄ {} ►", def_cmd),
                ),
                (
                    7,
                    "Show Live Preview on Startup",
                    format!("◄ {} ► (Saved to config.toml)", app.global_config.show_preview_on_startup),
                ),
            ];
            for (idx, label, val) in rows {
                let focused = sel_row == idx;
                let prefix = if focused { " ▸ " } else { "   " };
                let label_style = if focused {
                    Style::default()
                        .fg(theme.accent_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme.secondary_accent_color())
                        .bg(theme.surface_color())
                };
                let val_display = if focused && idx == 1 {
                    format!("{}█", val)
                } else {
                    val
                };
                let val_style = if focused {
                    Style::default()
                        .fg(theme.highlight_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.surface_style()
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{}{:<42}: ", prefix, label), label_style),
                    Span::styled(val_display, val_style),
                ]));
            }
        }
        SettingsTab::ExecutionAndWorkspace => {
            let flags = &app.global_config.agy_flags;
            let storage_mode_str = if app.force_local_state || app.global_config.workspace_storage_local {
                "◄ Project-Local (./.verbot/state.toml) ►"
            } else {
                "◄ XDG Data Cache (~/.local/share/verbot/workspaces/) ►"
            };
            let rows: [(usize, &str, String); 9] = [
                (
                    0,
                    "Default agy --mode Flag (Left/Right)",
                    format!("◄ {} ►", flags.mode.as_str()),
                ),
                (
                    1,
                    "Default agy --sandbox Flag",
                    format!("◄ {} ►", flags.sandbox),
                ),
                (
                    2,
                    "Default agy --continue Flag",
                    format!("◄ {} ►", flags.continue_session),
                ),
                (
                    3,
                    "Default agy --dangerously-skip-permissions",
                    format!("◄ {} ►", flags.dangerously_skip_permissions),
                ),
                (
                    4,
                    "Custom agy --add-dir Paths (comma-sep)",
                    app.settings_page.add_dirs_input.clone(),
                ),
                (
                    5,
                    "Shell Text Formatting Policy",
                    format!("◄ {} ►", app.global_config.shell_formatting_policy.label()),
                ),
                (
                    6,
                    "Workspace Storage Mode",
                    storage_mode_str.to_string(),
                ),
                (
                    7,
                    "Workspace Default Plan .md Path",
                    app.settings_page.default_plan_path_input.clone(),
                ),
                (
                    8,
                    "Workspace Pinned Constraints (;-sep)",
                    app.settings_page.pinned_constraints_input.clone(),
                ),
            ];
            for (idx, label, val) in rows {
                let focused = sel_row == idx;
                let prefix = if focused { " ▸ " } else { "   " };
                let label_style = if focused {
                    Style::default()
                        .fg(theme.accent_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    Style::default()
                        .fg(theme.secondary_accent_color())
                        .bg(theme.surface_color())
                };
                let is_text = matches!(idx, 4 | 7 | 8);
                let val_display = if focused && is_text {
                    format!("{}█", val)
                } else if val.is_empty() {
                    "(none)".to_string()
                } else {
                    val
                };
                let val_style = if focused {
                    Style::default()
                        .fg(theme.highlight_color())
                        .bg(theme.surface_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.surface_style()
                };
                lines.push(Line::from(vec![
                    Span::styled(format!("{}{:<42}: ", prefix, label), label_style),
                    Span::styled(val_display, val_style),
                ]));
            }
        }
    }

    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .style(theme.surface_style()),
        body_inner,
    );

    // Settings Footer
    let footer_block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_inactive_style())
        .title(Span::styled(
            " Settings Status & Keybindings (Esc / Alt+S: Return to Main Prompt Page with all text preserved) ",
            theme.title_inactive_style(),
        ))
        .style(theme.surface_style());
    let status_line = Line::from(vec![
        Span::styled(
            " Status: ",
            Style::default()
                .fg(theme.accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(app.settings_page.status_note.clone(), theme.surface_style()),
    ]);
    frame.render_widget(
        Paragraph::new(status_line)
            .block(footer_block)
            .style(theme.surface_style()),
        chunks[3],
    );
}

fn render_file_picker_modal(frame: &mut Frame, root_area: Rect, app: &App) {
    let theme = &app.theme;
    let popup_area = centered_rect(72, 65, root_area);

    frame.render_widget(Clear, popup_area);

    let mode_label = if app.file_picker.markdown_only {
        "Markdown Plan Files (.md) [Ctrl+F: All Files]"
    } else {
        "All Workspace Files [Ctrl+F: .md Only]"
    };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(theme.border_focused_style())
        .title(Span::styled(
            format!(
                " CWD Fuzzy File Picker (@ / Ctrl+F) — {} ",
                mode_label
            ),
            theme.title_focused_style(),
        ))
        .style(theme.surface_style());

    let inner = block.inner(popup_area);
    frame.render_widget(block, popup_area);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(2), Constraint::Min(3)])
        .split(inner);

    let search_line = Line::from(vec![
        Span::styled(
            " Search Query: ",
            Style::default()
                .fg(theme.accent_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
        Span::styled(
            format!("{}█", app.file_picker.query),
            Style::default()
                .fg(theme.highlight_color())
                .bg(theme.surface_color())
                .add_modifier(Modifier::BOLD),
        ),
    ]);
    frame.render_widget(
        Paragraph::new(search_line).style(theme.surface_style()),
        chunks[0],
    );

    let matches = app.file_picker.matches(&app.file_index, 30);
    let items: Vec<ListItem> = if matches.is_empty() {
        vec![ListItem::new(Line::from(Span::styled(
            "  (No matching files in current working directory)",
            Style::default()
                .fg(theme.border_color())
                .bg(theme.surface_color()),
        )))]
    } else {
        matches
            .iter()
            .enumerate()
            .map(|(idx, file)| {
                let is_sel = idx == app.file_picker.selected_index;
                let prefix = if is_sel { " ▸ " } else { "   " };
                let badge = if file.is_plan_candidate {
                    " [PLAN]"
                } else if file.is_markdown {
                    " [.md]"
                } else {
                    ""
                };
                let style = if is_sel {
                    Style::default()
                        .fg(theme.bg_color())
                        .bg(theme.accent_color())
                        .add_modifier(Modifier::BOLD)
                } else {
                    theme.surface_style()
                };
                ListItem::new(Line::from(Span::styled(
                    format!("{}{}{}", prefix, file.relative_path, badge),
                    style,
                )))
            })
            .collect()
    };

    let list = List::new(items).style(theme.surface_style());
    frame.render_widget(list, chunks[1]);
}

fn centered_rect(percent_x: u16, percent_y: u16, r: Rect) -> Rect {
    let popup_layout = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Percentage((100 - percent_y) / 2),
            Constraint::Percentage(percent_y),
            Constraint::Percentage((100 - percent_y) / 2),
        ])
        .split(r);

    Layout::default()
        .direction(Direction::Horizontal)
        .constraints([
            Constraint::Percentage((100 - percent_x) / 2),
            Constraint::Percentage(percent_x),
            Constraint::Percentage((100 - percent_x) / 2),
        ])
        .split(popup_layout[1])[1]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::files::WorkspaceFileIndex;
    use crate::models::ModelCatalog;
    use crate::state::WorkspaceState;
    use crate::template::PromptTemplate;
    use crate::theme::{novos_palette, Theme, ThemePreset};
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use ratatui::style::Color;
    use std::path::PathBuf;

    fn build_test_app() -> App {
        static TEST_COUNTER: std::sync::atomic::AtomicUsize =
            std::sync::atomic::AtomicUsize::new(0);
        let seq = TEST_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let tmp_xdg = std::env::temp_dir().join(format!(
            "verbot_tui_unit_test_{}_{}",
            std::process::id(),
            seq
        ));
        std::fs::create_dir_all(&tmp_xdg).unwrap();
        let (ws_state, loc) =
            WorkspaceState::load_for_dir(&repo_root, Some(&tmp_xdg), None).unwrap();
        let template = PromptTemplate::default();
        let catalog = ModelCatalog::builtin();
        let theme = Theme::novos_cosmic_dark();
        let file_index = WorkspaceFileIndex::scan(&repo_root).unwrap();

        let mut app = App::new(
            repo_root,
            template,
            catalog,
            ws_state,
            loc,
            theme,
            file_index,
            false,
        );
        app.custom_xdg_data_dir = Some(tmp_xdg.clone());
        app.custom_theme_toml_path = Some(tmp_xdg.join("theme.toml"));
        app
    }

    #[test]
    fn test_headless_render_branding_novos_colors_and_live_switching() {
        let mut app = build_test_app();
        app.editors[0].set_text("Execute Step 3 multi-pane Ratatui interface");
        app.editors[1].set_text("STEPPED_IMPLEMENTATION_PLAN.md, src/app.rs, src/tui.rs");
        app.editors[2].set_text("Preserve field text across model and theme switches");
        app.editors[3].set_text("Verify with cargo test and --self-test");
        app.sync_values_from_editors();

        // 1. Render in default NovOS Cosmic Dark theme on 150x45 TestBackend
        let buf = render_headless_buffer(&mut app, 150, 45).unwrap();
        let rendered_screen = buffer_to_string(&buf);

        assert!(
            rendered_screen.contains("VERBOCITY"),
            "TUI screen must contain VERBOCITY header"
        );
        assert!(
            rendered_screen.contains("◁((│ ◉   ◉ │))▷") && rendered_screen.contains("(( [◉_◉] ))"),
            "TUI screen must contain speaker-eared boxy robot mascot"
        );
        assert!(
            rendered_screen.contains("Execute Step 3 multi-pane Ratatui interface"),
            "TUI screen must display Objective editor text"
        );
        assert!(
            rendered_screen.contains("agy -i --model gemini-4-argon-medium"),
            "TUI footer must display agy command preview"
        );

        // Verify exact NovOS Cosmic Nova Orange & Deep Obsidian RGB cells exist in the rendered buffer
        let mut has_surface_rgb = false;
        let mut has_nova_orange_rgb = false;
        let mut has_border_slate_rgb = false;
        for y in 0..45 {
            for x in 0..150 {
                let cell = &buf[(x, y)];
                if cell.bg == novos_palette::COSMIC_SPACE_SURFACE.to_ratatui() {
                    has_surface_rgb = true;
                }
                if cell.fg == novos_palette::COSMIC_NOVA_ORANGE.to_ratatui()
                    || cell.bg == novos_palette::COSMIC_NOVA_ORANGE.to_ratatui()
                {
                    has_nova_orange_rgb = true;
                }
                if cell.fg == novos_palette::ELEVATED_BORDER_SLATE.to_ratatui() {
                    has_border_slate_rgb = true;
                }
            }
        }
        assert!(has_surface_rgb, "Missing #121521 Cosmic Space Surface background in buffer");
        assert!(has_nova_orange_rgb, "Missing #FF6B1A Cosmic Nova Orange accent in buffer");
        assert!(has_border_slate_rgb, "Missing #2A314A Elevated Border Slate in buffer");

        // 2. Live Theme Switching (`Ctrl+T`) preserves all field text
        app.handle_key_event(KeyEvent::new(KeyCode::Char('t'), KeyModifiers::CONTROL))
            .unwrap();
        assert_eq!(app.theme.preset, ThemePreset::CosmicSystemAuto);
        assert_eq!(
            app.values.objective,
            "Execute Step 3 multi-pane Ratatui interface"
        );

        // Cycle to Terminal ANSI and verify Color::Reset passthrough
        app.set_theme_preset(ThemePreset::TerminalAnsi);
        let ansi_buf = render_headless_buffer(&mut app, 150, 45).unwrap();
        assert_eq!(ansi_buf[(1, 1)].bg, Color::Reset);

        // Switch back to NovOS Cosmic Dark
        app.set_theme_preset(ThemePreset::NovosCosmicDark);

        // 3. Model switching (`Alt+M` / `select_model_by_query`) preserves field text and updates preview + agy footer
        app.select_model_by_query("Opus");
        let claude_buf = render_headless_buffer(&mut app, 150, 45).unwrap();
        let claude_screen = buffer_to_string(&claude_buf);
        assert!(claude_screen.contains("<context>"));
        assert!(claude_screen.contains("claude-opus-5-5-medium"));
        assert_eq!(
            app.values.objective,
            "Execute Step 3 multi-pane Ratatui interface"
        );
        assert_eq!(
            app.values.constraints,
            "Preserve field text across model and theme switches"
        );

        app.select_model_by_query("GPT-OSS 120B");
        let gpt_buf = render_headless_buffer(&mut app, 150, 45).unwrap();
        let gpt_screen = buffer_to_string(&gpt_buf);
        assert!(gpt_screen.contains("### Context / Logs"));
        assert!(gpt_screen.contains("gpt-oss-120b-medium"));
        assert_eq!(
            app.values.objective,
            "Execute Step 3 multi-pane Ratatui interface"
        );

        if let Some(ref tmp) = app.custom_xdg_data_dir {
            let _ = std::fs::remove_dir_all(tmp);
        }
    }

    #[test]
    fn test_fuzzy_file_picker_modal_and_dispatch_manifest() {
        let mut app = build_test_app();
        // Focus Context/Logs field (index 1)
        app.focus = FocusPane::FieldEditor(1);
        app.editors[1].set_text("");
        app.sync_values_from_editors();

        // Trigger file picker via `@`
        app.handle_key_event(KeyEvent::new(KeyCode::Char('@'), KeyModifiers::NONE))
            .unwrap();
        assert!(app.file_picker.is_open);

        // Type query "state.rs"
        for ch in "state.rs".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .unwrap();
        }
        let modal_buf = render_headless_buffer(&mut app, 120, 40).unwrap();
        let modal_screen = buffer_to_string(&modal_buf);
        assert!(modal_screen.contains("CWD Fuzzy File Picker"));
        assert!(modal_screen.contains("src/state.rs"));

        // Confirm selection with Enter
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(!app.file_picker.is_open);
        assert_eq!(app.values.context_logs, "src/state.rs");

        // Test Output field quick-choice `Update <file>.md plan` staging approval before opening markdown file picker
        app.focus = FocusPane::FieldEditor(3);
        // Choice 0 = Diagnose only, Choice 1 = Update <file>.md plan
        app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT))
            .unwrap();
        assert_eq!(app.values.output, "Diagnose only");
        app.handle_key_event(KeyEvent::new(KeyCode::Char('c'), KeyModifiers::ALT))
            .unwrap();
        assert!(
            app.pending_plan_picker_approval.is_some() && !app.file_picker.is_open,
            "Selecting 'Update <file>.md plan' must stage pending approval before opening file picker"
        );

        // Approve opening the .md file picker with Enter
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(app.file_picker.is_open);
        assert!(app.file_picker.markdown_only);

        // Confirm top plan file (`STEPPED_IMPLEMENTATION_PLAN.md`)
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(
            app.values.output,
            "Update STEPPED_IMPLEMENTATION_PLAN.md plan"
        );

        // Confirm dispatch action 1 (`agy -i`)
        let manifest = app
            .confirm_dispatch(DispatchAction::AgyInteractive)
            .unwrap();
        assert_eq!(manifest.action, "agy-interactive");
        assert!(manifest.rendered_prompt.contains("src/state.rs"));
        assert!(manifest
            .rendered_prompt
            .contains("Update STEPPED_IMPLEMENTATION_PLAN.md plan"));

        if let Some(ref tmp) = app.custom_xdg_data_dir {
            let _ = std::fs::remove_dir_all(tmp);
        }
    }

    #[test]
    fn test_step5_placeholder_contrast_soft_wrap_model_modal_and_compact_guard() {
        let mut app = build_test_app();

        // 1. Verify #5A6588 placeholder color is rendered in empty editor cells
        let empty_buf = render_headless_buffer(&mut app, 120, 40).unwrap();
        let mut found_placeholder_rgb = false;
        for y in 0..40 {
            for x in 0..120 {
                if empty_buf[(x, y)].fg == novos_palette::PLACEHOLDER_SLATE_MUTED.to_ratatui() {
                    found_placeholder_rgb = true;
                    break;
                }
            }
        }
        assert!(
            found_placeholder_rgb,
            "Expected #5A6588 (PLACEHOLDER_SLATE_MUTED) foreground cells for empty placeholders"
        );

        // 2. Verify soft-wrapping in all 4 input boxes
        let long_line = "alpha_segment_one beta_segment_two gamma_segment_three delta_segment_four epsilon_segment_five zeta_segment_six";
        for editor in &mut app.editors {
            editor.set_text(long_line);
        }
        app.sync_values_from_editors();
        let (wrapped, v_row, _) =
            wrap_editor_lines_with_cursor(&[long_line.to_string()], (0, long_line.len()), 36);
        assert!(
            wrapped.len() >= 3 && v_row >= 2,
            "Expected long line to soft-wrap across >=3 visual lines, got {:?}",
            wrapped
        );
        let wrapped_buf = render_headless_buffer(&mut app, 100, 38).unwrap();
        let wrapped_screen = buffer_to_string(&wrapped_buf);
        assert!(
            wrapped_screen.contains("alpha_segment_one")
                && wrapped_screen.contains("zeta_segment_six"),
            "Soft-wrapped segments must both be visible inside the editor boxes"
        );

        // 3. Verify Alt+U Model Sync/Add Modal adds custom model while preserving field values
        app.handle_key_event(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::ALT))
            .unwrap();
        assert!(app.model_modal.is_open);
        for ch in "Nebula".chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .unwrap();
        }
        let modal_buf = render_headless_buffer(&mut app, 120, 40).unwrap();
        let modal_screen = buffer_to_string(&modal_buf);
        assert!(modal_screen.contains("Model Catalog Sync & Add Custom Sub-Model"));
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(!app.model_modal.is_open);
        assert_eq!(app.active_agy_model_id(), "gemini-nebula-medium");
        assert_eq!(app.values.objective, long_line);

        // 4. Verify adaptive compact layout (75x30) and minimum window size guard (< 75x28)
        let compact_buf = render_headless_buffer(&mut app, 75, 30).unwrap();
        let compact_screen = buffer_to_string(&compact_buf);
        assert!(compact_screen.contains("Objective"));
        assert!(compact_screen.contains("Output"));

        let tiny_buf = render_headless_buffer(&mut app, 60, 18).unwrap();
        let tiny_screen = buffer_to_string(&tiny_buf);
        assert!(tiny_screen.contains("Minimum size: 75x28"));

        if let Some(ref tmp) = app.custom_xdg_data_dir {
            let _ = std::fs::remove_dir_all(tmp);
        }
    }

    #[test]
    fn test_step6_unfocused_title_contrast_nonwarping_banner_scaling_slash_commands_and_placeholder_free() {
        let mut app = build_test_app();
        // Focus field 0 (Objective), leaving fields 1, 2, 3 unfocused
        app.focus = FocusPane::FieldEditor(0);
        for editor in &mut app.editors {
            editor.set_text("");
        }
        app.sync_values_from_editors();

        // 1. Verify high-contrast unfocused field title RGB cells (#FF9E44 / #9BA8CC on #121521, distinct from #2A314A border)
        let buf_150 = render_headless_buffer(&mut app, 150, 45).unwrap();
        let editor_rects_150 = compute_field_editor_rects_for_terminal(150, 45, &app);
        assert_eq!(editor_rects_150.len(), 4);

        for (idx, rect) in editor_rects_150.iter().enumerate().skip(1) {
            let top_y = rect.y;
            let mut has_inactive_title_amber = false;
            let mut has_inactive_badge_slate = false;
            let mut has_inactive_border_2a314a = false;
            for x in rect.x..rect.x + rect.width {
                let cell = &buf_150[(x, top_y)];
                if cell.fg == novos_palette::ACCRETION_AMBER.to_ratatui()
                    && cell.bg == novos_palette::COSMIC_SPACE_SURFACE.to_ratatui()
                {
                    has_inactive_title_amber = true;
                }
                if cell.fg == novos_palette::INACTIVE_BADGE_SLATE.to_ratatui()
                    && cell.bg == novos_palette::COSMIC_SPACE_SURFACE.to_ratatui()
                {
                    has_inactive_badge_slate = true;
                }
                if cell.fg == novos_palette::ELEVATED_BORDER_SLATE.to_ratatui() {
                    has_inactive_border_2a314a = true;
                }
            }
            assert!(
                has_inactive_title_amber && has_inactive_badge_slate && has_inactive_border_2a314a,
                "Unfocused field {} missing #FF9E44 title, #9BA8CC badge, or #2A314A border on top row",
                idx
            );
        }

        // 2. Verify non-warping 3-tier branding banner and >= 75x6 field rects across 150x45, 100x36, and 75x30
        for (w, h, expected_tier) in [
            (150u16, 45u16, BannerTier::Full7Row),
            (100u16, 36u16, BannerTier::Medium3Row),
            (75u16, 30u16, BannerTier::Compact1Row),
        ] {
            assert_eq!(select_banner_tier(w.saturating_sub(2), h), expected_tier);
            let rects = compute_field_editor_rects_for_terminal(w, h, &app);
            assert_eq!(rects.len(), 4);
            for (idx, r) in rects.iter().enumerate() {
                assert!(
                    r.width >= MIN_FIELD_EDITOR_WIDTH,
                    "Field {} width {} < 75 at {}x{}",
                    idx,
                    r.width,
                    w,
                    h
                );
                assert!(
                    r.height >= MIN_FIELD_EDITOR_HEIGHT,
                    "Field {} height {} < 6 at {}x{}",
                    idx,
                    r.height,
                    w,
                    h
                );
                assert!(
                    r.height.saturating_sub(2) >= MIN_FIELD_INNER_TEXT_ROWS,
                    "Field {} inner rows {} < 4 at {}x{}",
                    idx,
                    r.height.saturating_sub(2),
                    w,
                    h
                );
            }

            let buf = render_headless_buffer(&mut app, w, h).unwrap();
            let screen = buffer_to_string(&buf);
            assert!(
                screen.contains("VERBOCITY"),
                "Missing VERBOCITY banner at {}x{}",
                w,
                h
            );
            // Verify zero option truncation in Control Bar, Field Headers/Sub-Headers, and Action Bar
            for required_token in [
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
                assert!(
                    screen.contains(required_token),
                    "Truncated or missing '{}' at {}x{}:\n{}",
                    required_token,
                    w,
                    h,
                    screen
                );
            }
        }

        // 3. Verify all built-in agy slash commands + (none) are selectable via cycle_slash_command
        let mut seen_commands: Vec<Option<String>> = Vec::new();
        let total_opts = app.slash_command_options().len();
        for _ in 0..total_opts {
            seen_commands.push(app.values.slash_command.clone());
            app.cycle_slash_command(true);
        }
        for builtin in crate::template::BUILTIN_AGY_SLASH_COMMANDS {
            assert!(
                seen_commands.contains(&Some(builtin.to_string())),
                "Built-in agy slash command /{} not selectable in cycle_slash_command",
                builtin
            );
        }
        assert!(
            seen_commands.contains(&None),
            "(none) option not selectable in cycle_slash_command"
        );

        // 4. Verify empty fields never emit <placeholder> text into rendered_prompt or DispatchManifest
        app.values.slash_command = Some("plan".to_string());
        let empty_rendered = app.rendered_prompt();
        assert_eq!(empty_rendered, "/plan");
        assert!(!empty_rendered.contains('<'));
        let empty_manifest = app
            .confirm_dispatch(DispatchAction::StdoutClipboard)
            .unwrap();
        assert_eq!(empty_manifest.rendered_prompt, "/plan");
        assert!(!empty_manifest.rendered_prompt.contains("1-2 sentence"));
        assert!(!empty_manifest.agy_command_preview.contains("1-2 sentence"));

        if let Some(ref tmp) = app.custom_xdg_data_dir {
            let _ = std::fs::remove_dir_all(tmp);
        }
    }

    #[test]
    fn test_step7_settings_page_model_toml_ingest_and_shell_formatting() {
        let mut app = build_test_app();
        let tmp_dir = app.custom_xdg_data_dir.clone().unwrap();
        app.custom_config_toml_path = Some(tmp_dir.join("config.toml"));
        app.custom_models_toml_path = Some(tmp_dir.join("models.toml"));

        let complex_obj = "Please add the following features:\n* Create a Settings Page (Alt+S)\n* Allow model .toml ingestion\n* Handle \"quotes\", 'single', $VARS, and (parens)";
        app.editors[0].set_text(complex_obj);
        app.editors[1].set_text("src/settings.rs, src/formatting.rs");
        app.editors[2].set_text("Preserve field text on Alt+S / Esc");
        app.editors[3].set_text("Execute Step 7");
        app.sync_values_from_editors();

        // 1. Open Settings Page via Alt+S and verify all 4 tabs render and navigate cleanly
        app.handle_key_event(KeyEvent::new(KeyCode::Char('s'), KeyModifiers::ALT))
            .unwrap();
        assert!(app.settings_page.is_open);
        assert_eq!(app.settings_page.active_tab, SettingsTab::ThemeAndContrast);

        let tab1_buf = render_headless_buffer(&mut app, 130, 40).unwrap();
        let tab1_screen = buffer_to_string(&tab1_buf);
        assert!(tab1_screen.contains("VERBOCITY SETTINGS PAGE (Alt+S)"));
        assert!(tab1_screen.contains("Primary Accent (accent_primary)"));

        // Switch to Tab 2 (Models & TOML Ingestion) via '2'
        app.handle_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.settings_page.active_tab, SettingsTab::ModelsAndToml);
        let tab2_buf = render_headless_buffer(&mut app, 130, 40).unwrap();
        let tab2_screen = buffer_to_string(&tab2_buf);
        assert!(tab2_screen.contains("Ingest Model .toml / .yaml File"));

        // Write an external model .toml file and ingest it from Tab 2
        let external_model_toml = tmp_dir.join("gemini-orion.toml");
        std::fs::write(
            &external_model_toml,
            r#"
family = "Gemini"
short_name = "Orion"
display_name = "Gemini 4 Orion"
agy_base_id = "gemini-4-orion"
available_tiers = ["low", "medium", "high"]
default_effort = "high"
"#,
        )
        .unwrap();
        for ch in external_model_toml.to_string_lossy().chars() {
            app.handle_key_event(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE))
                .unwrap();
        }
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(app.catalog.find_by_query("Orion").is_some());
        assert_eq!(app.active_profile().short_name, "Orion");
        assert!(tmp_dir.join("models.toml").is_file());

        // Switch to Tab 3 (Templates & Defaults) via Alt+3
        app.handle_key_event(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT))
            .unwrap();
        assert_eq!(
            app.settings_page.active_tab,
            SettingsTab::TemplatesAndDefaults
        );
        let tab3_buf = render_headless_buffer(&mut app, 130, 40).unwrap();
        assert!(buffer_to_string(&tab3_buf).contains("Active Prompt Template"));

        // Switch to Tab 4 (Execution, Shell Formatting & Workspace Storage) via '4'
        app.handle_key_event(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::NONE))
            .unwrap();
        assert_eq!(
            app.settings_page.active_tab,
            SettingsTab::ExecutionAndWorkspace
        );
        // Row 0 = Default agy --mode -> cycle to 'auto' then 'plan'
        app.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .unwrap(); // auto
        app.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .unwrap(); // plan
        // Move to Row 1 = Default agy --sandbox -> toggle to true
        app.handle_key_event(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE))
            .unwrap();
        app.handle_key_event(KeyEvent::new(KeyCode::Right, KeyModifiers::NONE))
            .unwrap();
        assert_eq!(app.global_config.agy_flags.mode.as_str(), "plan");
        assert!(app.global_config.agy_flags.sandbox);
        assert!(tmp_dir.join("config.toml").is_file());

        // Exit Settings Page via Esc and verify all prompt field text is preserved
        app.handle_key_event(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE))
            .unwrap();
        assert!(!app.settings_page.is_open);
        assert_eq!(app.values.objective, complex_obj);
        assert_eq!(app.values.constraints, "Preserve field text on Alt+S / Esc");

        // 2. Trigger interactive dispatch (Alt+1) and verify ShellQuoteModal opens on complex input
        app.handle_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT))
            .unwrap();
        assert!(app.shell_quote_modal.is_open);
        assert!(!app.should_quit);
        let quote_buf = render_headless_buffer(&mut app, 130, 40).unwrap();
        let quote_screen = buffer_to_string(&quote_buf);
        assert!(quote_screen.contains("Shell Metacharacter & Quote Confirmation"));
        assert!(quote_screen.contains("Auto-Escape & Quote for Terminal Execution"));

        // Approve with Enter and verify shell-parseable command in DispatchManifest
        app.handle_key_event(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE))
            .unwrap();
        assert!(!app.shell_quote_modal.is_open);
        assert!(
            !app.should_quit,
            "Verbocity must remain open after confirming dispatch (only Ctrl+Q quits)"
        );
        let manifest = app.confirmed_manifest.as_ref().unwrap();
        let parsed_words =
            crate::formatting::parse_posix_shell_words(&manifest.shell_safe_command).unwrap();
        assert_eq!(parsed_words, manifest.agy_argv);
        assert!(manifest.agy_extra_flags.contains(&"--mode".to_string()));
        assert!(manifest.agy_extra_flags.contains(&"plan".to_string()));
        assert!(manifest.agy_extra_flags.contains(&"--sandbox".to_string()));

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_step8_persistent_execution_loop_and_terminal_output_pane() {
        let mut app = build_test_app();
        let tmp_dir = app.custom_xdg_data_dir.clone().unwrap();
        std::fs::create_dir_all(&tmp_dir).unwrap();

        // Create a mock `agy` executable that echoes its arguments
        let mock_agy = tmp_dir.join("mock-agy-step8");
        std::fs::write(
            &mock_agy,
            "#!/usr/bin/env bash\necho \"MOCK_AGY_EXEC: $*\"\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&mock_agy).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&mock_agy, perms).unwrap();
        }
        app.custom_agy_bin = Some(mock_agy);
        app.global_config.shell_formatting_policy =
            crate::formatting::ShellFormattingPolicy::AutoEscapeSilently;
        app.saved_markdown_path = Some(tmp_dir.join("step8-saved.md"));

        app.editors[0].set_text("Execute Step 8 persistent loop verification");
        app.editors[1].set_text("src/app.rs, src/tui.rs, src/main.rs");
        app.editors[2].set_text("Do not close Verbocity upon command execution");
        app.editors[3].set_text("Verify with cargo test");
        app.sync_values_from_editors();

        // 1. Dispatch Action 2 (`agy -p`): must stay open (`should_quit == false`) and stream output to TerminalOutputDrawer
        app.handle_key_event(KeyEvent::new(KeyCode::Char('2'), KeyModifiers::ALT))
            .unwrap();
        assert!(!app.should_quit);
        assert!(app.terminal_output.is_open);
        assert_eq!(app.focus, FocusPane::TerminalOutput);
        assert_eq!(app.terminal_output.last_exit_code, Some(0));
        assert!(app
            .terminal_output
            .lines
            .iter()
            .any(|l| l.contains("MOCK_AGY_EXEC: -p --model")));

        let buf = render_headless_buffer(&mut app, 150, 45).unwrap();
        let screen = buffer_to_string(&buf);
        assert!(screen.contains("Terminal Execution Output Pane [Ctrl+O]"));
        assert!(screen.contains("MOCK_AGY_EXEC: -p --model"));

        // 2. Dispatch Action 3 (`Stdout / Clipboard`): stays open and appends to Terminal Output Pane
        app.handle_key_event(KeyEvent::new(KeyCode::Char('3'), KeyModifiers::ALT))
            .unwrap();
        assert!(!app.should_quit);
        assert!(app
            .terminal_output
            .lines
            .iter()
            .any(|l| l.contains("Action 3: Rendered Prompt Output / Clipboard")));

        // 3. Dispatch Action 4 (`Save .md`): stays open and writes file
        app.handle_key_event(KeyEvent::new(KeyCode::Char('4'), KeyModifiers::ALT))
            .unwrap();
        assert!(!app.should_quit);
        assert!(tmp_dir.join("step8-saved.md").is_file());

        // 4. Dispatch Action 1 (`agy -i`): executes child handoff and returns to active App loop with `should_quit == false`
        app.handle_key_event(KeyEvent::new(KeyCode::Char('1'), KeyModifiers::ALT))
            .unwrap();
        assert!(!app.should_quit);
        assert_eq!(app.terminal_output.last_exit_code, Some(0));
        assert!(app
            .terminal_output
            .lines
            .iter()
            .any(|l| l.contains("MOCK_AGY_EXEC: -i --model")));
        assert_eq!(
            app.values.objective,
            "Execute Step 8 persistent loop verification"
        );

        // 5. Toggle Terminal Output Pane via `Ctrl+O` and verify `Ctrl+Q` is the only action that sets `should_quit = true`
        app.handle_key_event(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(!app.terminal_output.is_open);
        app.handle_key_event(KeyEvent::new(KeyCode::Char('q'), KeyModifiers::CONTROL))
            .unwrap();
        assert!(app.should_quit);

        let _ = std::fs::remove_dir_all(&tmp_dir);
    }
}
