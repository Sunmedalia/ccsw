"""Real Codex /model smoke test; local mock providers and isolated homes only.

Run after cargo build: python3 tests/fixtures/codex_model_switch.py
Requires a POSIX terminal and Codex CLI (or CCSW_CODEX_BIN). No API key needed.
"""
import errno
import fcntl
import http.server
import json
import os
import pathlib
import pty
import re
import select
import shutil
import signal
import socket
import struct
import subprocess
import tempfile
import termios
import threading
import time


class Terminal:
    def __init__(self, command, env, cwd):
        self.pid, self.fd = pty.fork()
        if self.pid == 0:
            os.chdir(cwd)
            os.execvpe(command[0], command, env)
        fcntl.ioctl(self.fd, termios.TIOCSWINSZ, struct.pack("HHHH", 45, 140, 0, 0))
        self.output = b""

    def pump(self, duration=0.2):
        end = time.monotonic() + duration
        while time.monotonic() < end:
            if not select.select([self.fd], [], [], min(0.1, end - time.monotonic()))[0]:
                continue
            try:
                data = os.read(self.fd, 65536)
            except OSError as error:
                if error.errno == errno.EIO:
                    raise RuntimeError("Codex exited:\n" + self.text()[-6000:]) from error
                raise
            self.output += data
            if b"\x1b[6n" in data:
                os.write(self.fd, b"\x1b[1;1R")
            if b"\x1b[c" in data:
                os.write(self.fd, b"\x1b[?1;2c")

    def text(self):
        return re.sub(r"\x1b\[[0-?]*[ -/]*[@-~]", "", self.output.decode(errors="replace"))

    def wait(self, text, timeout=20):
        end = time.monotonic() + timeout
        while text not in self.text():
            if time.monotonic() > end:
                raise AssertionError(f"Missing {text!r}:\n{self.text()[-7000:]}")
            self.pump()

    def send(self, data):
        os.write(self.fd, data.encode())
        self.pump(0.4)

    def submit(self, text):
        self.send(text)
        self.send("\r")

    def close(self):
        try:
            os.killpg(self.pid, signal.SIGTERM)
        except ProcessLookupError:
            pass
        os.waitpid(self.pid, 0)
        os.close(self.fd)


def main():
    ccsw = str(pathlib.Path(os.environ.get("CCSW_TEST_BINARY", "target/debug/ccsw")).resolve())
    codex = shutil.which(os.environ.get("CCSW_CODEX_BIN", "codex"))
    assert codex, "Install Codex CLI or set CCSW_CODEX_BIN"
    seen = []

    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *_):
            pass

        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers["Content-Length"])))
            provider = self.path.split("/")[1]
            # Newer CLIs also issue background title-generation requests.
            if body.get("tools"):
                seen.append((provider, body, self.headers.get("Authorization")))
            marker = provider.upper() + "_OK" if body.get("tools") else '{"title":"Run smoke"}'
            item = {"type": "message", "id": f"msg_{len(seen)}", "role": "assistant", "status": "completed",
                    "content": [{"type": "output_text", "text": marker, "annotations": []}]}
            response = {"id": f"resp_{len(seen)}", "object": "response", "created_at": 0,
                        "model": "shared", "status": "completed", "output": [item],
                        "usage": {"input_tokens": 10, "output_tokens": 2, "total_tokens": 12}}
            events = [{"type": "response.created", "response": dict(response, status="in_progress", output=[])},
                      {"type": "response.output_item.added", "output_index": 0, "item": dict(item, status="in_progress")},
                      {"type": "response.output_item.done", "output_index": 0, "item": item},
                      {"type": "response.completed", "response": response}]
            output = "".join(f"event: {event['type']}\ndata: {json.dumps(dict(event, sequence_number=i))}\n\n"
                             for i, event in enumerate(events)).encode()
            self.send_response(200)
            self.send_header("Content-Type", "text/event-stream")
            self.send_header("Content-Length", str(len(output)))
            self.end_headers()
            self.wfile.write(output)

    server = http.server.ThreadingHTTPServer(("127.0.0.1", 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    with tempfile.TemporaryDirectory(prefix="ccsw-model-switch-") as root:
        root = pathlib.Path(root)
        work, home = root / "work", root / "codex"
        work.mkdir()
        home.mkdir()
        (home / "config.toml").write_text(f'cli_auth_credentials_store="file"\n[projects.{json.dumps(str(work))}]\ntrust_level="trusted"\n')
        config = root / "config.toml"
        config.write_text("version=5\n" + "".join(
            f"[codex.profiles.{name}]\nname='{name.title()}'\nbase_url='http://127.0.0.1:{server.server_port}/{name}/v1'\n"
            f"api_format='openai-responses'\ndefault_model='shared'\nenabled={'false' if name == 'gamma' else 'true'}\n"
            f"[codex.profiles.{name}.credential]\nkind='bearer'\nvalue='{name}-mock-secret'\n"
            for name in ["alpha", "beta", "gamma"]))
        env = dict(os.environ, HOME=str(root), USERPROFILE=str(root), CODEX_HOME=str(home),
                   CCSW_CONFIG=str(config), XDG_CONFIG_HOME=str(root / "config"),
                   XDG_STATE_HOME=str(root / "state"), XDG_CACHE_HOME=str(root / "cache"),
                   TERM="xterm-256color", COLORTERM="truecolor")
        for key in ["OPENAI_API_KEY", "CODEX_ACCESS_TOKEN", "CODEX_AUTH", "ANTHROPIC_API_KEY"]:
            env.pop(key, None)

        def run(*args):
            result = subprocess.run([ccsw, *args], env=env, capture_output=True, text=True, timeout=30)
            assert result.returncode == 0, result.stderr
            return result.stdout

        terminal = None
        try:
            with socket.socket() as sock:
                sock.bind(("127.0.0.1", 0))
                port = sock.getsockname()[1]
            run("proxy", "start", "--listen", f"127.0.0.1:{port}")
            run("codex", "apply", "--profile", "alpha")
            command = [codex, "--no-alt-screen", "-C", str(work), "-a", "never", "-s", "workspace-write",
                       "-c", 'web_search="disabled"']
            terminal = Terminal(command + ["first-smoke"], env, str(work))
            terminal.wait("ALPHA_OK")
            terminal.pump(0.5)
            terminal.output = b""
            terminal.submit("/model")
            terminal.wait("beta::shared")
            terminal.send("\x1b[B")
            terminal.send("\r")
            terminal.wait("Model changed to beta::shared")
            terminal.send("\x1b")
            terminal.submit("second-smoke")
            terminal.wait("BETA_OK")
            assert [entry[0] for entry in seen] == ["alpha", "beta"], [entry[0] for entry in seen]
            assert all(entry[1]["model"] == "shared" for entry in seen)
            assert [entry[2] for entry in seen] == ["Bearer alpha-mock-secret", "Bearer beta-mock-secret"]
            assert "ALPHA_OK" in json.dumps(seen[1][1]), "Switch lost conversation history"
            session_ids = [entry[1].get("client_metadata", {}).get("thread_id") for entry in seen]
            assert session_ids[0] == session_ids[1], "Switch created another thread"
            original_pid = terminal.pid
            config.write_text(config.read_text().replace("enabled = false", "enabled = true"))
            run("codex", "apply", "--profile", "alpha")
            terminal.output = b""
            terminal.submit("/model")
            terminal.wait("beta::shared")
            terminal.pump(0.5)
            assert "gamma::shared" not in terminal.text(), "Unexpected live catalog reload"
            terminal.close()
            terminal = None
            terminal = Terminal(command, env, str(work))
            terminal.wait("alpha::shared")
            terminal.pump(0.5)
            terminal.submit("/model")
            terminal.wait("gamma::shared")
            print(f"PASS: /model switched alpha -> beta in PID {original_pid}, preserved history and routed credentials correctly")
            print("PASS: new model became visible after apply + restart")
        finally:
            if terminal:
                terminal.close()
            run("proxy", "stop")
            server.shutdown()


if __name__ == "__main__":
    main()
