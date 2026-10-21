use super::*;

const DOCK: &str = crate::app::dock::DOCK_PANE_ID;

fn dock_surface_pane(
    surface: &crate::protocol::PaneSurfaceFrame,
) -> Option<&crate::protocol::PaneSurfacePane> {
    surface.panes.iter().find(|pane| pane.pane_id == DOCK)
}

fn shell_request(
    server: &mut HeadlessServer,
    client_id: u64,
    method: crate::api::schema::Method,
) -> serde_json::Value {
    let (respond_to, response_rx) = std::sync::mpsc::channel();
    server.handle_client_shell_api_request(
        client_id,
        crate::api::ApiRequestMessage {
            request: crate::api::schema::Request {
                id: "dock-test".into(),
                method,
            },
            respond_to,
            response_write_complete: None,
        },
    );
    serde_json::from_str(&response_rx.recv().expect("api response")).unwrap()
}

fn focus_dock(server: &mut HeadlessServer, client_id: u64) {
    let response = shell_request(
        server,
        client_id,
        crate::api::schema::Method::PaneFocus(crate::api::schema::PaneTarget {
            pane_id: DOCK.into(),
        }),
    );
    assert_eq!(response["result"]["pane"]["pane_id"], DOCK, "{response}");
}

/// The newest projection the client has been sent, skipping other control messages.
fn latest_snapshot(
    receiver: &std::sync::mpsc::Receiver<Vec<u8>>,
) -> Box<protocol::ClientShellSnapshot> {
    let mut snapshot = None;
    while let Ok(bytes) = receiver.recv_timeout(Duration::from_millis(100)) {
        if let ServerMessage::EndpointControl { kind, data } = read_server_message(bytes) {
            if kind == protocol::endpoint::ENDPOINT_SNAPSHOT_KIND {
                snapshot = Some(serde_json::from_str(&data).unwrap());
            }
        }
    }
    snapshot.expect("a projection was sent")
}

fn text_key(text: &str) -> crate::protocol::ClientPaneInputEvent {
    crate::protocol::ClientPaneInputEvent::TextCommit(text.into())
}

#[tokio::test]
async fn dock_is_visible_on_every_tab_and_survives_tab_switch() {
    let mut server = test_headless_server();
    let mut first = crate::workspace::Workspace::test_new("dock-first");
    let first_pane = first.tabs[0].root_pane;
    let second_tab = first.test_add_tab(Some("second"));
    first.insert_test_runtime(
        first_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"FIRST"),
    );
    let second_tab_pane = first.tabs[second_tab].root_pane;
    first.insert_test_runtime(
        second_tab_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"SECOND"),
    );
    let mut other = crate::workspace::Workspace::test_new("dock-other");
    let other_pane = other.tabs[0].root_pane;
    other.insert_test_runtime(
        other_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"OTHER"),
    );
    server.app.state.workspaces = vec![first, other];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let first_tab_id = server.app.public_tab_id(0, 0).unwrap();
    let second_tab_id = server.app.public_tab_id(0, second_tab).unwrap();
    let other_tab_id = server.app.public_tab_id(1, 0).unwrap();
    let first_pane_id = server.app.public_pane_id(0, first_pane).unwrap();
    let tiled_size = |server: &HeadlessServer| {
        server
            .app
            .state
            .runtime_for_pane_in_workspace(&server.app.terminal_runtimes, 0, first_pane)
            .unwrap()
            .current_size()
    };

    let (control, render) = connect_matching_test_shell(&mut server, 31);
    let _ = control.recv().expect("snapshot");
    server.render_and_stream();
    let full = recv_pane_surface(&render, "surface without dock");
    assert!(dock_surface_pane(&full).is_none());
    let full_size = tiled_size(&server);

    let (dock_runtime, _dock_input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            20, 23, 0, b"DOCK", 4,
        );
    let (dock_pane_id, dock_terminal_id) = server.app.install_test_dock_runtime(
        dock_runtime,
        Some(crate::popup_size::PopupSize::Percent(30)),
    );
    server.render_and_stream();
    let docked = recv_pane_surface(&render, "surface with dock");
    let dock = dock_surface_pane(&docked).expect("dock in surface");
    assert_eq!(
        (dock.rect.x, dock.rect.width, dock.rect.height),
        (56, 24, 23)
    );
    assert!(!dock.focused, "the dock does not take focus when it opens");
    let tiled = docked
        .panes
        .iter()
        .find(|pane| pane.pane_id == first_pane_id)
        .unwrap();
    assert!(tiled.focused);
    assert!(tiled.rect.x + tiled.rect.width <= 56);
    assert!(frame_text(&docked.frame).contains("DOCK"));
    assert!(frame_text(&docked.frame).contains("FIRST"));
    assert!(
        tiled_size(&server).1 <= 56,
        "tiled PTY shrank to the tab area"
    );
    assert_eq!(
        server
            .app
            .terminal_runtimes
            .get(&dock_terminal_id)
            .unwrap()
            .current_size(),
        (23, 23)
    );

    // Output in the dock repaints only the dock.
    let tiled_revision = tiled.content_revision;
    let dock_revision = dock.content_revision;
    server
        .app
        .terminal_runtimes
        .get(&dock_terminal_id)
        .unwrap()
        .test_process_pty_bytes(b" MORE");
    server.sync_immediate_pty_sources();
    assert!(server.pty_sources_visible_to_any_render_target(&HashSet::from([dock_pane_id])));
    assert!(!server.render_retained_pane_surface_and_stream(&HashSet::from([dock_pane_id])));
    server.render_and_stream();
    let repainted = recv_pane_surface(&render, "surface after dock output");
    let dock = dock_surface_pane(&repainted).unwrap();
    assert_ne!(dock.content_revision, dock_revision);
    assert!(frame_text(&repainted.frame).contains("DOCK MORE"));
    assert_eq!(
        repainted
            .panes
            .iter()
            .find(|pane| pane.pane_id == first_pane_id)
            .unwrap()
            .content_revision,
        tiled_revision
    );

    assert!(server.focus_shell_client_on_tab(31, &second_tab_id));
    server.render_and_stream();
    let second = recv_pane_surface(&render, "second tab surface");
    assert!(dock_surface_pane(&second).is_some());
    assert!(frame_text(&second.frame).contains("SECOND"));

    assert!(server.focus_shell_client_on_tab(31, &other_tab_id));
    server.render_and_stream();
    let other = recv_pane_surface(&render, "other workspace surface");
    assert!(dock_surface_pane(&other).is_some());
    assert!(frame_text(&other.frame).contains("OTHER"));

    assert!(server.focus_shell_client_on_tab(31, &first_tab_id));
    server
        .app
        .handle_internal_event(crate::events::AppEvent::PaneDied {
            pane_id: dock_pane_id,
            exit_reason: crate::platform::ChildExitReason::Exited,
        });
    assert!(server.app.state.dock_pane.is_none());
    server.render_and_stream();
    let closed = recv_pane_surface(&render, "surface after dock exit");
    assert!(dock_surface_pane(&closed).is_none());
    let tiled = closed
        .panes
        .iter()
        .find(|pane| pane.pane_id == first_pane_id)
        .unwrap();
    assert_eq!(tiled.rect.x + tiled.rect.width, 80);
    assert_eq!(tiled_size(&server), full_size, "tiled PTY grew back");
    server.app.state.assert_invariants_for_test();

    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn dock_receives_keys_only_while_focused() {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("dock-keys");
    let left = workspace.tabs[0].root_pane;
    let right = workspace.test_split(ratatui::layout::Direction::Horizontal);
    let (left_runtime, mut left_input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            40, 23, 0, b"LEFT", 4,
        );
    workspace.insert_test_runtime(left, left_runtime);
    let (right_runtime, mut right_input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            40, 23, 0, b"RIGHT", 4,
        );
    workspace.insert_test_runtime(right, right_runtime);
    server.app.state.workspaces = vec![workspace];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let left_id = server.app.public_pane_id(0, left).unwrap();
    let right_id = server.app.public_pane_id(0, right).unwrap();
    server.app.state.focus_pane_in_workspace(0, left);

    let (control, render) = connect_matching_test_shell(&mut server, 41);
    let _ = control.recv().expect("snapshot");
    let (dock_runtime, mut dock_input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            20, 23, 0, b"DOCK", 4,
        );
    server.app.install_test_dock_runtime(dock_runtime, None);

    server.handle_server_event(ServerEvent::ClientShellPaneInput {
        client_id: 41,
        pane_id: left_id.clone(),
        events: vec![text_key("tiled")],
    });
    assert_eq!(
        left_input.try_recv().expect("tiled input"),
        Bytes::from_static(b"tiled")
    );
    assert!(dock_input.try_recv().is_err());

    // Right from the right-most pane enters the dock; Left leaves it.
    let direction = |pane_id: &str, direction| {
        crate::api::schema::Method::PaneFocusDirection(
            crate::api::schema::PaneFocusDirectionParams {
                pane_id: Some(pane_id.to_owned()),
                direction,
            },
        )
    };
    let moved = shell_request(
        &mut server,
        41,
        direction(&left_id, crate::api::schema::PaneDirection::Right),
    );
    assert_eq!(
        moved["result"]["focus"]["focused_pane_id"],
        right_id.as_str()
    );
    assert!(!server.app.state.dock_focused);
    let entered = shell_request(
        &mut server,
        41,
        direction(&right_id, crate::api::schema::PaneDirection::Right),
    );
    assert_eq!(entered["result"]["focus"]["changed"], true, "{entered}");
    assert_eq!(entered["result"]["focus"]["focused_pane_id"], DOCK);
    server.render_and_stream();
    assert_eq!(
        latest_snapshot(&control).focused_pane_id.as_deref(),
        Some(DOCK)
    );
    let surface = recv_pane_surface(&render, "dock focused surface");
    assert!(dock_surface_pane(&surface).unwrap().focused);
    assert!(surface
        .panes
        .iter()
        .filter(|pane| pane.pane_id != DOCK)
        .all(|pane| !pane.focused));

    // Tiled output cannot take the retained path: only a full render draws the dock cursor.
    write_shared_test_pane(&mut server, right, b" more");
    assert!(!server.render_retained_pane_surface_and_stream(&HashSet::from([right])));

    server.handle_server_event(ServerEvent::ClientShellPaneInput {
        client_id: 41,
        pane_id: DOCK.into(),
        events: vec![text_key("board")],
    });
    assert_eq!(
        dock_input.try_recv().expect("dock input"),
        Bytes::from_static(b"board")
    );
    assert!(right_input.try_recv().is_err());

    let up = shell_request(
        &mut server,
        41,
        direction(DOCK, crate::api::schema::PaneDirection::Up),
    );
    assert_eq!(up["result"]["focus"]["changed"], false);
    assert!(server.app.state.dock_focused);
    let left_out = shell_request(
        &mut server,
        41,
        direction(DOCK, crate::api::schema::PaneDirection::Left),
    );
    assert_eq!(left_out["result"]["focus"]["source_pane_id"], DOCK);
    assert_eq!(
        left_out["result"]["focus"]["focused_pane_id"],
        right_id.as_str()
    );
    assert!(!server.app.state.dock_focused);
    server.render_and_stream();
    assert_eq!(
        latest_snapshot(&control).focused_pane_id.as_deref(),
        Some(right_id.as_str())
    );

    // `herdr pane focus --direction` outside a pane omits the pane id: it means the focused pane.
    let omitted = |direction| {
        crate::api::schema::Method::PaneFocusDirection(
            crate::api::schema::PaneFocusDirectionParams {
                pane_id: None,
                direction,
            },
        )
    };
    let entered = shell_request(
        &mut server,
        41,
        omitted(crate::api::schema::PaneDirection::Right),
    );
    assert_eq!(
        entered["result"]["focus"]["focused_pane_id"], DOCK,
        "{entered}"
    );
    let up = shell_request(
        &mut server,
        41,
        omitted(crate::api::schema::PaneDirection::Up),
    );
    assert_eq!(up["result"]["focus"]["changed"], false, "{up}");
    assert_eq!(up["result"]["focus"]["source_pane_id"], DOCK);
    let left_out = shell_request(
        &mut server,
        41,
        omitted(crate::api::schema::PaneDirection::Left),
    );
    assert_eq!(left_out["result"]["focus"]["changed"], true, "{left_out}");
    assert_eq!(left_out["result"]["focus"]["source_pane_id"], DOCK);
    assert_eq!(
        left_out["result"]["focus"]["focused_pane_id"],
        right_id.as_str()
    );
    assert!(!server.app.state.dock_focused);
    server.app.state.assert_invariants_for_test();

    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn dock_focus_clears_on_tab_and_workspace_focus() {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("dock-tabs");
    let second_tab = workspace.test_add_tab(Some("second"));
    workspace.insert_test_runtime(
        workspace.tabs[0].root_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"ONE"),
    );
    workspace.insert_test_runtime(
        workspace.tabs[second_tab].root_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"TWO"),
    );
    let mut other = crate::workspace::Workspace::test_new("dock-other-space");
    other.insert_test_runtime(
        other.tabs[0].root_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"THREE"),
    );
    server.app.state.workspaces = vec![workspace, other];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let second_tab_id = server.app.public_tab_id(0, second_tab).unwrap();
    let other_workspace_id = server.app.public_workspace_id(1);
    let (control, _render) = connect_matching_test_shell(&mut server, 51);
    let _ = control.recv().expect("snapshot");
    server.app.install_test_dock_runtime(
        crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 23, b"DOCK"),
        None,
    );

    focus_dock(&mut server, 51);
    assert!(server.app.state.dock_focused);
    shell_request(
        &mut server,
        51,
        crate::api::schema::Method::TabFocus(crate::api::schema::TabTarget {
            tab_id: second_tab_id,
        }),
    );
    assert!(!server.app.state.dock_focused, "tab focus leaves the dock");

    focus_dock(&mut server, 51);
    shell_request(
        &mut server,
        51,
        crate::api::schema::Method::WorkspaceFocus(crate::api::schema::WorkspaceTarget {
            workspace_id: other_workspace_id,
        }),
    );
    assert!(
        !server.app.state.dock_focused,
        "workspace focus leaves the dock"
    );
    assert!(server.app.state.dock_pane.is_some(), "the dock stays open");

    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn dock_focus_returns_to_tiled_pane_on_close() {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("dock-close");
    let pane = workspace.tabs[0].root_pane;
    workspace.insert_test_runtime(
        pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"TILED"),
    );
    server.app.state.workspaces = vec![workspace];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let pane_id = server.app.public_pane_id(0, pane).unwrap();
    let (control, _render) = connect_matching_test_shell(&mut server, 61);
    let _ = control.recv().expect("snapshot");
    server.app.install_test_dock_runtime(
        crate::terminal::TerminalRuntime::test_with_screen_bytes(20, 23, b"DOCK"),
        None,
    );
    focus_dock(&mut server, 61);

    // The close-pane key sends the focused pane id, which is `dock` here.
    let closed = shell_request(
        &mut server,
        61,
        crate::api::schema::Method::PaneClose(crate::api::schema::PaneTarget {
            pane_id: DOCK.into(),
        }),
    );
    assert_eq!(closed["result"]["type"], "ok", "{closed}");
    assert!(server.app.state.dock_pane.is_none());
    assert!(!server.app.state.dock_focused);
    server.render_and_stream();
    assert_eq!(
        latest_snapshot(&control).focused_pane_id.as_deref(),
        Some(pane_id.as_str())
    );
    server.app.state.assert_invariants_for_test();

    shutdown_test_runtimes(&mut server);
}

#[tokio::test]
async fn popup_takes_input_over_focused_dock() {
    let mut server = test_headless_server();
    let mut workspace = crate::workspace::Workspace::test_new("dock-popup");
    workspace.insert_test_runtime(
        workspace.tabs[0].root_pane,
        crate::terminal::TerminalRuntime::test_with_screen_bytes(80, 23, b"TILED"),
    );
    server.app.state.workspaces = vec![workspace];
    server.app.state.ensure_test_terminals();
    server.app.state.active = Some(0);
    server.app.state.selected = 0;
    server.app.state.mode = crate::app::Mode::Terminal;
    let tab_id = server.app.public_tab_id(0, 0).unwrap();
    let (control, _render) = connect_matching_test_shell(&mut server, 71);
    let _ = control.recv().expect("snapshot");
    let (dock_runtime, mut dock_input) =
        crate::terminal::TerminalRuntime::test_with_channel_and_scrollback_bytes(
            20, 23, 0, b"DOCK", 4,
        );
    server.app.install_test_dock_runtime(dock_runtime, None);
    focus_dock(&mut server, 71);
    server.app.install_test_popup_runtime(
        crate::terminal::TerminalRuntime::test_with_screen_bytes(40, 12, b"POPUP"),
    );
    server.popup_owner_tab_id = Some(tab_id);

    assert!(
        !server.handle_server_event(ServerEvent::ClientShellPaneInput {
            client_id: 71,
            pane_id: DOCK.into(),
            events: vec![text_key("blocked")],
        })
    );
    assert!(dock_input.try_recv().is_err(), "the popup owns input");

    assert!(server.app.close_popup_pane());
    server.popup_owner_tab_id = None;
    assert!(
        server.app.state.dock_focused,
        "dock focus survives the popup"
    );
    server.handle_server_event(ServerEvent::ClientShellPaneInput {
        client_id: 71,
        pane_id: DOCK.into(),
        events: vec![text_key("after")],
    });
    assert_eq!(
        dock_input.try_recv().expect("dock input after popup"),
        Bytes::from_static(b"after")
    );

    shutdown_test_runtimes(&mut server);
}
