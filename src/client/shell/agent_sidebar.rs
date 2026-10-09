use std::collections::HashMap;

use ratatui::{
    buffer::Buffer,
    layout::Rect,
    style::{Modifier, Style},
    text::Line,
    widgets::{Paragraph, Widget},
};

use super::*;

pub(super) struct AgentRow {
    pub(super) pane_id: String,
    pub(super) status: crate::api::schema::AgentStatus,
    pub(super) focused: bool,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
    /// One entry per configured footer row (empty when this agent resolves none).
    pub(super) footer: Vec<Vec<crate::ui::ResolvedToken>>,
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
) -> Vec<String> {
    if snapshot.agent_view_label.is_some() {
        return snapshot
            .agent_order
            .iter()
            .filter(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .any(|agent| agent.pane_id == pane_id.as_str())
            })
            .cloned()
            .collect();
    }
    let mut agents = snapshot.agents.iter().collect::<Vec<_>>();
    if sort == crate::config::AgentPanelSortConfig::Priority {
        agents.sort_by_key(|agent| {
            (
                std::cmp::Reverse(status_priority(agent.agent_status)),
                std::cmp::Reverse(agent.state_change_seq),
            )
        });
    }
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
) {
    if !render_agent_panel_header(
        buffer,
        area,
        snapshot.agent_view_label.as_deref(),
        config,
        hits,
    ) {
        return;
    }

    let rows = agent_rows(snapshot, config, None);
    let footer = crate::ui::sidebar_agent_footer_rows(
        &config.agents.footer,
        rows.iter().map(|row| row.footer.as_slice()),
        snapshot
            .workspaces
            .iter()
            .map(|workspace| workspace.tokens.as_slice()),
    );
    render_agent_list(
        buffer,
        area,
        &rows,
        &footer,
        snapshot
            .agent_view_label
            .as_ref()
            .map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row.rows.len(),
        |buffer, rect, row, hits| {
            hits.agents.push((rect, row.pane_id.clone()));
            render_agent_row(buffer, rect, row, config);
        },
    );
}

pub(super) fn render_agent_panel_header(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) -> bool {
    if area.height == 0 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
    );
    if area.height < 2 {
        return false;
    }
    put_text(
        buffer,
        area.x,
        area.y + 1,
        area.width,
        " agents",
        Style::default()
            .fg(config.palette.overlay0)
            .add_modifier(Modifier::BOLD),
    );
    let sort_label = agent_view_label.unwrap_or(match config.agent_panel_sort {
        crate::config::AgentPanelSortConfig::Spaces => "grouped",
        crate::config::AgentPanelSortConfig::Priority => "priority",
    });
    let sort_width = display_width(sort_label).min(area.width as usize) as u16;
    let sort_rect = Rect::new(
        area.right().saturating_sub(sort_width),
        area.y + 1,
        sort_width,
        1,
    );
    hits.agent_sort_toggle = if config.mouse_capture && agent_view_label.is_none() {
        sort_rect
    } else {
        Rect::default()
    };
    put_text(
        buffer,
        sort_rect.x,
        sort_rect.y,
        sort_rect.width,
        sort_label,
        Style::default()
            .fg(if agent_view_label.is_some() {
                config.palette.accent
            } else {
                config.palette.overlay0
            })
            .add_modifier(Modifier::BOLD),
    );
    true
}

pub(super) fn render_agent_list<T>(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[T],
    footer: &[Vec<crate::ui::ResolvedToken>],
    empty_message: Option<&str>,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    hits: &mut ShellHitMap,
    row_lines: impl Fn(&T) -> usize,
    mut render_row: impl FnMut(&mut Buffer, Rect, &T, &mut ShellHitMap),
) {
    let list = Rect::new(
        area.x,
        area.y.saturating_add(3),
        area.width,
        area.height.saturating_sub(3),
    );
    // The list keeps MIN_AGENT_LIST_ROWS; footer rows beyond that are dropped
    // from the top so the bottom (account) rows survive.
    let footer_height = list
        .height
        .saturating_sub(MIN_AGENT_LIST_ROWS)
        .min(u16::try_from(footer.len()).unwrap_or(u16::MAX));
    let body = Rect {
        height: list.height - footer_height,
        ..list
    };
    render_agent_footer(
        buffer,
        Rect {
            y: body.bottom(),
            height: footer_height,
            ..list
        },
        &footer[footer.len() - usize::from(footer_height)..],
        config,
    );
    hits.agent_body = body;
    if body.is_empty() || rows.is_empty() {
        *agent_scroll = 0;
        if let Some(message) = empty_message.filter(|_| !body.is_empty()) {
            put_text(
                buffer,
                body.x,
                body.y,
                body.width,
                message,
                Style::default()
                    .fg(config.palette.overlay0)
                    .add_modifier(Modifier::DIM),
            );
        }
        return;
    }

    let row_heights = rows
        .iter()
        .map(|row| row_lines(row).max(1).min(u16::MAX as usize) as u16)
        .collect::<Vec<_>>();
    let gaps = rows
        .iter()
        .enumerate()
        .map(|(index, _)| {
            if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            }
        })
        .collect::<Vec<_>>();
    let metrics =
        super::scroll::list_scroll_metrics(&row_heights, &gaps, body.height, *agent_scroll);
    hits.agent_max_scroll = metrics.max_offset_from_bottom;
    hits.agent_scroll_metrics = Some(metrics);
    *agent_scroll = metrics
        .max_offset_from_bottom
        .saturating_sub(metrics.offset_from_bottom);
    let show_scrollbar = metrics.max_offset_from_bottom > 0 && body.width > 1;
    let content_width = body.width.saturating_sub(u16::from(show_scrollbar));
    let mut y = body.y;
    for (index, row) in rows.iter().enumerate().skip(*agent_scroll) {
        let height = row_heights[index].min(body.height);
        if y.saturating_add(height) > body.bottom() {
            break;
        }
        let rect = Rect::new(body.x, y, content_width, height);
        render_row(buffer, rect, row, hits);
        y = y
            .saturating_add(height)
            .saturating_add(if index + 1 < rows.len() {
                config.agents.row_gap
            } else {
                0
            });
    }

    if show_scrollbar {
        let track = Rect::new(body.right().saturating_sub(1), body.y, 1, body.height);
        hits.agent_scrollbar = track;
        super::scroll::render_list_scrollbar(buffer, track, metrics, &config.palette);
    }
}

const MIN_AGENT_LIST_ROWS: u16 = 2;

fn render_agent_footer(
    buffer: &mut Buffer,
    area: Rect,
    footer: &[Vec<crate::ui::ResolvedToken>],
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let base = Style::default().fg(palette.overlay0);
    for (offset, tokens) in footer.iter().enumerate() {
        let y = area.y + offset as u16;
        // Both expanded sidebars draw `«` in the panel's last cell after the
        // panel renders; the last footer row leaves it and one blank before it.
        let reserved = if y + 1 == area.bottom() { 3 } else { 1 };
        let mut spans = vec![ratatui::text::Span::raw(" ")];
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            ("", base),
            base,
            base,
            base,
            base,
            palette,
            area.width.saturating_sub(reserved) as usize,
            crate::ui::TokenJoin::Space,
        ));
        Paragraph::new(Line::from(spans)).render(Rect::new(area.x, y, area.width, 1), buffer);
    }
}

pub(super) fn agent_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Vec<AgentRow> {
    ordered_agent_pane_ids(snapshot, config.agent_panel_sort)
        .into_iter()
        .filter_map(|pane_id| agent_row(snapshot, &pane_id, config, machine))
        .collect()
}

pub(super) fn agent_row(
    snapshot: &ClientShellSnapshot,
    pane_id: &str,
    config: &ClientShellConfig,
    machine: Option<&str>,
) -> Option<AgentRow> {
    let agent = snapshot
        .agents
        .iter()
        .find(|agent| agent.pane_id == pane_id)?;
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|workspace| workspace.workspace_id == agent.workspace_id)?;
    let tab = snapshot.tabs.iter().find(|tab| tab.tab_id == agent.tab_id);
    let pane = snapshot
        .panes
        .iter()
        .find(|pane| pane.pane_id == agent.pane_id);
    let tab_count = snapshot
        .tabs
        .iter()
        .filter(|candidate| candidate.workspace_id == agent.workspace_id)
        .count();
    let tab_label = tab
        .filter(|tab| tab_count > 1 || tab.custom_label)
        .map(|tab| tab.label.as_str());
    let agent_label = agent
        .display_agent
        .as_deref()
        .or(agent.name.as_deref())
        .or(agent.agent.as_deref())
        .or(agent.title.as_deref());
    let labels = agent
        .state_labels
        .iter()
        .cloned()
        .collect::<HashMap<_, _>>();
    let tokens = agent.tokens.iter().cloned().collect::<HashMap<_, _>>();
    let state_text = labels
        .get(status_text(agent.agent_status))
        .map(String::as_str)
        .unwrap_or_else(|| sidebar_status_text(agent.agent_status));
    let canonical_agent = agent
        .agent
        .as_deref()
        .and_then(crate::detect::parse_agent_label);
    let context = crate::ui::AgentTokenContext {
        machine,
        workspace: &workspace.label,
        tab: tab_label,
        pane: agent
            .title
            .as_deref()
            .or_else(|| pane.and_then(|pane| pane.label.as_deref())),
        agent_label,
        terminal_title: agent.terminal_title.as_deref(),
        terminal_title_stripped: agent.terminal_title_stripped.as_deref(),
        canonical_agent,
        tokens: &tokens,
    };
    let footer =
        crate::ui::sidebar_agent_footer_candidates(&config.agents.footer, &context, state_text);
    let rows = crate::ui::sidebar_agent_rows(&config.agents, context, state_text);
    Some(AgentRow {
        pane_id: agent.pane_id.clone(),
        status: agent.agent_status,
        focused: agent.focused,
        rows,
        footer,
    })
}

pub(super) fn render_agent_row(
    buffer: &mut Buffer,
    rect: Rect,
    row: &AgentRow,
    config: &ClientShellConfig,
) {
    let palette = &config.palette;
    let row_style = if row.focused {
        Style::default().bg(palette.active_row_bg)
    } else {
        Style::default()
    };
    let name_style = if row.focused {
        Style::default()
            .fg(palette.text)
            .add_modifier(Modifier::BOLD)
    } else {
        Style::default()
            .fg(palette.subtext0)
            .add_modifier(Modifier::BOLD)
    };
    let status_style = Style::default().fg(status_color(row.status, palette));
    let secondary = Style::default().fg(palette.overlay0);
    let icon = (
        status_icon(row.status, config.status_indicators),
        Style::default().fg(status_color(row.status, palette)),
    );
    let rows = if row.rows.is_empty() {
        vec![vec![crate::ui::ResolvedToken {
            kind: crate::ui::ResolvedTokenKind::StateIcon,
            style: Default::default(),
        }]]
    } else {
        row.rows.clone()
    };
    for (index, tokens) in rows.iter().take(rect.height as usize).enumerate() {
        let indent = if index == 0 { 1 } else { 3 };
        let mut spans = vec![ratatui::text::Span::raw(" ".repeat(indent))];
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            icon,
            status_style,
            name_style,
            secondary,
            secondary,
            palette,
            rect.width.saturating_sub(indent as u16) as usize,
            crate::ui::TokenJoin::Separator,
        ));
        Paragraph::new(Line::from(spans)).style(row_style).render(
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            buffer,
        );
    }
}

fn put_text(buffer: &mut Buffer, x: u16, y: u16, width: u16, text: &str, style: Style) {
    for (offset, character) in text.chars().take(width as usize).enumerate() {
        if let Some(cell) = buffer.cell_mut((x + offset as u16, y)) {
            cell.set_char(character).set_style(style);
        }
    }
}

fn display_width(text: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(text)
}

fn sidebar_status_text(status: crate::api::schema::AgentStatus) -> &'static str {
    use crate::api::schema::AgentStatus;
    match status {
        AgentStatus::Blocked => "blocked",
        AgentStatus::Done => "done",
        AgentStatus::Working => "working",
        AgentStatus::Idle | AgentStatus::Unknown => "idle",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::protocol::{render_ansi::BlitEncoder, FrameData};

    const ICON: &str = "\u{e1a0}";

    fn rendered(rows_toml: &str, width: u16) -> (Buffer, ClientShellConfig) {
        let sidebar: crate::config::AgentsSidebarConfig = toml::from_str(rows_toml).unwrap();
        let tokens = HashMap::from([
            ("icon".to_string(), ICON.to_string()),
            ("model".to_string(), "opus-4".to_string()),
            ("empty".to_string(), String::new()),
        ]);
        let rows = crate::ui::sidebar_agent_rows(
            &sidebar,
            crate::ui::AgentTokenContext {
                machine: None,
                workspace: "repo",
                tab: None,
                pane: None,
                agent_label: None,
                terminal_title: None,
                terminal_title_stripped: None,
                canonical_agent: None,
                tokens: &tokens,
            },
            "idle",
        );
        let config = ClientShellConfig::from_config(&crate::config::Config::default());
        let rect = Rect::new(0, 0, width, rows.len() as u16);
        let mut buffer = Buffer::empty(rect);
        let row = AgentRow {
            pane_id: "pane_1".into(),
            status: crate::api::schema::AgentStatus::Idle,
            focused: true,
            rows,
            footer: Vec::new(),
        };
        render_agent_row(&mut buffer, rect, &row, &config);
        (buffer, config)
    }

    fn row_text(buffer: &Buffer, y: u16) -> String {
        (0..buffer.area.width)
            .map(|x| buffer[(x, y)].symbol())
            .collect::<String>()
            .trim_end()
            .to_string()
    }

    const ICON_ROWS: &str = r##"
rows = [[{ token = "$icon", fg = "#f9e2af", bold = false, dim = false }, { token = "$model", fg = "#e9e9f0", bold = true, dim = false }], ["state_icon", "workspace"]]
"##;

    #[test]
    fn separator_leading_blank_stays_in_the_preceding_token_style_run() {
        let (buffer, config) = rendered(ICON_ROWS, 30);
        let overlay0 = config.palette.overlay0;
        assert_eq!(row_text(&buffer, 0), format!(" {ICON} · opus-4"));

        // Outer terminals shape glyphs per attribute run, so the blank after a
        // wide icon must not start a new run.
        let (icon, blank, dot, trailing) = (
            &buffer[(1, 0)],
            &buffer[(2, 0)],
            &buffer[(3, 0)],
            &buffer[(4, 0)],
        );
        assert_eq!(icon.symbol(), ICON);
        assert_eq!(blank.symbol(), " ");
        assert_eq!(blank.style(), icon.style());
        assert_eq!(dot.symbol(), "·");
        assert_eq!(dot.fg, overlay0);
        assert_eq!(trailing.fg, overlay0);
        assert_eq!(dot.modifier, Modifier::empty());
        assert_eq!(trailing.style(), dot.style());
        // Nothing with ink on a blank cell may leak into the separator.
        assert_eq!(blank.bg, dot.bg);
        assert_eq!(blank.underline_color, dot.underline_color);
        assert!(!blank
            .modifier
            .intersects(Modifier::REVERSED | Modifier::UNDERLINED | Modifier::CROSSED_OUT));

        let frame = FrameData::from_ratatui_buffer(&buffer, None);
        let ansi = BlitEncoder::new().encode(&frame, true).bytes;
        let ansi = String::from_utf8_lossy(&ansi);
        let after_icon = &ansi[ansi.find(ICON).unwrap() + ICON.len()..];
        let (before_blank, after_blank) = after_icon.split_once(' ').unwrap();
        assert!(
            !before_blank.contains('m'),
            "style change between icon and blank: {before_blank:?}"
        );
        let before_dot = after_blank.split_once('·').unwrap().0;
        assert!(
            before_dot.ends_with('m'),
            "separator colour must start after the blank: {before_dot:?}"
        );

        // The single-space separator after the state icon follows the same rule.
        assert_eq!(buffer[(4, 1)].symbol(), " ");
        assert_eq!(buffer[(4, 1)].style(), buffer[(3, 1)].style());
        assert_eq!(
            row_text(&buffer, 1)[buffer[(3, 1)].symbol().len() + 3..],
            *" repo"
        );
    }

    #[test]
    fn separator_split_keeps_text_and_width_for_narrow_and_empty_tokens() {
        // Truncated: icon + " · " + 3 cells of the model.
        let (buffer, _) = rendered(ICON_ROWS, 8);
        assert_eq!(row_text(&buffer, 0), format!(" {ICON} · op…"));
        assert_eq!(buffer[(2, 0)].style(), buffer[(1, 0)].style());

        // Too narrow for both: the icon is dropped and no separator is drawn.
        let (buffer, _) = rendered(ICON_ROWS, 4);
        assert_eq!(row_text(&buffer, 0), " op…");

        // An empty token keeps both of its separators.
        let (buffer, config) = rendered(r##"rows = [["$icon", "$empty", "$model"]]"##, 30);
        assert_eq!(row_text(&buffer, 0), format!(" {ICON} ·  · opus-4"));
        assert_eq!(buffer[(2, 0)].style(), buffer[(1, 0)].style());
        assert_eq!(buffer[(3, 0)].fg, config.palette.overlay0);
        assert_eq!(buffer[(6, 0)].fg, config.palette.overlay0);
    }
}
