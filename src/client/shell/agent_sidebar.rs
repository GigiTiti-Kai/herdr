use std::collections::{HashMap, HashSet};

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
    pub(super) pinned: bool,
    pub(super) workspace_id: String,
    /// Whether this card's space is drawn with `folded_rows`.
    pub(super) folded: bool,
    /// Index into `rows` of the line that folds or unfolds this space when clicked.
    pub(super) toggle_row: Option<usize>,
    pub(super) rows: Vec<Vec<crate::ui::ResolvedToken>>,
    /// One entry per configured footer row (empty when this agent resolves none).
    pub(super) footer: Vec<Vec<crate::ui::ResolvedToken>>,
    /// The pane's repository, set only in a space whose agents span several.
    pub(super) repo: Option<AgentRepo>,
    /// Index into `rows` of the repository heading, drawn at the title indent.
    pub(super) repo_heading_row: Option<usize>,
}

pub(super) struct AgentRepo {
    key: String,
    name: String,
    /// The space title already names this repository.
    is_space_label: bool,
}

/// Spaces whose agent panes sit in more than one repository.
pub(super) fn mixed_repo_spaces(snapshot: &ClientShellSnapshot) -> HashSet<&str> {
    let agent_panes = snapshot
        .agents
        .iter()
        .map(|agent| agent.pane_id.as_str())
        .collect::<HashSet<_>>();
    let mut first_repo = HashMap::new();
    let mut mixed = HashSet::new();
    for pane in &snapshot.panes {
        let Some(git) = pane
            .git_context
            .as_ref()
            .filter(|_| agent_panes.contains(pane.pane_id.as_str()))
        else {
            continue;
        };
        let first = first_repo
            .entry(pane.workspace_id.as_str())
            .or_insert(git.repo_key.as_str());
        if *first != git.repo_key {
            mixed.insert(pane.workspace_id.as_str());
        }
    }
    mixed
}

/// In a space whose agents span several repositories, name the repository
/// above each run of cards that leaves the one before it. The space title
/// stands for the repository it is named after. Each row comes with a scope
/// that keeps equal space ids of different machines apart.
pub(super) fn insert_repo_headings<'a, S: PartialEq>(
    rows: impl IntoIterator<Item = (S, &'a mut AgentRow)>,
) {
    let mut previous: Option<(S, String, String)> = None;
    for (scope, row) in rows {
        let Some(repo) = row.repo.as_ref() else {
            // A card outside any repository leaves its own space's run open,
            // so the next card still gets a heading when its repository changed.
            if previous.as_ref().is_some_and(|(space_scope, space, _)| {
                *space_scope != scope || *space != row.workspace_id
            }) {
                previous = None;
            }
            continue;
        };
        let same_repo = previous
            .as_ref()
            .filter(|(space_scope, space, _)| *space_scope == scope && *space == row.workspace_id)
            .map(|(_, _, key)| *key == repo.key);
        let key = repo.key.clone();
        if !same_repo.unwrap_or(repo.is_space_label) && !row.rows.is_empty() {
            let heading = vec![crate::ui::ResolvedToken {
                kind: crate::ui::ResolvedTokenKind::Workspace(format!("+ {}", repo.name)),
                style: Default::default(),
            }];
            // Under the space title when this card carries it, else above the
            // card's first line of content.
            let at = row.toggle_row.map_or_else(
                || {
                    row.rows
                        .iter()
                        .take_while(|tokens| {
                            tokens.iter().all(crate::ui::ResolvedToken::is_blank_spacer)
                        })
                        .count()
                },
                |toggle| toggle + 1,
            );
            row.rows.insert(at, heading);
            row.repo_heading_row = Some(at);
        }
        previous = Some((scope, row.workspace_id.clone(), key));
    }
}

/// The user's click-chosen Agents-panel folds, Local and per remote endpoint.
#[derive(Clone, Copy)]
pub(super) struct AgentFolds<'a> {
    pub(super) local: &'a HashMap<String, bool>,
    pub(super) remote: &'a HashMap<ClientEndpointId, HashMap<String, bool>>,
}

impl<'a> AgentFolds<'a> {
    pub(super) fn for_endpoint(
        self,
        endpoint_id: &ClientEndpointId,
    ) -> Option<&'a HashMap<String, bool>> {
        if endpoint_id.is_local() {
            Some(self.local)
        } else {
            self.remote.get(endpoint_id)
        }
    }
}

/// A click-chosen fold wins; otherwise a space starts folded when its label is listed.
fn workspace_folded(
    config: &ClientShellConfig,
    overrides: Option<&HashMap<String, bool>>,
    workspace_id: &str,
    label: &str,
) -> bool {
    config.agents.fold_enabled()
        && overrides
            .and_then(|folds| folds.get(workspace_id))
            .copied()
            .unwrap_or_else(|| config.agents.start_folded.iter().any(|name| name == label))
}

/// Record a click target on the card's fold-toggle line, if it has one on screen.
pub(super) fn push_fold_hit(
    hits: &mut ShellHitMap,
    rect: Rect,
    endpoint_id: &ClientEndpointId,
    row: &AgentRow,
) {
    if let Some(index) = row
        .toggle_row
        .filter(|index| *index < usize::from(rect.height))
    {
        hits.agent_fold_toggles.push((
            Rect::new(rect.x, rect.y + index as u16, rect.width, 1),
            endpoint_id.clone(),
            row.workspace_id.clone(),
            row.folded,
        ));
    }
}

pub(super) fn ordered_agent_pane_ids(
    snapshot: &ClientShellSnapshot,
    sort: crate::config::AgentPanelSortConfig,
    pinned: &[String],
) -> Vec<String> {
    let mut agents = if snapshot.agent_view_label.is_some() {
        snapshot
            .agent_order
            .iter()
            .filter_map(|pane_id| {
                snapshot
                    .agents
                    .iter()
                    .find(|agent| agent.pane_id == pane_id.as_str())
            })
            .collect::<Vec<_>>()
    } else {
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
    };
    agents.sort_by_key(|agent| pinned_rank(agent.name.as_deref(), pinned));
    agents
        .into_iter()
        .map(|agent| agent.pane_id.clone())
        .collect()
}

/// Stable-sort key: pinned names first in config order, everyone else tied after them.
pub(super) fn pinned_rank(name: Option<&str>, pinned: &[String]) -> usize {
    name.and_then(|name| pinned.iter().position(|entry| entry == name))
        .unwrap_or(pinned.len())
}

pub(super) fn render_agent_panel(
    buffer: &mut Buffer,
    area: Rect,
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    endpoint_id: &ClientEndpointId,
    folds: AgentFolds<'_>,
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

    let rows = agent_rows(snapshot, config, None, folds.for_endpoint(endpoint_id));
    let footer = if config.agents.footer_in_bottom_bar() {
        Vec::new()
    } else {
        footer_from_rows(snapshot, config, &rows)
    };
    let pinned = rows.iter().take_while(|row| row.pinned).count();
    let pinned_height = rows[..pinned]
        .iter()
        .map(|row| row.rows.len().max(1))
        .sum::<usize>()
        + pinned.saturating_sub(1) * usize::from(config.agents.row_gap);
    let fits = pinned_height + 1 + usize::from(MIN_AGENT_LIST_ROWS)
        <= usize::from(area.height.saturating_sub(AGENT_LIST_TOP));
    let (fixed, listed) = rows.split_at(if fits { pinned } else { 0 });
    let mut list_area = area;
    if !fixed.is_empty() {
        render_pinned_agents(buffer, area, fixed, config, endpoint_id, hits);
        // render_agent_list adds its own header offset, so this lands under the separator.
        let shift = pinned_height as u16 + 1;
        list_area.y += shift;
        list_area.height -= shift;
    }
    hits.agent_pinned_rows = fixed.len();
    render_agent_list(
        buffer,
        list_area,
        listed,
        &footer,
        snapshot
            .agent_view_label
            .as_ref()
            .filter(|_| fixed.is_empty())
            .map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row.rows.len(),
        |buffer, rect, row, hits| {
            hits.agents.push((rect, row.pane_id.clone()));
            push_fold_hit(hits, rect, endpoint_id, row);
            render_agent_row(buffer, rect, row, config);
        },
    );
}

fn footer_from_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
    rows: &[AgentRow],
) -> Vec<Vec<crate::ui::ResolvedToken>> {
    crate::ui::sidebar_agent_footer_rows(
        &config.agents.footer,
        rows.iter().map(|row| row.footer.as_slice()),
        snapshot
            .workspaces
            .iter()
            .map(|workspace| workspace.tokens.as_slice()),
    )
}

/// The footer for the bottom band, picked the way the Agent panel picks it.
pub(super) fn bar_footer_rows(
    snapshot: &ClientShellSnapshot,
    config: &ClientShellConfig,
) -> Vec<Vec<crate::ui::ResolvedToken>> {
    footer_from_rows(snapshot, config, &agent_rows(snapshot, config, None, None))
}

/// Draw the footer rows side by side on one line: `row │ row`, cut on the right.
pub(super) fn render_footer_bar(
    buffer: &mut Buffer,
    area: Rect,
    footer: &[Vec<crate::ui::ResolvedToken>],
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    if area.is_empty() {
        return;
    }
    let palette = &config.palette;
    let base = Style::default().fg(palette.overlay0);
    buffer.set_style(area, Style::default().bg(palette.panel_bg));
    let width = area.width.saturating_sub(1) as usize;
    let mut spans = vec![ratatui::text::Span::raw(" ")];
    // Plugin spacer rows (only blanks or U+2800) would leave an empty gap.
    let visible = footer.iter().filter(|row| {
        !row.iter().all(|token| {
            matches!(
                &token.kind,
                crate::ui::ResolvedTokenKind::Custom(text)
                    if text.chars().all(|c| c.is_whitespace() || c == '\u{2800}')
            )
        })
    });
    for (index, tokens) in visible.enumerate() {
        if index > 0 {
            spans.push(ratatui::text::Span::styled(
                " │ ",
                Style::default().fg(palette.overlay0),
            ));
        }
        spans.extend(crate::ui::resolved_token_spans(
            tokens,
            ("", base),
            base,
            base,
            base,
            base,
            palette,
            width,
            crate::ui::TokenJoin::Space,
        ));
    }
    Paragraph::new(Line::from(spans)).render(area, buffer);
    if config.mouse_capture && config.agents.footer_click_action().is_some() {
        hits.agent_footer = area;
    }
}

fn render_pinned_agents(
    buffer: &mut Buffer,
    area: Rect,
    rows: &[AgentRow],
    config: &ClientShellConfig,
    endpoint_id: &ClientEndpointId,
    hits: &mut ShellHitMap,
) {
    let mut y = area.y + AGENT_LIST_TOP;
    for (index, row) in rows.iter().enumerate() {
        if index > 0 {
            y += config.agents.row_gap;
        }
        let rect = Rect::new(area.x, y, area.width, row.rows.len().max(1) as u16);
        hits.agents.push((rect, row.pane_id.clone()));
        push_fold_hit(hits, rect, endpoint_id, row);
        render_agent_row(buffer, rect, row, config);
        y = rect.bottom();
    }
    put_text(
        buffer,
        area.x,
        y,
        area.width,
        &"─".repeat(area.width as usize),
        Style::default().fg(config.palette.surface_dim),
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
        area.y.saturating_add(AGENT_LIST_TOP),
        area.width,
        area.height.saturating_sub(AGENT_LIST_TOP),
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
    let footer_area = Rect {
        y: body.bottom(),
        height: footer_height,
        ..list
    };
    render_agent_footer(
        buffer,
        footer_area,
        &footer[footer.len() - usize::from(footer_height)..],
        config,
    );
    hits.agent_footer = if config.mouse_capture && config.agents.footer_click_action().is_some() {
        footer_area
    } else {
        Rect::default()
    };
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
/// Panel rows above the list: top rule, `agents` heading, blank line.
const AGENT_LIST_TOP: u16 = 3;

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
    folds: Option<&HashMap<String, bool>>,
) -> Vec<AgentRow> {
    let mixed = mixed_repo_spaces(snapshot);
    let mut rows = ordered_agent_pane_ids(snapshot, config.agent_panel_sort, &config.agents.pinned)
        .into_iter()
        .filter_map(|pane_id| agent_row(snapshot, &pane_id, config, machine, folds, &mixed))
        .collect::<Vec<_>>();
    // Pinned cards are drawn apart from the list, so a run never crosses that split.
    insert_repo_headings(rows.iter_mut().map(|row| (row.pinned, row)));
    rows
}

pub(super) fn agent_row(
    snapshot: &ClientShellSnapshot,
    pane_id: &str,
    config: &ClientShellConfig,
    machine: Option<&str>,
    folds: Option<&HashMap<String, bool>>,
    mixed_repo_spaces: &HashSet<&str>,
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
    let repo = pane
        .and_then(|pane| pane.git_context.as_ref())
        .filter(|_| mixed_repo_spaces.contains(workspace.workspace_id.as_str()))
        .map(|git| AgentRepo {
            key: git.repo_key.clone(),
            name: git.repo.clone(),
            is_space_label: git.repo == workspace.label,
        });
    // The repository heading names the repo, so the card's rows drop the
    // server's `$repo` copy of it. The footer keeps it.
    let row_tokens = repo
        .as_ref()
        .filter(|repo| tokens.get("repo") == Some(&repo.name))
        .map(|_| {
            let mut tokens = tokens.clone();
            tokens.remove("repo");
            tokens
        });
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
    let folded = workspace_folded(config, folds, &workspace.workspace_id, &workspace.label);
    let context = crate::ui::AgentTokenContext {
        tokens: row_tokens.as_ref().unwrap_or(&tokens),
        ..context
    };
    let crate::ui::AgentRows { rows, toggle_row } =
        crate::ui::sidebar_agent_rows_folded(&config.agents, context, state_text, folded);
    Some(AgentRow {
        pane_id: agent.pane_id.clone(),
        status: agent.agent_status,
        focused: agent.focused,
        pinned: pinned_rank(agent.name.as_deref(), &config.agents.pinned)
            < config.agents.pinned.len(),
        workspace_id: workspace.workspace_id.clone(),
        folded,
        toggle_row,
        rows,
        footer,
        repo,
        repo_heading_row: None,
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
        // The repository heading sits at the title indent and does not push
        // the card's own first line off it.
        let heading = row.repo_heading_row;
        let own_index = index - usize::from(heading.is_some_and(|heading| heading < index));
        let indent = if own_index == 0 || heading == Some(index) {
            1
        } else {
            3
        };
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
        let tokens = HashMap::from([
            ("icon".to_string(), ICON.to_string()),
            ("model".to_string(), "opus-4".to_string()),
            ("empty".to_string(), String::new()),
        ]);
        rendered_with_tokens(
            rows_toml,
            width,
            "idle",
            crate::api::schema::AgentStatus::Idle,
            tokens,
            None,
        )
    }

    fn rendered_with_tokens(
        rows_toml: &str,
        width: u16,
        state_text: &str,
        status: crate::api::schema::AgentStatus,
        tokens: HashMap<String, String>,
        canonical_agent: Option<crate::detect::Agent>,
    ) -> (Buffer, ClientShellConfig) {
        let sidebar: crate::config::AgentsSidebarConfig = toml::from_str(rows_toml).unwrap();
        let rows = crate::ui::sidebar_agent_rows(
            &sidebar,
            crate::ui::AgentTokenContext {
                machine: None,
                workspace: "repo",
                tab: None,
                pane: None,
                agent_label: canonical_agent.map(crate::detect::agent_label),
                terminal_title: None,
                terminal_title_stripped: None,
                canonical_agent,
                tokens: &tokens,
            },
            state_text,
        );
        let config = ClientShellConfig::from_config(&crate::config::Config::default());
        let rect = Rect::new(0, 0, width, rows.len() as u16);
        let mut buffer = Buffer::empty(rect);
        let row = AgentRow {
            pane_id: "pane_1".into(),
            status,
            focused: true,
            pinned: false,
            workspace_id: String::new(),
            folded: false,
            toggle_row: None,
            rows,
            footer: Vec::new(),
            repo: None,
            repo_heading_row: None,
        };
        render_agent_row(&mut buffer, rect, &row, &config);
        (buffer, config)
    }

    fn shell_tokens(count: Option<&str>) -> HashMap<String, String> {
        let mut tokens = HashMap::from([(
            "model".into(),
            "Claude/Opus 5.5 extended thinking context".into(),
        )]);
        if let Some(count) = count {
            tokens.insert("background_shells".into(), count.into());
        }
        tokens
    }

    #[test]
    fn background_shell_explicit_agent_label_yields_to_the_complete_state() {
        let (buffer, _) = rendered_with_tokens(
            r#"rows = [["$model"], ["state_text", "agent", "$background_shells"]]"#,
            24,
            "blocked",
            crate::api::schema::AgentStatus::Blocked,
            shell_tokens(Some("10 shells")),
            Some(crate::detect::Agent::Claude),
        );
        assert_eq!(row_text(&buffer, 1).trim(), "blocked · 10 shells");
    }

    #[test]
    fn background_shell_default_row_keeps_state_and_count_visible_and_distinct() {
        use crate::api::schema::AgentStatus;
        for (status, label, wanted) in [
            (
                AgentStatus::Working,
                "working · claude",
                "working · 2 shells",
            ),
            (AgentStatus::Done, "done · claude", "done · 2 shells"),
            (AgentStatus::Idle, "idle · claude", "idle · 2 shells"),
            (
                AgentStatus::Blocked,
                "blocked · claude",
                "blocked · 2 shells",
            ),
        ] {
            let (buffer, config) = rendered_with_tokens(
                r#"rows = [["$model"], [{ token = "state_text", dim = true }]]"#,
                24,
                label,
                status,
                shell_tokens(Some("2 shells")),
                Some(crate::detect::Agent::Claude),
            );
            assert_eq!(buffer.area.height, 2);
            let line = row_text(&buffer, 1);
            assert_eq!(line.trim(), wanted);
            let shell_x = line.chars().position(|ch| ch == '2').unwrap() as u16;
            assert_eq!(buffer[(shell_x, 1)].fg, config.palette.mauve);
            assert!(!buffer[(shell_x, 1)].modifier.contains(Modifier::DIM));
            assert_eq!(buffer[(3, 1)].fg, status_color(status, &config.palette));
            assert!(buffer[(3, 1)].modifier.contains(Modifier::DIM));
            assert!(!row_text(&buffer, 0).contains("shell"));
        }
    }

    #[test]
    fn background_shell_explicit_separate_row_is_single_colored_and_optional() {
        let rows = r##"rows = [["$model"], ["state_text"], [{ token = "$background_shells", fg = "#123456" }]]"##;
        let (buffer, _) = rendered_with_tokens(
            rows,
            40,
            "done · claude",
            crate::api::schema::AgentStatus::Done,
            shell_tokens(Some("1 shell")),
            Some(crate::detect::Agent::Claude),
        );
        assert_eq!(buffer.area.height, 3);
        assert_eq!(row_text(&buffer, 1).trim(), "done · claude");
        assert_eq!(row_text(&buffer, 2).trim(), "1 shell");
        assert_eq!(buffer[(3, 2)].fg, ratatui::style::Color::Rgb(18, 52, 86));
        let (empty, _) = rendered_with_tokens(
            rows,
            40,
            "idle · claude",
            crate::api::schema::AgentStatus::Idle,
            shell_tokens(None),
            Some(crate::detect::Agent::Claude),
        );
        assert_eq!(
            empty.area.height, 2,
            "an absent count must not leave a blank row"
        );
    }

    #[test]
    fn background_shell_agent_specific_and_hidden_config_override_automatic_placement() {
        let per_agent = r#"
rows = [["$model"], ["state_text"]]
[rows_by_agent]
claude = [["$model"], ["state_text"], ["$background_shells"]]
"#;
        let (buffer, config) = rendered_with_tokens(
            per_agent,
            40,
            "idle · claude",
            crate::api::schema::AgentStatus::Idle,
            shell_tokens(Some("1 shell")),
            Some(crate::detect::Agent::Claude),
        );
        assert_eq!(buffer.area.height, 3);
        assert_eq!(row_text(&buffer, 1).trim(), "idle · claude");
        assert_eq!(row_text(&buffer, 2).trim(), "1 shell");
        assert_eq!(buffer[(3, 2)].fg, config.palette.mauve);

        let hidden = r#"rows = [["$model"], ["state_text"], [{ token = "$background_shells", rules = [{ contains = "shell", hide = true }] }]]"#;
        let (buffer, _) = rendered_with_tokens(
            hidden,
            40,
            "idle · claude",
            crate::api::schema::AgentStatus::Idle,
            shell_tokens(Some("1 shell")),
            Some(crate::detect::Agent::Claude),
        );
        assert_eq!(buffer.area.height, 2);
        assert_eq!(row_text(&buffer, 1).trim(), "idle · claude");
    }

    #[test]
    fn blank_spacer_aligns_state_icon_with_the_second_row_indent() {
        let rows = r#"rows = [["$group"], ["$gap", "state_icon", "state_text"]]"#;
        let icon_x = |tokens: HashMap<String, String>| {
            let (buffer, config) = rendered_with_tokens(
                rows,
                30,
                "working",
                crate::api::schema::AgentStatus::Working,
                tokens,
                None,
            );
            let y = buffer.area.height - 1;
            let text = row_text(&buffer, y);
            assert!(text.ends_with("working"), "{text:?}");
            assert!(!text.contains('·'), "{text:?}");
            let x = (0..buffer.area.width)
                .find(|&x| !matches!(buffer[(x, y)].symbol(), " " | "\u{2800}"))
                .unwrap();
            let icon = status_icon(
                crate::api::schema::AgentStatus::Working,
                config.status_indicators,
            );
            assert_eq!(buffer[(x, y)].symbol(), icon.trim(), "{text:?}");
            x
        };
        let first = icon_x(HashMap::from([("group".into(), "crowd".into())]));
        let other = icon_x(HashMap::from([("gap".into(), "\u{2800}".into())]));
        assert_eq!((first, other), (3, 3));
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

    const GAP: &str = "\u{2800}";
    const SCHEDULER: &str = "character-message-scheduler";

    const SPACE_ROWS: &str = r##"
rows = [["$group"], ["$gap"], ["$model"], ["state_text"], ["$repo", "$worktree"]]
folded_rows = [["$group"], ["$gap", "state_icon", "state_text"]]
fold_toggle_token = "$group"
"##;

    /// A space labelled `prompts-box` with one agent pane per entry, each in
    /// that repository (`None`: outside any), carrying the tokens the server
    /// and a group-header plugin give it.
    fn repo_space(repos: &[Option<&str>]) -> (ClientShellSnapshot, ClientShellConfig) {
        let mut snapshot = crate::client::shell::tests::snapshot();
        snapshot.workspaces[0].label = "prompts-box".into();
        let pane = snapshot.panes.remove(0);
        for (index, repo) in repos.iter().enumerate() {
            let pane_id = format!("pane_{}", index + 1);
            let mut tokens = vec![("model".to_string(), "Claude".to_string())];
            tokens.push(if index == 0 {
                ("group".into(), "prompts-box".into())
            } else {
                ("gap".into(), GAP.into())
            });
            if let Some(repo) = repo.filter(|repo| *repo != "prompts-box") {
                tokens.push(("repo".into(), repo.into()));
                tokens.push(("worktree".into(), "wt:rippling-juggling-crystal".into()));
            }
            snapshot.panes.push(crate::protocol::ClientShellPane {
                pane_id: pane_id.clone(),
                git_context: repo.map(|repo| crate::protocol::ClientShellPaneGitContext {
                    repo_key: format!("/{repo}/.git"),
                    repo: repo.into(),
                    worktree: None,
                }),
                focused: false,
                ..pane.clone()
            });
            snapshot.agents.push(crate::protocol::ClientShellAgent {
                pane_id,
                workspace_id: "ws_1".into(),
                tab_id: "tab_1".into(),
                name: None,
                display_agent: None,
                agent: None,
                title: None,
                terminal_title: None,
                terminal_title_stripped: None,
                agent_status: crate::api::schema::AgentStatus::Idle,
                state_change_seq: index as u64,
                state_labels: Vec::new(),
                tokens,
                focused: false,
            });
        }
        let mut config = ClientShellConfig::from_config(&crate::config::Config::default());
        config.agents = toml::from_str(SPACE_ROWS).unwrap();
        (snapshot, config)
    }

    /// The panel's lines at the sidebar width the truncation was reported at.
    fn panel_lines(
        snapshot: &ClientShellSnapshot,
        config: &ClientShellConfig,
        folds: Option<&HashMap<String, bool>>,
    ) -> Vec<String> {
        agent_rows(snapshot, config, None, folds)
            .iter()
            .flat_map(|row| {
                let rect = Rect::new(0, 0, 47, row.rows.len() as u16);
                let mut buffer = Buffer::empty(rect);
                render_agent_row(&mut buffer, rect, row, config);
                (0..rect.height)
                    .map(|y| row_text(&buffer, y))
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    #[test]
    fn mixed_space_heads_the_other_repository_and_drops_its_repo_token() {
        let (snapshot, config) = repo_space(&[Some("prompts-box"), Some(SCHEDULER)]);
        assert_eq!(
            panel_lines(&snapshot, &config, None),
            [
                " prompts-box".to_string(),
                "   Claude".into(),
                "   idle".into(),
                format!(" {GAP}"),
                format!(" + {SCHEDULER}"),
                "   Claude".into(),
                "   idle".into(),
                // `$repo` left the row, so the worktree is no longer truncated.
                "   wt:rippling-juggling-crystal".into(),
            ]
        );
    }

    #[test]
    fn mixed_space_heads_the_title_repository_when_it_follows_another() {
        let (snapshot, config) = repo_space(&[Some(SCHEDULER), Some("prompts-box")]);
        let rows = agent_rows(&snapshot, &config, None, None);
        // The heading goes under the title, so the fold click target stays put.
        assert_eq!(rows[0].toggle_row, Some(0));
        assert_eq!(rows[0].repo_heading_row, Some(1));
        assert_eq!(
            panel_lines(&snapshot, &config, None),
            [
                " prompts-box".to_string(),
                format!(" + {SCHEDULER}"),
                "   Claude".into(),
                "   idle".into(),
                "   wt:rippling-juggling-crystal".into(),
                format!(" {GAP}"),
                " + prompts-box".into(),
                "   Claude".into(),
                "   idle".into(),
            ]
        );
    }

    #[test]
    fn mixed_space_heads_each_run_once_and_skips_panes_outside_a_repository() {
        let headings = |repos: &[Option<&str>]| {
            let (snapshot, config) = repo_space(repos);
            panel_lines(&snapshot, &config, None)
                .into_iter()
                .filter(|line| line.starts_with(" + "))
                .collect::<Vec<_>>()
        };
        assert_eq!(
            headings(&[Some(SCHEDULER), Some(SCHEDULER), Some("prompts-box")]),
            [format!(" + {SCHEDULER}"), " + prompts-box".to_string()]
        );
        assert_eq!(
            headings(&[Some("prompts-box"), None, Some(SCHEDULER)]),
            [format!(" + {SCHEDULER}")]
        );
        // The card outside any repository does not hide the change behind it.
        assert_eq!(
            headings(&[Some(SCHEDULER), None, Some("prompts-box")]),
            [format!(" + {SCHEDULER}"), " + prompts-box".to_string()]
        );
    }

    #[test]
    fn mixed_space_run_does_not_cross_from_pinned_cards_into_the_list() {
        let (mut snapshot, mut config) =
            repo_space(&[Some(SCHEDULER), Some(SCHEDULER), Some("prompts-box")]);
        snapshot.agents[0].name = Some("pin".into());
        config.agents.pinned = vec!["pin".into()];
        let headings = panel_lines(&snapshot, &config, None)
            .into_iter()
            .filter(|line| line.starts_with(" + "))
            .collect::<Vec<_>>();
        assert_eq!(
            headings,
            [
                format!(" + {SCHEDULER}"),
                format!(" + {SCHEDULER}"),
                " + prompts-box".to_string()
            ]
        );
    }

    #[test]
    fn mixed_space_footer_keeps_the_repo_token_the_card_rows_drop() {
        let (snapshot, mut config) = repo_space(&[Some("prompts-box"), Some(SCHEDULER)]);
        config.agents = toml::from_str(&format!("{SPACE_ROWS}footer = [[\"$repo\"]]")).unwrap();
        let rows = agent_rows(&snapshot, &config, None, None);
        assert_eq!(rows[1].footer.len(), 1);
        assert!(!rows[1].footer[0].is_empty());
        let lines = panel_lines(&snapshot, &config, None);
        assert!(
            lines.contains(&"   wt:rippling-juggling-crystal".to_string()),
            "{lines:?}"
        );
    }

    #[test]
    fn single_repository_space_keeps_its_repo_token_and_gets_no_heading() {
        let (mut snapshot, config) = repo_space(&[Some(".prompts-box"), Some(".prompts-box")]);
        // A plain shell in another repository does not make the agents mixed.
        let mut shell = snapshot.panes[0].clone();
        shell.pane_id = "pane_shell".into();
        shell.git_context.as_mut().unwrap().repo_key = "/other/.git".into();
        snapshot.panes.push(shell);
        let lines = panel_lines(&snapshot, &config, None);
        assert!(
            !lines.iter().any(|line| line.starts_with(" + ")),
            "{lines:?}"
        );
        assert_eq!(
            lines
                .iter()
                .filter(|line| *line == "   .prompts-box · wt:rippling-juggling-crystal")
                .count(),
            2,
            "{lines:?}"
        );
    }

    #[test]
    fn folded_mixed_space_keeps_the_heading_and_the_card_line_indent() {
        let (snapshot, config) = repo_space(&[Some("prompts-box"), Some(SCHEDULER)]);
        let folds = HashMap::from([("ws_1".to_string(), true)]);
        let lines = panel_lines(&snapshot, &config, Some(&folds));
        assert_eq!(lines.len(), 4, "{lines:?}");
        assert_eq!(lines[0], " prompts-box");
        assert_eq!(lines[2], format!(" + {SCHEDULER}"));
        // Same line as without the heading: indent 1, spacer, icon, state.
        assert!(lines[3].starts_with(&format!(" {GAP} ")), "{lines:?}");
        assert!(lines[3].ends_with(" idle"), "{lines:?}");
    }
}
