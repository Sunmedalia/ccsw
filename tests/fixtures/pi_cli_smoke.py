"""Real Pi CLI + local mock upstream, with isolated configuration and fixed tools.
CCSW_TEST_BINARY and CCSW_PI_BIN override executables; SMOKE_FORMAT selects protocol.
"""
import http.server
import json
import os
from pathlib import Path
import subprocess
import tempfile
import threading

binary = os.environ.get('CCSW_TEST_BINARY', str(Path('target/debug/ccsw').resolve()))
pi = os.environ.get('CCSW_PI_BIN', 'pi')
kind = os.environ.get('SMOKE_FORMAT', 'openai-chat')
with tempfile.TemporaryDirectory(prefix='ccsw-pi-smoke-') as directory:
    root = Path(directory)
    (root / 'pi').mkdir()
    (root / 'work').mkdir()
    seen = []
    class Handler(http.server.BaseHTTPRequestHandler):
        def log_message(self, *args): pass
        def do_POST(self):
            body = json.loads(self.rfile.read(int(self.headers['Content-Length'])))
            seen.append(body)
            expected_path = {"anthropic": "/v1/messages", "openai-chat": "/v1/chat/completions", "openai-responses": "/v1/responses"}[kind]
            if self.path.split("?", 1)[0] != expected_path:
                print("Unexpected mock path:", self.path, flush=True); self.send_response(404); self.end_headers(); return
            step = len(seen)
            command = "printf 'verified\\n' > example.txt"
            call = {'type': 'function_call', 'id': 'fc_one', 'call_id': 'call_one', 'name': 'bash', 'arguments': json.dumps({'command': command}), 'status': 'completed'}
            message = {'type': 'message', 'id': 'msg_done', 'role': 'assistant', 'status': 'completed', 'content': [{'type': 'output_text', 'text': 'Verified.', 'annotations': []}]}
            item = call if step == 1 else message
            if kind == 'anthropic':
                block = {'type': 'tool_use', 'id': 'call_one', 'name': 'bash', 'input': {}} if step == 1 else {'type': 'text', 'text': ''}
                delta = {'type': 'input_json_delta', 'partial_json': call['arguments']} if step == 1 else {'type': 'text_delta', 'text': 'Verified.'}
                events = [
                    {'type': 'message_start', 'message': {'id': 'msg_one', 'model': 'test-model', 'role': 'assistant', 'content': [], 'usage': {'input_tokens': 1, 'output_tokens': 0}}},
                    {'type': 'content_block_start', 'index': 0, 'content_block': block},
                    {'type': 'content_block_delta', 'index': 0, 'delta': delta},
                    {'type': 'content_block_stop', 'index': 0},
                    {'type': 'message_delta', 'delta': {'stop_reason': 'tool_use' if step == 1 else 'end_turn'}, 'usage': {'output_tokens': 1}},
                    {'type': 'message_stop'}]
            elif kind == 'openai-chat':
                delta = {'tool_calls': [{'index': 0, 'id': 'call_one', 'type': 'function', 'function': {'name': 'bash', 'arguments': call['arguments']}}]} if step == 1 else {'content': 'Verified.'}
                events = [{'id': 'chat_one', 'model': 'test-model', 'choices': [{'index': 0, 'delta': delta, 'finish_reason': None}]}, {'id': 'chat_one', 'model': 'test-model', 'choices': [{'index': 0, 'delta': {}, 'finish_reason': 'tool_calls' if step == 1 else 'stop'}], 'usage': {'prompt_tokens': 1, 'completion_tokens': 1}}]
            else:
                response = {'id': 'resp_one', 'object': 'response', 'created_at': 0, 'model': 'test-model', 'status': 'completed', 'output': [item], 'usage': {'input_tokens': 1, 'output_tokens': 1, 'total_tokens': 2}}
                events = [{'type': 'response.created', 'response': dict(response, status='in_progress', output=[])}, {'type': 'response.output_item.added', 'output_index': 0, 'item': dict(item, status='in_progress')}, {'type': 'response.output_item.done', 'output_index': 0, 'item': item}, {'type': 'response.completed', 'response': response}]
            payload = ''.join(('event: ' + e['type'] + '\n' if 'type' in e else '') + 'data: ' + json.dumps(e) + '\n\n' for e in events)
            if kind == 'openai-chat': payload += 'data: [DONE]\n\n'
            encoded = payload.encode()
            self.send_response(200)
            self.send_header('Content-Type', 'text/event-stream')
            self.send_header('Content-Length', str(len(encoded)))
            self.end_headers()
            self.wfile.write(encoded)
    server = http.server.ThreadingHTTPServer(('127.0.0.1', 0), Handler)
    threading.Thread(target=server.serve_forever, daemon=True).start()
    (root / 'config.toml').write_text(f"version=3\n[profiles.local]\nname='Pi test'\nbase_url='http://127.0.0.1:{server.server_port}/v1'\napi_format='{kind}'\ndefault_model='test-model'\n")
    env = dict(os.environ, HOME=directory, PI_CODING_AGENT_DIR=str(root/'pi'), CCSW_CONFIG=str(root/'config.toml'), XDG_STATE_HOME=str(root/'state'), XDG_CACHE_HOME=str(root/'cache'))
    for k in list(env):
        if k.endswith('_API_KEY') or k in ['ANTHROPIC_AUTH_TOKEN', 'CODEX_ACCESS_TOKEN', 'CODEX_AUTH']: env.pop(k)
    result = subprocess.run([binary, 'pi', 'apply', '--profile', 'local'], env=env, capture_output=True, text=True, timeout=20)
    assert result.returncode == 0, result.stderr
    flags = ['--no-extensions', '--no-skills', '--no-prompt-templates', '--no-themes', '--no-context-files', '--no-session', '--no-approve']
    listed = subprocess.run([pi, *flags, '--list-models', 'test-model'], cwd=root/'work', env=env, capture_output=True, text=True, timeout=30)
    assert listed.returncode == 0 and 'ccsw-local' in listed.stdout, listed.stderr
    result = subprocess.run([pi, *flags, '--provider', 'ccsw-local', '--model', 'test-model', '--tools', 'bash', '-p', 'Create example.txt with verified and confirm.'], cwd=root/'work', env=env, capture_output=True, text=True, timeout=45)
    assert result.returncode == 0, result.stderr[-1200:]
    assert len(seen) == 2, f'Expected tool + final requests, got {len(seen)}: {result.stdout[-1200:]}'
    assert (root/'work/example.txt').read_text() == 'verified\n'
    second = json.dumps(seen[1])
    assert 'call_one' in second, 'Tool result must return to upstream'
    print(f'PASS Pi {kind}: native model discovery, streamed tool call, tool output, final response; 2 requests')
    server.shutdown()
