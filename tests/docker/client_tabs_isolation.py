"""Real PTY: each tab renders only its own v4 provider configuration."""
import fcntl,os,pathlib,pty,select,struct,subprocess,tempfile,termios,time
with tempfile.TemporaryDirectory(prefix='ccsw-tabs-isolation-') as d:
 p=pathlib.Path(d);config='version=4\n'
 for section,label in [('profiles','CLAUDE_ONLY'),('codex.profiles','CODEX_ONLY'),('pi.profiles','PI_ONLY')]:
  config+=f'[{section}.local]\nname="{label}"\nbase_url="https://example.invalid"\ndefault_model="model-{label}"\n'
 (p/'config.toml').write_text(config)
 env=dict(os.environ,HOME=d,TERM='xterm-256color',CCSW_CONFIG=str(p/'config.toml'),XDG_STATE_HOME=str(p/'state'),XDG_CACHE_HOME=str(p/'cache'),CODEX_HOME=str(p/'codex'),PI_CODING_AGENT_DIR=str(p/'pi'),CLAUDE_CONFIG_DIR=str(p/'claude'))
 master,slave=pty.openpty();fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,120,0,0))
 child=subprocess.Popen([os.environ['CCSW_TEST_BINARY']],env=env,stdin=slave,stdout=slave,stderr=slave);os.close(slave)
 def read():
  out=b'';end=time.monotonic()+0.5
  while time.monotonic()<end:
   if select.select([master],[],[],0.05)[0]:
    try:out+=os.read(master,65536)
    except OSError:break
  return out.decode(errors='replace')
 def key(b):os.write(master,b);return read()
 try:
  assert 'CLAUDE_ONLY' in read()
  assert 'CODEX_ONLY' in key(b'\x1b[<0;20;1M')
  assert 'PI_ONLY' in key(b'\x1b[<0;30;1M')
  assert 'CLAUDE_ONLY' in key(b'\x1b[<0;5;1M')
  key(b'\x03');child.wait(timeout=5);assert child.returncode==0
  assert (p/'config.toml').read_text()==config
  print('PASS Debian PTY: distinct Claude/Codex/Pi provider catalogs, mouse switching, no config mutation')
 finally:
  if child.poll() is None:child.kill();child.wait()
  os.close(master)
