use super::render::put_text;
use super::*;

pub(super) fn render_collapsed(
    buffer: &mut Buffer,
    area: Rect,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    hits: &mut ShellHitMap,
) {
    let rows = agent_rows(
        endpoints,
        active_endpoint_id,
        config,
        &config.agents.pinned,
        None,
    );
    for (index, row) in rows.into_iter().take(area.height as usize).enumerate() {
        let rect = Rect::new(area.x, area.y + index as u16, area.width, 1);
        if row.agent.focused {
            buffer.set_style(rect, Style::default().bg(config.palette.active_row_bg));
        }
        let initial = row.machine_label.chars().next().unwrap_or('?');
        put_text(
            buffer,
            rect.x,
            rect.y,
            rect.width,
            &format!(
                "{initial}{}",
                status_icon(row.agent.status, config.status_indicators)
            ),
            Style::default()
                .fg(if row.stale {
                    config.palette.overlay0
                } else {
                    status_color(row.agent.status, &config.palette)
                })
                .add_modifier(if row.stale {
                    Modifier::DIM
                } else {
                    Modifier::empty()
                }),
        );
        hits.endpoint_agents
            .push((rect, row.endpoint_id, row.agent.pane_id));
    }
}

pub(super) fn render_expanded(
    buffer: &mut Buffer,
    area: Rect,
    agent_view_label: Option<&str>,
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    agent_scroll: &mut usize,
    folds: super::agent_sidebar::AgentFolds<'_>,
    hits: &mut ShellHitMap,
) {
    if !super::agent_sidebar::render_agent_panel_header(
        buffer,
        area,
        agent_view_label,
        config,
        hits,
    ) {
        return;
    }
    // The expanded multi-machine list ignores `pinned`: only the rail and mobile reorder.
    let rows = agent_rows(endpoints, active_endpoint_id, config, &[], Some(folds));
    let footer = if config.agents.footer_in_bottom_bar() {
        Vec::new()
    } else {
        footer_from_rows(endpoints, config, &rows)
    };
    super::agent_sidebar::render_agent_list(
        buffer,
        area,
        &rows,
        &footer,
        agent_view_label.map(|_| " no matching agents"),
        config,
        agent_scroll,
        hits,
        |row| row.agent.rows.len(),
        |buffer, rect, row, hits| {
            super::agent_sidebar::render_agent_row(buffer, rect, &row.agent, config);
            if row.stale {
                buffer.set_style(
                    rect,
                    Style::default()
                        .fg(config.palette.overlay0)
                        .add_modifier(Modifier::DIM),
                );
            }
            hits.endpoint_agents
                .push((rect, row.endpoint_id.clone(), row.agent.pane_id.clone()));
            super::agent_sidebar::push_fold_hit(hits, rect, &row.endpoint_id, &row.agent);
        },
    );
}

impl ClientShellState {
    pub(super) fn reveal_endpoint_agent(
        &mut self,
        endpoint_id: &ClientEndpointId,
        pane_id: &str,
        body_height: u16,
    ) {
        if body_height == 0 {
            return;
        }
        let rows = agent_rows(
            &self.endpoints,
            &self.active_endpoint_id,
            &self.config,
            &[],
            Some(super::agent_sidebar::AgentFolds {
                local: &self.agent_folds,
                remote: &self.remote_agent_folds,
            }),
        );
        let Some(target) = rows
            .iter()
            .position(|row| &row.endpoint_id == endpoint_id && row.agent.pane_id == pane_id)
        else {
            return;
        };
        let heights = rows
            .iter()
            .map(|row| row.agent.rows.len().max(1).min(u16::MAX as usize) as u16)
            .collect::<Vec<_>>();
        let mut gaps = vec![self.config.agents.row_gap; rows.len()];
        if let Some(last) = gaps.last_mut() {
            *last = 0;
        }
        self.agent_scroll = super::scroll::list_scroll_start_to_reveal(
            &heights,
            &gaps,
            body_height,
            self.agent_scroll,
            target,
        );
    }
}

struct EndpointAgentRow {
    endpoint_id: ClientEndpointId,
    machine_label: String,
    stale: bool,
    agent: super::agent_sidebar::AgentRow,
}

fn footer_from_rows(
    endpoints: &[ClientShellEndpoint],
    config: &ClientShellConfig,
    rows: &[EndpointAgentRow],
) -> Vec<Vec<crate::ui::ResolvedToken>> {
    crate::ui::sidebar_agent_footer_rows(
        &config.agents.footer,
        // Stale endpoints hold frozen values that would look live in the footer.
        rows.iter()
            .filter(|row| !row.stale)
            .map(|row| row.agent.footer.as_slice()),
        super::aggregate_navigation::cached_endpoint_snapshots(endpoints)
            .filter(|endpoint| !endpoint.stale())
            .flat_map(|endpoint| &endpoint.snapshot.workspaces)
            .map(|workspace| workspace.tokens.as_slice()),
    )
}

/// The footer for the bottom band, picked the way the expanded list picks it.
pub(super) fn bar_footer_rows(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
) -> Vec<Vec<crate::ui::ResolvedToken>> {
    let rows = agent_rows(endpoints, active_endpoint_id, config, &[], None);
    footer_from_rows(endpoints, config, &rows)
}

fn agent_rows(
    endpoints: &[ClientShellEndpoint],
    active_endpoint_id: &ClientEndpointId,
    config: &ClientShellConfig,
    pinned: &[String],
    folds: Option<super::agent_sidebar::AgentFolds<'_>>,
) -> Vec<EndpointAgentRow> {
    let mut rendered_rows = endpoints
        .iter()
        .filter_map(|endpoint| {
            endpoint.snapshot.as_deref().map(|snapshot| {
                let mixed = super::agent_sidebar::mixed_repo_spaces(snapshot);
                snapshot
                    .agents
                    .iter()
                    .filter_map(|agent| {
                        super::agent_sidebar::agent_row(
                            snapshot,
                            &agent.pane_id,
                            config,
                            Some(&endpoint.label),
                            folds.and_then(|folds| folds.for_endpoint(&endpoint.endpoint_id)),
                            &mixed,
                        )
                    })
                    .map(|agent| ((endpoint.endpoint_id.clone(), agent.pane_id.clone()), agent))
                    .collect::<Vec<_>>()
            })
        })
        .flatten()
        .collect::<HashMap<_, _>>();

    let mut rows = super::aggregate_navigation::aggregate_agent_rows(
        endpoints,
        active_endpoint_id,
        config.agent_panel_sort,
        pinned,
    )
    .into_iter()
    .filter_map(|row| {
        let key = (row.endpoint.endpoint_id.clone(), row.agent.pane_id.clone());
        let mut agent = rendered_rows.remove(&key)?;
        agent.focused &= row.endpoint.endpoint_id == active_endpoint_id;
        Some(EndpointAgentRow {
            endpoint_id: row.endpoint.endpoint_id.clone(),
            machine_label: row.endpoint.label.to_owned(),
            stale: row.endpoint.stale(),
            agent,
        })
    })
    .collect::<Vec<_>>();
    super::agent_sidebar::insert_repo_headings(
        rows.iter_mut()
            .map(|row| (&row.endpoint_id, &mut row.agent)),
    );
    rows
}
