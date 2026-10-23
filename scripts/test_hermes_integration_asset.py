import importlib.util
import sys
import types
import unittest
from pathlib import Path
from unittest import mock


ASSET = Path(__file__).parents[1] / "src/integration/assets/hermes/__init__.py"


def load_asset():
    spec = importlib.util.spec_from_file_location("herdr_hermes_integration", ASSET)
    if spec is None or spec.loader is None:
        raise RuntimeError("could not load Hermes integration asset")
    module = importlib.util.module_from_spec(spec)
    spec.loader.exec_module(module)
    return module


class FakeContext:
    def __init__(self):
        self.hooks = {}

    def register_hook(self, name, callback):
        self.hooks[name] = callback


class HermesIntegrationAssetTests(unittest.TestCase):
    def test_reports_only_root_session_identity(self):
        module = load_asset()
        calls = []
        module._send_session = lambda session_id, start_source: calls.append(
            (session_id, start_source)
        )
        context = FakeContext()
        module.register(context)

        self.assertEqual(
            set(context.hooks),
            {"on_session_start", "on_session_reset", "pre_llm_call"},
        )

        context.hooks["on_session_start"](session_id="root-1", platform="tui")
        context.hooks["pre_llm_call"](session_id="root-1", platform="tui")
        context.hooks["pre_llm_call"](session_id="child", platform="subagent")
        context.hooks["on_session_reset"](session_id="root-2", platform="tui")
        context.hooks["pre_llm_call"](
            session_id="background", platform="tui"
        )

        self.assertEqual(calls, [("root-1", "startup"), ("root-2", "new")])

    def test_send_session_uses_cli_with_the_active_pane(self):
        module = load_asset()
        environment = {
            "HERDR_ENV": "1",
            "HERDR_PANE_ID": "w1:p2",
            "HERDR_BIN_PATH": "C:/bin/herdr.exe",
        }
        with mock.patch.dict(module.os.environ, environment, clear=True):
            with mock.patch.object(module.subprocess, "run") as run:
                module._send_session("session-1", "resume")

        command = run.call_args.args[0]
        self.assertEqual(
            command[:4],
            ["C:/bin/herdr.exe", "pane", "report-agent-session", "w1:p2"],
        )
        self.assertIn("session-1", command)
        self.assertIn("resume", command)
        self.assertFalse(run.call_args.kwargs["check"])
        if module.os.name == "nt":
            self.assertEqual(
                run.call_args.kwargs["creationflags"],
                module.subprocess.CREATE_NO_WINDOW,
            )

    def test_first_turn_recovers_resumed_session_identity(self):
        module = load_asset()
        calls = []
        module._send_session = lambda session_id, start_source: calls.append(
            (session_id, start_source)
        )
        context = FakeContext()
        module.register(context)

        context.hooks["pre_llm_call"](session_id="resumed", platform="cli")

        self.assertEqual(calls, [("resumed", "resume")])

    def send_in_gateway(self, client_env):
        """`_send_session` inside a shared gateway that bound `client_env` (None: unbound)."""
        module = load_asset()
        gateway = types.ModuleType("gateway.run")
        gateway._gateway_runner_ref = lambda: object()
        bound = types.ModuleType("agent.session_client_env")
        bound.bound_client_env = lambda: client_env
        modules = {
            "gateway.run": gateway,
            "agent": types.ModuleType("agent"),
            "agent.session_client_env": bound,
        }
        gateway_env = {
            "HERDR_ENV": "1",
            "HERDR_PANE_ID": "w1:p1",
            "HERDR_SOCKET_PATH": "/gateway.sock",
            "PATH": "/gateway/bin",
        }
        with mock.patch.dict(sys.modules, modules), mock.patch.dict(
            module.os.environ, gateway_env, clear=True
        ), mock.patch.object(module.subprocess, "run") as run:
            module._send_session("session-1", "resume")
        return run

    def test_a_gateway_session_reports_to_its_clients_pane_and_server(self):
        run = self.send_in_gateway(
            {
                "HERDR_ENV": "1",
                "HERDR_PANE_ID": "w2:p3",
                "HERDR_SOCKET_PATH": "/client.sock",
                "PATH": "/client/bin",
            }
        )

        self.assertEqual(run.call_args.args[0][3], "w2:p3")
        env = run.call_args.kwargs["env"]
        self.assertEqual(env["HERDR_SOCKET_PATH"], "/client.sock")
        # Only HERDR_* come from the client; the report runs in the gateway's own PATH.
        self.assertEqual(env["PATH"], "/gateway/bin")

    def test_a_gateway_session_without_a_client_pane_reports_nowhere(self):
        for client_env in ({"PATH": "/client/bin"}, {"HERDR_PANE_ID": "w2:p3"}, None):
            run = self.send_in_gateway(client_env)
            self.assertFalse(run.called, client_env)


if __name__ == "__main__":
    unittest.main()
