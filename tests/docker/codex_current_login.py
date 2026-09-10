"""Real copied subscription login: import/use/read/quota/disconnect. No token logging."""
import hashlib,json,os,pathlib,subprocess,tempfile,tomllib
source=pathlib.Path(os.environ['CCSW_TEST_INPUT'])
before={p:hashlib.sha256(p.read_bytes()).digest() for p in source.iterdir() if p.is_file()}
with tempfile.TemporaryDirectory(prefix='ccsw-codex-login-') as d:
 p=pathlib.Path(d);(p/'codex').mkdir();(p/'work').mkdir()
 auth=(source/'auth.json').read_bytes();original=(source/'config.toml').read_bytes()
 (p/'codex/auth.json').write_bytes(auth);(p/'codex/auth.json').chmod(0o600)
 (p/'codex/config.toml').write_bytes(original)
 (p/'config.toml').write_text('version=4\n')
 env=dict(os.environ,HOME=d,CODEX_HOME=str(p/'codex'),CCSW_CONFIG=str(p/'config.toml'),XDG_STATE_HOME=str(p/'state'),XDG_CACHE_HOME=str(p/'cache'))
 for k in ['OPENAI_API_KEY','CODEX_AUTH','CODEX_ACCESS_TOKEN']:env.pop(k,None)
 binary=os.environ['CCSW_TEST_BINARY']
 def run(*args):
  r=subprocess.run([binary,*args],env=env,cwd=p/'work',capture_output=True,text=True,timeout=75)
  if r.returncode:print('FAIL', ' '.join(args[:3]), 'exit',r.returncode,flush=True)
  return r
 r=run('codex','accounts','import','--name','Docker current');assert r.returncode==0
 account=r.stdout.strip().removeprefix('Saved account ')
 assert run('codex','accounts','use',account).returncode==0
 assert json.loads((p/'codex/auth.json').read_bytes())==json.loads(auth)
 print('PASS real subscription import and activation',flush=True)
 r=run('codex','accounts','refresh',account)
 config=tomllib.loads((p/'config.toml').read_text());a=config['codex']['accounts'][account]
 print('quota_cached',bool(a.get('refreshed_at')),'refresh_error',bool(a.get('error')),flush=True)
 if r.returncode:
  # CCSW error strings are sanitized; no stdout/stderr from upstream is exposed.
  print('CCSW diagnostic:',r.stderr.strip()[:400],flush=True)
 assert run('codex','disconnect').returncode==0
 assert tomllib.loads((p/'codex/config.toml').read_text())==tomllib.loads(original.decode())
 print('PASS original Codex configuration restored',flush=True)
 print('copied_live_auth_unchanged',json.loads((p/'codex/auth.json').read_bytes())==json.loads(auth),flush=True)
assert all(hashlib.sha256(p.read_bytes()).digest()==v for p,v in before.items())
print('PASS input copies unchanged',flush=True)
