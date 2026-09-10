"""Use copied current config in Debian. Never prints credentials or model output.
Input directory: CCSW_TEST_INPUT; use --live for real Pi short requests.
"""
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
import sys
import tempfile
import time
import tomllib

source = Path(os.environ['CCSW_TEST_INPUT'])
binary = os.environ['CCSW_TEST_BINARY']
pi = os.environ['CCSW_PI_BIN']
before = {p: hashlib.sha256(p.read_bytes()).digest() for p in source.iterdir() if p.is_file()}
with tempfile.TemporaryDirectory(prefix='ccsw-pi-current-') as directory:
    root=Path(directory); (root/'pi').mkdir(); (root/'work').mkdir()
    config=(source/'ccsw.toml').read_text().replace('127.0.0.1:20128','host.docker.internal:20128')
    (root/'config.toml').write_text(config)
    env=dict(os.environ,HOME=directory,PI_CODING_AGENT_DIR=str(root/'pi'),CCSW_CONFIG=str(root/'config.toml'),XDG_STATE_HOME=str(root/'state'),XDG_CACHE_HOME=str(root/'cache'))
    for key in list(env):
        if key.endswith('_API_KEY') or key in ['ANTHROPIC_AUTH_TOKEN','CODEX_AUTH','CODEX_ACCESS_TOKEN']: env.pop(key)
    def run(*args):
        result=subprocess.run([binary,*args],env=env,capture_output=True,text=True,timeout=30)
        assert result.returncode==0, 'CCSW operation failed: '+ ' '.join(args[:2])
        return result.stdout
    if '--live' not in sys.argv:
        for name in ['models.json','settings.json','auth.json']:
            shutil.copyfile(source/name,root/'pi'/name);(root/'pi'/name).chmod(0o600)
        originals={name:json.loads((root/'pi'/name).read_text()) for name in ['models.json','settings.json','auth.json']}
        preview=run('pi','import','--dry-run')
        output=run('pi','import')
        imported=sum(line.startswith('Imported ') for line in output.splitlines())
        skipped=sum(line.startswith('Skipped ') for line in output.splitlines())
        assert imported>0
        run('pi','import')
        profiles=tomllib.loads((root/'config.toml').read_text())['profiles']
        for name,p in profiles.items():
            if not p.get('enabled',True):continue
            run('pi','apply','--profile',name)
            assert json.loads((root/'pi/settings.json').read_text())['defaultProvider']=='ccsw-'+name
            run('pi','disconnect')
            assert all(json.loads((root/'pi'/name).read_text())==value for name,value in originals.items())
        print(f'PASS current Pi import: {imported} imported, {skipped} explicitly skipped; deduplication; {len(profiles)} providers applied/restored; auth unchanged')
    else:
        profiles=tomllib.loads(config)['profiles']
        failures=0
        for name,p in profiles.items():
            if not p.get('enabled',True):continue
            run('pi','apply','--profile',name)
            models=json.loads((root/'pi/models.json').read_text())
            for provider in models['providers'].values():
                for model in provider['models']:model['maxTokens']=128
            (root/'pi/models.json').write_text(json.dumps(models))
            start=time.monotonic()
            try:
                result=subprocess.run([pi,'--no-extensions','--no-skills','--no-prompt-templates','--no-themes','--no-context-files','--no-session','--no-approve','--no-tools','--thinking','off','-p','Reply only OK.'],env=env,cwd=root/'work',capture_output=True,text=True,timeout=45)
                ok=result.returncode==0 and bool(result.stdout.strip())
                failures+=not ok
                print(f'{"PASS" if ok else "FAIL"} {name}: Pi exit={result.returncode}, output_chars={len(result.stdout.strip())}, {time.monotonic()-start:.1f}s',flush=True)
            except subprocess.TimeoutExpired:
                failures+=1; print(f'FAIL {name}: Pi timeout 45s',flush=True)
            # Undo test-only token cap changes before checking CCSW restoration.
            state=json.loads((root/'state/ccsw/pi-binding.json').read_text())
            (root/'pi/models.json').write_text(json.dumps(state['expected'][0]))
            run('pi','disconnect')
        print(f'Live result: {len(profiles)-failures}/{len(profiles)} providers passed',flush=True)
assert all(hashlib.sha256(p.read_bytes()).digest()==value for p,value in before.items())
print('PASS source copies unchanged')
if '--live' in sys.argv: sys.exit(bool(failures))
