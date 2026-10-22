"""Hermes plugin installed by Herdr to report resumable session identity."""

# HERDR_INTEGRATION_ID=hermes
# HERDR_INTEGRATION_VERSION=6

from __future__ import annotations

import os
import subprocess
import sys
import time

_SOURCE = "herdr:hermes"
_AGENT = "hermes"
_INTERACTIVE_PLATFORMS = {"cli", "tui", "desktop", "acp"}


def _client_env():
    """The environment of the terminal this session's client runs in, or None when unknown.

    Hermes 0.22.4+ runs every CLI session in one shared gateway, whose own HERDR_PANE_ID names the
    pane that happened to start it. The gateway binds each session's client environment for its
    turns; HERDR_* reach it only when `terminal.client_env_passthrough` lists them, which
    `herdr integration install hermes` adds.
    shortcut: the map is frozen when the session is created, so a session resumed in another pane
    still reports to the pane it started in; herdr restores panes under the same id.
    """
    try:
        from agent.session_client_env import bound_client_env

        bound = bound_client_env()
    except Exception:
        bound = None
    if bound is not None:
        return bound
    # An unbound session in the gateway (resumed from before client_env, Desktop) has no pane.
    return None if _in_shared_gateway() else os.environ


def _in_shared_gateway() -> bool:
    if os.environ.get("HERMES_GATEWAY_SESSION") == "1":
        return True
    try:
        return sys.modules["gateway.run"]._gateway_runner_ref() is not None
    except Exception:
        return False


def _pane_id(env) -> str | None:
    if env is None or env.get("HERDR_ENV") != "1":
        return None
    return env.get("HERDR_PANE_ID", "").strip() or None


def _send_session(session_id: str, start_source: str) -> None:
    env = _client_env()
    pane_id = _pane_id(env)
    if pane_id is None:
        return
    command = [
        env.get("HERDR_BIN_PATH") or "herdr",
        "pane",
        "report-agent-session",
        pane_id,
        "--source",
        _SOURCE,
        "--agent",
        _AGENT,
        "--seq",
        str(time.time_ns()),
        "--agent-session-id",
        session_id,
        "--session-start-source",
        start_source,
    ]
    try:
        # The report goes to the client's herdr server, not the one the gateway was started under.
        herdr_env = {name: value for name, value in env.items() if name.startswith("HERDR_")}
        kwargs = {
            "timeout": 1,
            "stdout": subprocess.DEVNULL,
            "stderr": subprocess.DEVNULL,
            "env": {**os.environ, **herdr_env},
        }
        if os.name == "nt":
            kwargs["creationflags"] = subprocess.CREATE_NO_WINDOW
        subprocess.run(command, check=False, **kwargs)
    except Exception:
        pass


def _report_session(start_source: str, **kwargs) -> None:
    if kwargs.get("platform") not in _INTERACTIVE_PLATFORMS:
        return
    session_id = kwargs.get("session_id")
    if not isinstance(session_id, str) or not session_id:
        return
    _send_session(session_id, start_source)


def _session_started(**kwargs) -> None:
    _report_session("startup", **kwargs)


def _session_reset(**kwargs) -> None:
    _report_session("new", **kwargs)


def _session_observed(**kwargs) -> None:
    if kwargs.get("platform") == "cli":
        _report_session("resume", **kwargs)


def register(ctx):
    ctx.register_hook("on_session_start", _session_started)
    ctx.register_hook("on_session_reset", _session_reset)
    ctx.register_hook("pre_llm_call", _session_observed)
