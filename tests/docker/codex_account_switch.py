"""Switch retained real logins locally; never query quota or print credentials."""
import hashlib,json,os,pathlib,subprocess,tempfile,tomllib
source=pathlib.Path(os.environ.get('CCSW_TEST_INPUT','/root/codex-test'))
binary=os.environ['CCSW_TEST_BINARY']
inputs=[source/'auth.json',source/'auth.previous.json']
before={p:hashlib.sha256(p.read_bytes()).digest() for p in inputs}
with tempfile.TemporaryDirectory(prefix='ccsw-account-switch-') as d:
 p=pathlib.Path(d);home=p/'codex';home.mkdir(mode=0o700)
 (home/'config.toml').write_text('cli_auth_credentials_store="file"\n')
 (p/'config.toml').write_text('version=4\n')
 env=dict(os.environ,HOME=d,CODEX_HOME=str(home),CCSW_CONFIG=str(p/'config.toml'),XDG_STATE_HOME=str(p/'state'),XDG_CACHE_HOME=str(p/'cache'))
 for key in ['OPENAI_API_KEY','CODEX_ACCESS_TOKEN','CODEX_AUTH']:env.pop(key,None)
 def run(*args):
  r=subprocess.run([binary,*args],env=env,capture_output=True,text=True,timeout=30)
  assert r.returncode==0, f'{args[:3]} failed (diagnostic withheld)'
  return r.stdout.strip()
 ids=[]
 for name,file in [('Previous',inputs[1]),('Current',inputs[0])]:
  ids.append(run('codex','accounts','import','--name',name,'--file',str(file)).removeprefix('Saved account '))
 if ids[0]!=ids[1]:
  for id,file in [(ids[0],inputs[1]),(ids[1],inputs[0]),(ids[0],inputs[1]),(ids[1],inputs[0])]:
   run('codex','accounts','use',id)
   assert json.loads((home/'auth.json').read_bytes())==json.loads(file.read_bytes())
   assert tomllib.loads((home/'config.toml').read_text())['model_provider']=='openai'
  print('PASS two real account identities: previous -> current -> previous -> current')
 else:
  (home/'auth.json').write_bytes(inputs[1].read_bytes())
  run('codex','accounts','use',ids[1])
  assert json.loads((home/'auth.json').read_bytes())==json.loads(inputs[0].read_bytes())
  print('PASS same identity reimport: fresh credentials replace previous login')
 r=subprocess.run(['codex','login','status'],env=env,capture_output=True,text=True,timeout=30)
 assert r.returncode==0 and 'ChatGPT' in r.stderr+r.stdout
 print('PASS installed Codex recognizes the selected ChatGPT login (local check only)')
 assert not any(a.get('refreshed_at') for a in tomllib.loads((p/'config.toml').read_text())['codex']['accounts'].values())
 assert all(hashlib.sha256(p.read_bytes()).digest()==h for p,h in before.items())
 print('PASS no quota queries; retained input files unchanged')
