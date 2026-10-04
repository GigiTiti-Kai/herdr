use super::*;

pub(super) struct WorkspaceTabRow {
    pub(super) tab_id: String,
    pub(super) label: String,
    pub(super) detail: String,
    pub(super) position: usize,
    pub(super) focused: bool,
}

/// Build only the active workspace's overview, once per chrome composition.
/// All inputs are cached snapshot data; hidden panes need no presentation work.
pub(super) fn rows(snapshot: &ClientShellSnapshot) -> Vec<WorkspaceTabRow> {
    let Some(workspace_id) = snapshot.focused_workspace_id.as_deref() else {
        return Vec::new();
    };
    let tabs: Vec<_> = snapshot
        .tabs
        .iter()
        .filter(|tab| tab.workspace_id == workspace_id)
        .collect();
    if tabs.len() < 2 {
        return Vec::new();
    }
    let mut panes_by_tab = std::collections::HashMap::<_, Vec<_>>::new();
    for pane in &snapshot.panes {
        if pane.workspace_id == workspace_id {
            panes_by_tab
                .entry(pane.tab_id.as_str())
                .or_default()
                .push(pane);
        }
    }
    let mut agents_by_tab = std::collections::HashMap::<_, Vec<_>>::new();
    for agent in &snapshot.agents {
        if agent.workspace_id == workspace_id {
            agents_by_tab
                .entry(agent.tab_id.as_str())
                .or_default()
                .push(agent);
        }
    }
    tabs.into_iter()
        .enumerate()
        .map(|(index, tab)| {
            let panes = panes_by_tab
                .get(tab.tab_id.as_str())
                .map(Vec::as_slice)
                .unwrap_or(&[]);
            let mut repos = Vec::new();
            let mut worktrees = Vec::new();
            for pane in panes {
                if let Some(git) = &pane.git_context {
                    if !repos.iter().any(|(key, _)| *key == git.repo_key.as_str()) {
                        repos.push((git.repo_key.as_str(), git.repo.as_str()));
                    }
                    if let Some(name) = git.worktree.as_deref() {
                        if !worktrees.contains(&name) {
                            worktrees.push(name);
                        }
                    }
                }
            }
            let project = repos.first().map(|(_, name)| {
                if repos.len() > 1 {
                    format!("{name} +{}", repos.len() - 1)
                } else {
                    (*name).to_owned()
                }
            });
            let label = if tab.custom_label {
                tab.label.clone()
            } else if let Some(project) = &project {
                project.clone()
            } else if tab.label.parse::<usize>().is_err() {
                tab.label.clone()
            } else {
                panes
                    .first()
                    .and_then(|pane| {
                        pane.label
                            .as_deref()
                            .or(pane.foreground_cwd.as_deref())
                            .or(pane.cwd.as_deref())
                    })
                    .map(|name| {
                        name.trim_end_matches(['/', '\\'])
                            .rsplit(['/', '\\'])
                            .next()
                            .unwrap_or(name)
                    })
                    .filter(|name| !name.is_empty())
                    .unwrap_or("terminal")
                    .to_owned()
            };
            let detail = if let Some(project) = project.filter(|project| *project != label) {
                project
            } else if let Some(worktree) = worktrees.first() {
                if worktrees.len() > 1 {
                    format!("wt:{worktree} +{}", worktrees.len() - 1)
                } else {
                    format!("wt:{worktree}")
                }
            } else {
                let mut names = Vec::new();
                for agent in agents_by_tab.get(tab.tab_id.as_str()).into_iter().flatten() {
                    if let Some(name) = agent.display_agent.as_deref().or(agent.agent.as_deref()) {
                        if !names.contains(&name) {
                            names.push(name);
                        }
                    }
                }
                if names.is_empty() && tab.label != label && tab.label.parse::<usize>().is_err() {
                    tab.label.clone()
                } else {
                    names.join("/")
                }
            };
            WorkspaceTabRow {
                tab_id: tab.tab_id.clone(),
                label,
                detail,
                position: index + 1,
                focused: snapshot.focused_tab_id.as_deref() == Some(tab.tab_id.as_str()),
            }
        })
        .collect()
}

pub(super) fn render(
    buffer: &mut Buffer,
    rect: Rect,
    row: &WorkspaceTabRow,
    palette: &Palette,
    hits: &mut ShellHitMap,
) {
    let style = Style::default().fg(if row.focused {
        palette.text
    } else {
        palette.overlay1
    });
    if row.focused {
        buffer.set_style(rect, Style::default().bg(palette.active_row_bg));
    }
    let prefix = format!(" {} {} ", if row.focused { "▸" } else { " " }, row.position);
    let x = super::render::put_segment(
        buffer,
        rect.x,
        rect.y,
        rect.right(),
        &prefix,
        style.fg(if row.focused {
            palette.accent
        } else {
            palette.overlay0
        }),
    );
    let width = rect.right().saturating_sub(x) as usize;
    let label = crate::ui::truncate_end(&row.label, width);
    let x = super::render::put_segment(buffer, x, rect.y, rect.right(), &label, style);
    let remaining = rect.right().saturating_sub(x) as usize;
    if !row.detail.is_empty() && remaining >= 6 {
        let detail = crate::ui::truncate_end(&format!(" · {}", row.detail), remaining);
        super::render::put_text(
            buffer,
            x,
            rect.y,
            remaining as u16,
            &detail,
            Style::default().fg(palette.overlay0),
        );
    }
    hits.workspace_tabs.push((rect, row.tab_id.clone()));
}
