"""Test read-only /input/{ccsw.toml,codex.toml,auth.json} copies.
Run with --offline and Docker --network none for subscription snapshot checks.
Live mode sends one small Responses request per configured provider. Never prints credentials.
"""
import hashlib
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time
import tomllib
import urllib.request
import urllib.error

OFFLINE = '--offline' in sys.argv
failures = 0
sources = [p for p in Path('/input').iterdir() if p.is_file()]
before = {p: hashlib.sha256(p.read_bytes()).digest() for p in sources}
with tempfile.TemporaryDirectory(prefix='ccsw-current-') as directory:
    root = Path(directory)
    (root / 'codex').mkdir(mode=0o700)
    original = Path('/input/codex.toml').read_bytes()
    (root / 'codex/config.toml').write_bytes(original)
    config = Path('/input/ccsw.toml').read_text()
    if not OFFLINE:
        config = config.replace('127.0.0.1:20128', 'host.docker.internal:20128')
    (root / 'config.toml').write_text(config)
    profiles = tomllib.loads(config)['profiles']
    env = dict(os.environ, HOME=directory, USERPROFILE=directory,
               CODEX_HOME=str(root / 'codex'), CCSW_CONFIG=str(root / 'config.toml'),
               XDG_STATE_HOME=str(root / 'state'), XDG_CACHE_HOME=str(root / 'cache'),
               CLAUDE_CONFIG_DIR=str(root / 'claude'))
    for name in ['OPENAI_API_KEY', 'CODEX_ACCESS_TOKEN', 'CODEX_AUTH']:
        env.pop(name, None)
    def run(*args):
        result = subprocess.run(['/usr/local/bin/ccsw-test', *args], env=env,
                                capture_output=True, text=True, timeout=25)
        if result.returncode:
            raise RuntimeError('Command failed: ' + ' '.join(args[:3]))
        return result.stdout.strip()
    with socket.socket() as sock:
        sock.bind(('127.0.0.1', 0))
        port = sock.getsockname()[1]
    try:
        run('proxy', 'start', '--listen', f'127.0.0.1:{port}')
        account = None
        if OFFLINE:
            auth = Path('/input/auth.json').read_bytes()
            (root / 'codex/auth.json').write_bytes(auth)
            (root / 'codex/auth.json').chmod(0o600)
            account = run('codex', 'accounts', 'import', '--name', 'Docker current').removeprefix('Saved account ')
            assert run('codex', 'accounts', 'import', '--name', 'Docker current') == 'Saved account ' + account
            assert len(tomllib.loads((root / 'config.toml').read_text())['codex']['accounts']) == 1
            run('codex', 'accounts', 'use', account)
            run('codex', 'disconnect')
            assert json.loads((root / 'codex/auth.json').read_bytes()) == json.loads(auth)
            print('PASS real account import, deduplication, activation and disconnect (offline)', flush=True)
        for name, profile in profiles.items():
            if (os.environ.get('TEST_PROFILE') and name != os.environ['TEST_PROFILE']) or not profile.get('enabled', True):
                continue
            run('codex', 'apply', '--profile', name)
            applied = tomllib.loads((root / 'codex/config.toml').read_text())
            assert applied['model_provider'] == 'ccsw'
            assert not applied['model'].endswith('[1m]')
            provider = applied['model_providers']['ccsw']
            if OFFLINE:
                run('codex', 'accounts', 'use', account)
                assert json.loads((root / 'codex/auth.json').read_bytes()) == json.loads(auth)
                print(f'PASS {name}: API configuration -> subscription', flush=True)
            else:
                start = time.monotonic()
                body = {'model': applied['model'], 'input': 'Reply only OK.', 'stream': True, 'max_output_tokens': int(os.environ.get('TEST_MAX_TOKENS', '128'))}
                request = urllib.request.Request(provider['base_url'].rstrip('/') + '/responses',
                    data=json.dumps(body).encode(), headers={'Content-Type': 'application/json',
                    'Authorization': 'Bearer ' + provider['experimental_bearer_token']})
                try:
                    with urllib.request.urlopen(request, timeout=45) as response:
                        payload = response.read(2 * 1024 * 1024).decode()
                        events = []
                        for line in payload.splitlines():
                            if line.startswith('data: ') and line[6:] != '[DONE]':
                                events.append(json.loads(line[6:]))
                        completed = any(e.get('type') == 'response.completed' for e in events)
                        terminal = [e.get('type') for e in events if e.get('type') in ['response.completed', 'response.incomplete', 'response.failed', 'error']]
                        capped = any(e.get('response', {}).get('incomplete_details', {}).get('reason') == 'max_output_tokens' for e in events if isinstance(e.get('response', {}).get('incomplete_details'), dict))
                        text = ''.join(e.get('delta', '') for e in events if e.get('type') == 'response.output_text.delta')
                        failures += not (completed and text.strip())
                        print(f'{"PASS" if completed and text.strip() else "FAIL"} {name}: HTTP {response.status}, completed={completed}, text_chars={len(text)}, terminal={terminal}, token_limit={capped}, {time.monotonic()-start:.1f}s', flush=True)
                except urllib.error.HTTPError as error:
                    failures += 1
                    print(f'FAIL {name}: HTTP {error.code}, {time.monotonic()-start:.1f}s', flush=True)
                except Exception as error:
                    failures += 1
                    print(f'FAIL {name}: {type(error).__name__}, {time.monotonic()-start:.1f}s', flush=True)
            run('codex', 'disconnect')
            restored = tomllib.loads((root / 'codex/config.toml').read_text())
            assert restored == tomllib.loads(original.decode()), 'Configuration restoration mismatch'
        assert tomllib.loads((root / 'config.toml').read_text())['version'] == 4
        print('PASS v2 -> v4 migration; original Codex configuration restored after every provider', flush=True)
    finally:
        run('proxy', 'stop')
assert all(hashlib.sha256(p.read_bytes()).digest() == digest for p, digest in before.items())
print('PASS all read-only source files unchanged', flush=True)

sys.exit(1 if failures else 0)
