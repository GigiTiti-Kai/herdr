use std::path::PathBuf;

use crate::app::App;
use crate::layout::PaneId;
use crate::pane::PaneLaunchEnv;
use crate::popup_size::PopupSize;
use crate::terminal::{TerminalId, TerminalRuntime, TerminalState};

/// Public id of the session-wide dock pane.
pub(crate) const DOCK_PANE_ID: &str = "dock";

/// Narrowest dock column and tab area a dock drag may leave.
const MIN_DOCK_COLS: u16 = 20;
const MIN_TAB_COLS: u16 = 20;

/// Dock width in columns, keeping both the dock and the tab area at least 20 columns
/// when the screen allows it. Narrower screens fall back to `dock_column_rects`.
pub(crate) fn clamp_dock_width(requested: u16, area_width: u16) -> u16 {
    requested
        .min(area_width.saturating_sub(MIN_TAB_COLS))
        .max(MIN_DOCK_COLS)
}

impl App {
    /// The area the dock column is cut from; the headless size until a view exists.
    fn dock_terminal_area(&self) -> ratatui::layout::Rect {
        let area = self.state.view.terminal_area;
        if area.width >= 4 && area.height >= 4 {
            area
        } else {
            let (cols, rows) = self.state.headless_size;
            ratatui::layout::Rect::new(0, 0, cols, rows)
        }
    }

    pub(crate) fn handle_dock_set_width(
        &mut self,
        id: String,
        params: crate::api::schema::DockSetWidthParams,
    ) -> String {
        use crate::app::api::responses::{encode_error, encode_success};

        let width = clamp_dock_width(params.width, self.dock_terminal_area().width);
        let Some(dock) = self.state.dock_pane.as_mut() else {
            return encode_error(id, "dock_not_found", "the dock pane is not open");
        };
        dock.width = Some(PopupSize::Cells(width));
        self.state.dock_last_width = dock.width;
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        encode_success(
            id,
            crate::api::schema::ResponseResult::DockWidthSet { width },
        )
    }

    pub(crate) fn close_dock_pane(&mut self) -> bool {
        let Some(dock) = self.state.dock_pane.take() else {
            return false;
        };
        self.state.dock_focused = false;
        self.state
            .direct_attach_resize_locks
            .remove(&dock.terminal_id);
        self.state.terminals.remove(&dock.terminal_id);
        self.shutdown_terminal_runtime(dock.terminal_id);
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        true
    }

    /// Public pane info for the dock. It belongs to no workspace or tab.
    pub(crate) fn dock_pane_info(&self) -> Option<crate::api::schema::PaneInfo> {
        let dock = self.state.dock_pane.as_ref()?;
        let terminal = self.state.terminals.get(&dock.terminal_id)?;
        Some(crate::api::schema::PaneInfo {
            pane_id: DOCK_PANE_ID.to_string(),
            terminal_id: terminal.id.to_string(),
            workspace_id: String::new(),
            tab_id: String::new(),
            focused: self.state.dock_focused,
            cwd: Some(terminal.cwd.display().to_string()),
            foreground_cwd: None,
            restore_error: None,
            label: terminal.manual_label.clone(),
            agent: None,
            title: None,
            terminal_title: terminal.terminal_title.clone(),
            terminal_title_stripped: terminal.terminal_title_stripped(),
            display_agent: None,
            agent_status: crate::api::schema::AgentStatus::Unknown,
            state_labels: Default::default(),
            tokens: Default::default(),
            agent_session: None,
            scroll: self
                .terminal_runtimes
                .get(&dock.terminal_id)
                .and_then(|runtime| runtime.scroll_metrics())
                .map(|metrics| crate::api::schema::PaneScrollInfo {
                    offset_from_bottom: metrics.offset_from_bottom as u64,
                    max_offset_from_bottom: metrics.max_offset_from_bottom as u64,
                    viewport_rows: metrics.viewport_rows as u64,
                }),
            revision: terminal.revision,
        })
    }

    pub(crate) fn focus_dock_pane(&mut self) -> Option<crate::api::schema::PaneInfo> {
        self.state.dock_pane.as_ref()?;
        if !self.state.dock_focused {
            self.state.dock_focused = true;
            self.render_dirty.request_generic();
            self.render_notify.notify_one();
        }
        self.state.mode = crate::app::Mode::Terminal;
        self.dock_pane_info()
    }

    pub(crate) fn spawn_dock_argv_command(
        &mut self,
        argv: &[String],
        cwd: Option<PathBuf>,
        extra_env: Vec<(String, String)>,
        width: Option<PopupSize>,
    ) -> std::io::Result<()> {
        if self.state.dock_pane.is_some() {
            return Err(std::io::Error::other("dock already open"));
        }
        let cwd = cwd.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| "/".into()));
        let Some((_, inner)) = crate::ui::dock_column_rects(width, self.dock_terminal_area())
        else {
            return Err(std::io::Error::other("terminal area too small for dock"));
        };
        let pane_id = PaneId::alloc();
        let terminal_id = TerminalId::alloc();
        let launch_env =
            PaneLaunchEnv::from_extra(extra_env).with_pane_id_only(DOCK_PANE_ID.to_string());
        let runtime = TerminalRuntime::spawn_argv_command(
            pane_id,
            inner.height,
            inner.width,
            cwd.clone(),
            argv,
            &launch_env,
            crate::pane::AgentDetection::Disabled,
            self.state.pane_scrollback_limit_bytes,
            self.state.host_terminal_theme,
            self.state.host_terminal_appearance,
            self.event_tx.clone(),
            self.render_notify.clone(),
            self.render_dirty.clone(),
        )?;
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(
            terminal_id.clone(),
            TerminalState::new(terminal_id.clone(), cwd).with_launch_argv(argv.to_vec()),
        );
        self.state.dock_pane = Some(crate::app::state::DockPaneState {
            pane_id,
            terminal_id,
            width,
            plugin: None,
        });
        self.render_dirty.request_generic();
        self.render_notify.notify_one();
        Ok(())
    }
}

#[cfg(test)]
impl App {
    pub(crate) fn install_test_dock_runtime(
        &mut self,
        runtime: TerminalRuntime,
        width: Option<PopupSize>,
    ) -> (PaneId, TerminalId) {
        let pane_id = PaneId::alloc();
        let terminal_id = TerminalId::alloc();
        self.terminal_runtimes.insert(terminal_id.clone(), runtime);
        self.state.terminals.insert(
            terminal_id.clone(),
            TerminalState::new(terminal_id.clone(), PathBuf::from("/dock")),
        );
        self.state.dock_pane = Some(crate::app::state::DockPaneState {
            pane_id,
            terminal_id: terminal_id.clone(),
            width,
            plugin: None,
        });
        (pane_id, terminal_id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::schema::{DockSetWidthParams, ErrorResponse, ResponseResult, SuccessResponse};

    fn test_app() -> App {
        let (_api_tx, api_rx) = tokio::sync::mpsc::unbounded_channel();
        let mut app = App::new(
            &crate::config::Config::default(),
            crate::app::AppPolicy::TEST,
            None,
            api_rx,
            crate::api::EventHub::default(),
        );
        app.state.view.terminal_area = ratatui::layout::Rect::new(0, 0, 200, 40);
        app
    }

    #[test]
    fn clamp_dock_width_keeps_both_sides_at_least_twenty() {
        assert_eq!(clamp_dock_width(5, 200), 20);
        assert_eq!(clamp_dock_width(190, 200), 180);
        assert_eq!(clamp_dock_width(60, 200), 60);
    }

    #[tokio::test]
    async fn dock_set_width_sets_cells_and_remembers() {
        let mut app = test_app();
        app.install_test_dock_runtime(
            TerminalRuntime::test_with_screen_bytes(60, 40, b"dock"),
            Some(PopupSize::Percent(30)),
        );

        let response = app.handle_dock_set_width("w".into(), DockSetWidthParams { width: 70 });

        let success: SuccessResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(success.result, ResponseResult::DockWidthSet { width: 70 });
        assert_eq!(
            app.state.dock_pane.as_ref().unwrap().width,
            Some(PopupSize::Cells(70))
        );
        assert_eq!(app.state.dock_last_width, Some(PopupSize::Cells(70)));
        for (_, runtime) in app.terminal_runtimes.drain() {
            runtime.shutdown();
        }
    }

    #[test]
    fn dock_set_width_without_dock_is_not_found() {
        let mut app = test_app();

        let response = app.handle_dock_set_width("w".into(), DockSetWidthParams { width: 70 });

        let error: ErrorResponse = serde_json::from_str(&response).unwrap();
        assert_eq!(error.error.code, "dock_not_found");
        assert_eq!(app.state.dock_last_width, None);
    }

    #[test]
    fn remembered_dock_width_survives_a_narrow_terminal() {
        let area = ratatui::layout::Rect::new(0, 0, 60, 20);
        let (outer, _) = crate::ui::dock_column_rects(Some(PopupSize::Cells(70)), area).unwrap();
        assert!(outer.x >= 1, "the tab area keeps at least one column");
        assert_eq!(outer.right(), area.right());
    }
}
