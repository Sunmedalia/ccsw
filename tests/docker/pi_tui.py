"""Exercise the real TUI in a Linux PTY, using an isolated copy of current CCSW config."""
import fcntl
import os
from pathlib import Path
import pty
import select
import shutil
import struct
import subprocess
import tempfile
import termios
import time

with tempfile.TemporaryDirectory(prefix='ccsw-pi-tui-') as directory:
    root=Path(directory)
    shutil.copyfile(Path(os.environ['CCSW_TEST_INPUT'])/'ccsw.toml',root/'config.toml')
    env=dict(os.environ,HOME=directory,TERM='xterm-256color',CCSW_CONFIG=str(root/'config.toml'),PI_CODING_AGENT_DIR=str(root/'pi'),CODEX_HOME=str(root/'codex'),XDG_STATE_HOME=str(root/'state'),XDG_CACHE_HOME=str(root/'cache'),CLAUDE_CONFIG_DIR=str(root/'claude'))
    master,slave=pty.openpty()
    fcntl.ioctl(slave,termios.TIOCSWINSZ,struct.pack('HHHH',36,120,0,0))
    process=subprocess.Popen([os.environ['CCSW_TEST_BINARY']],env=env,stdin=slave,stdout=slave,stderr=slave)
    os.close(slave)
    def read():
        data=b'';end=time.monotonic()+0.5
        while time.monotonic()<end:
            if select.select([master],[],[],0.05)[0]:
                try:data+=os.read(master,65536)
                except OSError:break
        return data.decode(errors='replace')
    def key(value):os.write(master,value);return read()
    try:
        assert 'CCSW' in read()
        assert 'Codex API' in key(b'\x1bOQ')
        assert 'Accounts' in key(b'\x1bOR')
        assert 'Pi API' in key(b'\x1b[<0;30;1M')
        assert 'Profile' in key(b'e')
        key(b'\x1b');key(b'\r')
        assert 'Edit model' in key(b'e')
        key(b'\x1b')
        assert 'Help' in key(b'?')
        key(b'\x1b')
        key(b'\x1bOQ')
        key(b'\x03')
        process.wait(timeout=5)
        assert process.returncode==0
        print('PASS Debian PTY: launch, F2 and mouse tabs from Codex Accounts to Pi, provider e, model e, Help, clean exit')
    finally:
        if process.poll() is None:process.kill();process.wait()
        os.close(master)
