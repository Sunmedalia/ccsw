//! Short-lived client processes. Background proxies deliberately do not use this owner.
use anyhow::{Context, Result, bail};
use std::{
    io::Read,
    ops::{Deref, DerefMut},
    process::{Child, Command, Output, Stdio},
    time::{Duration, Instant},
};

pub struct ManagedChild {
    child: Child,
    #[cfg(windows)]
    job: crate::windows::process::Job,
}
impl ManagedChild {
    pub fn spawn(command: &mut Command) -> Result<Self> {
        #[cfg(windows)]
        {
            let (child, job) = crate::windows::process::spawn(command)
                .context("cannot start Windows client; check the executable path and use a native .exe if the batch launcher rejects special characters")?;
            Ok(Self { child, job })
        }
        #[cfg(not(windows))]
        {
            Ok(Self {
                child: command.spawn()?,
            })
        }
    }
    pub fn terminate(&mut self) {
        #[cfg(windows)]
        self.job.terminate();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
impl Deref for ManagedChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for ManagedChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}
impl Drop for ManagedChild {
    fn drop(&mut self) {
        self.terminate();
    }
}

pub fn output_timeout(command: &mut Command, timeout: Duration) -> Result<Output> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = ManagedChild::spawn(command)?;
    let stdout = child.stdout.take().context("child stdout unavailable")?;
    let stderr = child.stderr.take().context("child stderr unavailable")?;
    let (tx, rx) = std::sync::mpsc::channel();
    let read = |mut pipe: Box<dyn Read + Send>, sender: std::sync::mpsc::Sender<_>, is_stdout| {
        std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let result = pipe
                .by_ref()
                .take(1024 * 1024)
                .read_to_end(&mut bytes)
                .map(|_| bytes);
            let _ = sender.send((is_stdout, result));
        });
    };
    read(Box::new(stdout), tx.clone(), true);
    read(Box::new(stderr), tx, false);
    let deadline = Instant::now() + timeout;
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            bail!(
                "client version check timed out after {} seconds",
                timeout.as_secs()
            );
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    // A launcher may exit before its descendants close the inherited pipes.
    child.terminate();
    let mut output = Output {
        status,
        stdout: Vec::new(),
        stderr: Vec::new(),
    };
    for _ in 0..2 {
        let (stdout, bytes) = rx
            .recv_timeout(deadline.saturating_duration_since(Instant::now()))
            .context("client output pipe did not close before the deadline")?;
        if stdout {
            output.stdout = bytes?;
        } else {
            output.stderr = bytes?;
        }
    }
    Ok(output)
}
