"""Destructive-operation regression suite. Run only against isolated temporary HOME."""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import tempfile
import socket
import fcntl
import unittest

BINARY = os.environ.get("CCSW_TEST_BINARY", "/usr/local/bin/ccsw-test")

class CleanupSafety(unittest.TestCase):
    def setUp(self):
        self.temp = tempfile.TemporaryDirectory(prefix="ccsw-safety-")
        self.root = Path(self.temp.name)
        self.home = self.root / "用户 home"
        self.home.mkdir(mode=0o700)
        self.env = dict(os.environ)
        for key in ["HOME", "USERPROFILE", "CCSW_CONFIG", "CLAUDE_CONFIG_DIR", "XDG_CONFIG_HOME", "XDG_STATE_HOME", "XDG_CACHE_HOME"]:
            self.env.pop(key, None)
        self.env.update(HOME=str(self.home), USERPROFILE=str(self.home), PATH=os.environ["PATH"])
        self.config = self.home / ".config/ccsw/config.toml"
        self.state = self.home / ".local/state/ccsw"
        self.cache = self.home / ".cache/ccsw/models.json"
        self.settings = self.home / ".claude/settings.json"
        self.write(self.config, 'version = 2\n[profiles.local]\nname="Local"\nbase_url="https://example.invalid"\ndefault_model="model"\n')
        self.write(self.settings, json.dumps({"theme": "dark", "env": {"KEEP": "safe"}, "permissions": {"allow": ["Read"]}}))
        self.write(self.home / ".claude/projects/conversation.jsonl", "do not delete chats")
        self.write(self.home / "other-app/config.toml", "do not delete")

    def write(self, path, text):
        path.parent.mkdir(parents=True, exist_ok=True)
        path.write_text(text)
        path.chmod(0o600)

    def cmd(self, *args, ok=True):
        result = subprocess.run([BINARY, *args], env=self.env, capture_output=True, text=True, timeout=25)
        if ok:
            self.assertEqual(result.returncode, 0, result.stdout + result.stderr)
        else:
            self.assertNotEqual(result.returncode, 0, result.stdout + result.stderr)
        return result

    def snapshot(self):
        result = {}
        for path in self.root.rglob("*"):
            if path.is_symlink():
                result[str(path)] = ("link", os.readlink(path))
            elif path.is_file():
                result[str(path)] = hashlib.sha256(path.read_bytes()).hexdigest()
            else:
                result[str(path)] = "directory"
        return result

    def start(self, sync=False):
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        self.cmd("proxy", "start", "--listen", f"127.0.0.1:{port}")
        if sync:
            self.cmd("apply", "--profile", "local")
        return port

    def tearDown(self):
        # No production state is reachable by this isolated environment.
        if (self.state / "proxy.json").is_file() and not (self.state / "proxy.json").is_symlink():
            subprocess.run([BINARY, "proxy", "stop"], env=self.env, capture_output=True, timeout=15)
        self.temp.cleanup()

    def test_preview_no_mutation_and_no_confirmation_is_preview(self):
        self.start(sync=True)
        before = self.snapshot()
        self.cmd("uninstall", "--dry-run")
        self.assertEqual(before, self.snapshot())
        self.cmd("uninstall")
        self.assertEqual(before, self.snapshot())
        self.cmd("uninstall", "--dry-run", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_full_cleanup_stops_proxy_preserves_other_files_and_is_idempotent(self):
        port = self.start(sync=True)
        self.write(self.cache, '{"profiles":{}}')
        self.write(self.cache.with_suffix(".json.lock"), "")
        self.write(self.state / "unrelated.txt", "keep")
        self.cmd("uninstall", "--yes")
        self.assertFalse(self.config.exists())
        self.assertFalse(self.cache.exists())
        self.assertEqual([p.name for p in self.state.iterdir()], ["unrelated.txt"])
        self.assertEqual(json.loads(self.settings.read_text()), {"theme":"dark", "env":{"KEEP":"safe"}, "permissions":{"allow":["Read"]}})
        self.assertFalse(self.settings.with_suffix(".json.ccsw-backup").exists())
        self.assertTrue((self.home / ".claude/projects/conversation.jsonl").exists())
        self.assertTrue((self.home / "other-app/config.toml").exists())
        with socket.socket() as listener:
            listener.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
            listener.bind(("127.0.0.1", port))
        before = self.snapshot()
        self.cmd("uninstall", "--yes")
        self.assertEqual(before, self.snapshot())

    def test_custom_paths_inside_home(self):
        self.env.update(CCSW_CONFIG=str(self.home / "custom/my.toml"), XDG_STATE_HOME=str(self.home / "custom-state"), XDG_CACHE_HOME=str(self.home / "custom-cache"), CLAUDE_CONFIG_DIR=str(self.home / "custom-claude"))
        custom = Path(self.env["CCSW_CONFIG"])
        self.write(custom, self.config.read_text())
        original = self.config.read_bytes()
        self.cmd("uninstall", "--yes")
        self.assertFalse(custom.exists())
        self.assertEqual(original, self.config.read_bytes())

    def test_external_claude_target_preserved(self):
        self.start(sync=True)
        value = json.loads(self.settings.read_text())
        value["env"]["ANTHROPIC_BASE_URL"] = "https://other.invalid"
        self.settings.write_text(json.dumps(value))
        original = self.settings.read_bytes()
        backup = self.settings.with_suffix(".json.ccsw-backup").read_bytes()
        self.cmd("uninstall", "--yes")
        self.assertEqual(original, self.settings.read_bytes())
        self.assertEqual(backup, self.settings.with_suffix(".json.ccsw-backup").read_bytes())

    def test_symlink_file_rejected_without_any_deletion(self):
        victim = self.root / "victim"
        victim.write_text("important")
        self.config.unlink()
        self.config.symlink_to(victim)
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_symlink_parent_rejected(self):
        external = self.root / "external"
        external.mkdir()
        shutil.rmtree(self.home / ".config/ccsw")
        (self.home / ".config/ccsw").symlink_to(external, target_is_directory=True)
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_hardlink_rejected(self):
        os.link(self.config, self.home / "important")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_outside_home_rejected_even_for_root(self):
        self.env["CCSW_CONFIG"] = str(self.root / "other-user/config.toml")
        self.write(Path(self.env["CCSW_CONFIG"]), self.config.read_text())
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_parent_traversal_rejected(self):
        self.env["CCSW_CONFIG"] = str(self.home / "../other.toml")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_directory_in_place_of_file_rejected(self):
        self.config.unlink()
        self.config.mkdir()
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_invalid_config_rejected(self):
        self.config.write_text("this is not toml")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_corrupt_registry_rejected(self):
        self.write(self.state / "proxy.json", "{broken")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_invalid_settings_rejected_before_removing_config(self):
        self.settings.write_text("{broken")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_busy_session_rejected(self):
        self.write(self.state / "session.lock", "")
        with (self.state / "session.lock").open("r+") as lock:
            fcntl.flock(lock, fcntl.LOCK_SH)
            before = self.snapshot()
            self.cmd("uninstall", "--yes", ok=False)
            self.assertEqual(before, self.snapshot())

    def test_busy_config_lock_rejected(self):
        lock_path = self.config.with_suffix(".toml.lock")
        self.write(lock_path, "")
        with lock_path.open("r+") as lock:
            fcntl.flock(lock, fcntl.LOCK_EX)
            before = self.snapshot()
            self.cmd("uninstall", "--yes", ok=False)
            self.assertEqual(before, self.snapshot())

    def test_world_writable_directory_rejected(self):
        self.config.parent.chmod(0o777)
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_shared_config_registry_rejected(self):
        self.start()
        registry = self.state / "proxy.json"
        value = json.loads(registry.read_text())
        value["routes"]["other"] = {"config_path": str(self.home / "other.toml"), "profile_id": "other"}
        registry.write_text(json.dumps(value))
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())
        value["routes"] = {}
        registry.write_text(json.dumps(value))

    def test_forged_pid_never_signals_other_process(self):
        self.start()
        victim = subprocess.Popen(["sleep", "30"])
        try:
            self.write(self.state / "proxy.pid", str(victim.pid))
            self.cmd("uninstall", "--yes")
            self.assertIsNone(victim.poll())
        finally:
            victim.terminate()
            victim.wait()

    def test_unknown_startup_preserved(self):
        self.write(self.home / ".config/systemd/user/ccsw-proxy.service", "unrelated user service")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_startup_disable_failure_retains_files(self):
        self.write(self.home / ".config/systemd/user/ccsw-proxy.service", f"[Unit]\nDescription=CCSW protocol proxy\n\n[Service]\nExecStart={BINARY} internal proxy-serve --registry {self.state}/proxy.json\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_wrong_uid_rejected(self):
        if os.geteuid() != 0:
            self.skipTest("requires root for chown setup")
        os.chown(self.config, 12345, 12345)
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_backup_symlink_aborts_before_settings_are_changed(self):
        self.start(sync=True)
        backup = self.settings.with_suffix(".json.ccsw-backup")
        backup.unlink()
        backup.symlink_to(self.home / "other-app/config.toml")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_forged_sync_path_outside_home_rejected(self):
        self.start(sync=True)
        state = self.state / "sync-state.json"
        value = json.loads(state.read_text())
        value["entries"][0]["settings"] = str(self.root / "victim/settings.json")
        state.write_text(json.dumps(value))
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_unrelated_sibling_files_randomized(self):
        import random
        randomizer = random.Random(17321)
        preserved = {}
        for directory in [self.config.parent, self.state, self.cache.parent, self.settings.parent]:
            for index in range(20):
                path = directory / (str(index) + ".user-" + str(randomizer.randrange(100000)))
                self.write(path, str(randomizer.getrandbits(256)))
                preserved[path] = path.read_bytes()
        self.cmd("uninstall", "--yes")
        for path, contents in preserved.items():
            self.assertEqual(path.read_bytes(), contents)

    def test_other_proxy_keeps_running(self):
        self.start(sync=True)
        other_home = self.root / "second-user"
        other_home.mkdir(mode=0o700)
        other_env = dict(self.env, HOME=str(other_home), USERPROFILE=str(other_home))
        with socket.socket() as listener:
            listener.bind(("127.0.0.1", 0))
            port = listener.getsockname()[1]
        def other(*args):
            return subprocess.run([BINARY, *args], env=other_env, capture_output=True, text=True, timeout=20)
        try:
            result = other("proxy", "start", "--listen", f"127.0.0.1:{port}")
            self.assertEqual(result.returncode, 0, result.stderr)
            registry = other_home / ".local/state/ccsw/proxy.json"
            before = registry.read_bytes()
            self.cmd("uninstall", "--yes")
            self.assertEqual(before, registry.read_bytes())
            self.assertTrue(other("proxy", "status").stdout.startswith("running"))
        finally:
            other("proxy", "stop")

    def test_startup_success_removes_only_verified_service(self):
        service = self.home / ".config/systemd/user/ccsw-proxy.service"
        self.write(service, f"[Unit]\nDescription=CCSW protocol proxy\n\n[Service]\nExecStart={BINARY} internal proxy-serve --registry {self.state}/proxy.json\nRestart=on-failure\n\n[Install]\nWantedBy=default.target\n")
        stub = self.home / "bin/systemctl"
        self.write(stub, '#!/bin/sh\n[ "$1 $2 $3 $4" = "--user disable --now ccsw-proxy.service" ]\n')
        stub.chmod(0o700)
        self.env["PATH"] = str(stub.parent) + ":" + self.env["PATH"]
        self.cmd("uninstall", "--yes")
        self.assertFalse(service.exists())
        self.assertTrue(stub.exists())

    def test_shutdown_requires_private_token(self):
        import urllib.request
        import urllib.error
        port = self.start()
        request = urllib.request.Request(f"http://127.0.0.1:{port}/internal/shutdown", data=b"", headers={"Authorization":"Bearer wrong-token"})
        with self.assertRaises(urllib.error.HTTPError) as error:
            urllib.request.urlopen(request, timeout=5)
        self.assertEqual(error.exception.code, 401)
        self.assertTrue(self.cmd("proxy", "status").stdout.startswith("running"))

    def test_nonloopback_registry_never_contacted(self):
        self.write(self.state / "proxy.json", json.dumps({"listen":"192.0.2.1:17321", "local_token":"private", "routes":{}}))
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())
        (self.state / "proxy.json").unlink()

    def test_symlink_lock_rejected(self):
        self.config.with_suffix(".toml.lock").symlink_to(self.home / "other-app/config.toml")
        before = self.snapshot()
        self.cmd("uninstall", "--yes", ok=False)
        self.assertEqual(before, self.snapshot())

    def test_uninstall_preserves_manually_edited_model(self):
        self.start(sync=True)
        value = json.loads(self.settings.read_text())
        value["model"] = "user-custom-model"
        value["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"] = "user-custom-opus"
        self.settings.write_text(json.dumps(value))
        self.cmd("uninstall", "--yes")
        result = json.loads(self.settings.read_text())
        self.assertEqual(result["model"], "user-custom-model")
        self.assertEqual(result["env"]["ANTHROPIC_DEFAULT_OPUS_MODEL"], "user-custom-opus")
        self.assertNotIn("ANTHROPIC_AUTH_TOKEN", result["env"])

    def test_legacy_uninstall_preserves_unverified_model_fields(self):
        self.start(sync=True)
        state = self.state / "sync-state.json"
        value = json.loads(state.read_text())
        for binding in value["entries"]:
            binding.pop("managed", None)
        state.write_text(json.dumps(value))
        original = json.loads(self.settings.read_text())
        self.cmd("uninstall", "--yes")
        value = json.loads(self.settings.read_text())
        self.assertEqual(value["model"], original["model"])
        self.assertEqual(value["modelPicker"], original["modelPicker"])
        self.assertNotIn("ANTHROPIC_AUTH_TOKEN", value["env"])

    def test_normal_stop_ignores_forged_pid(self):
        self.start()
        victim = subprocess.Popen(["sleep", "30"])
        try:
            self.write(self.state / "proxy.pid", str(victim.pid))
            self.cmd("proxy", "stop")
            self.assertIsNone(victim.poll())
        finally:
            victim.terminate()
            victim.wait()

    def test_real_proxy_applies_token_caps_for_each_protocol(self):
        import http.server
        import threading
        import urllib.request
        received = []
        class Upstream(http.server.BaseHTTPRequestHandler):
            def do_POST(handler):
                request = json.loads(handler.rfile.read(int(handler.headers["Content-Length"])))
                received.append((handler.path, request))
                if handler.path.endswith("/responses"):
                    response = {"id":"r", "model":"model", "output":[{"type":"message", "content":[{"type":"output_text", "text":"ok"}]}], "usage":{}}
                elif handler.path.endswith("/chat/completions"):
                    response = {"id":"r", "model":"model", "choices":[{"message":{"content":"ok"}, "finish_reason":"stop"}], "usage":{}}
                else:
                    response = {"type":"message", "content":[{"type":"text", "text":"ok"}]}
                body = json.dumps(response).encode()
                handler.send_response(200)
                handler.send_header("Content-Type", "application/json")
                handler.send_header("Content-Length", str(len(body)))
                handler.end_headers()
                handler.wfile.write(body)
            def log_message(self, *args):
                pass
        server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Upstream)
        thread = threading.Thread(target=server.serve_forever, daemon=True)
        thread.start()
        try:
            for protocol in ["anthropic", "openai-chat", "openai-responses"]:
                self.write(self.config, f'version=2\n[profiles.local]\nname="Local"\nbase_url="http://127.0.0.1:{server.server_port}"\napi_format="{protocol}"\ndefault_model="model"\n[[profiles.local.models]]\nid="model"\nmax_output_tokens=8192\ncontext_window=32768\n')
                if not (self.state / "proxy.json").exists():
                    self.start(sync=True)
                else:
                    self.cmd("apply", "--profile", "local")
                env = json.loads(self.settings.read_text())["env"]
                for amount in [4096, 16384]:
                    body = json.dumps({"model":"local::model", "max_tokens":amount, "messages":[{"role":"user", "content":"Hi"}]}).encode()
                    request = urllib.request.Request(env["ANTHROPIC_BASE_URL"] + "/v1/messages", data=body, headers={"Authorization":"Bearer " + env["ANTHROPIC_AUTH_TOKEN"], "Content-Type":"application/json"})
                    with urllib.request.urlopen(request, timeout=5) as response:
                        self.assertEqual(response.status, 200)
                        response.read()
                    key = "max_output_tokens" if protocol == "openai-responses" else "max_tokens"
                    self.assertEqual(received[-1][1][key], min(amount, 8192))
            self.cmd("uninstall", "--yes")
        finally:
            server.shutdown()
            server.server_close()
            thread.join()

if __name__ == "__main__":
    unittest.main(verbosity=2)
