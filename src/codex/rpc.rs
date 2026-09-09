//! Bounded stdio App Server client. Never logs RPC payloads or credentials.
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    io::{BufRead, BufReader, Read, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub struct Client {
    child: Child,
    input: ChildStdin,
    output: mpsc::Receiver<Result<Value>>,
    sequence: u64,
    pending: Vec<Value>,
}
impl Client {
    pub fn start(home: &Path) -> Result<Self> {
        let binary = std::env::var_os("CCSW_CODEX_BIN").unwrap_or_else(|| "codex".into());
        let mut child = Command::new(binary)
            .args([
                "app-server",
                "--stdio",
                "-c",
                "cli_auth_credentials_store=\"file\"",
            ])
            .current_dir(home)
            .env("CODEX_HOME", home)
            .env_remove("OPENAI_API_KEY")
            .env_remove("CODEX_ACCESS_TOKEN")
            .env_remove("CODEX_AUTH")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .context("Cannot start Codex; install Codex CLI or set CCSW_CODEX_BIN")?;
        let input = child.stdin.take().context("Codex stdin unavailable")?;
        let out = child.stdout.take().context("Codex stdout unavailable")?;
        let (sender, output) = mpsc::channel();
        std::thread::spawn(move || {
            let mut reader = BufReader::new(out);
            loop {
                let mut bytes = Vec::new();
                let result = reader
                    .by_ref()
                    .take(4 * 1024 * 1024 + 1)
                    .read_until(b'\n', &mut bytes);
                match result {
                    Ok(0) | Err(_) => break,
                    Ok(n) if n > 4 * 1024 * 1024 => {
                        let _ =
                            sender.send(Err(anyhow::anyhow!("Codex response exceeds size limit")));
                        break;
                    }
                    _ => {
                        let parsed =
                            serde_json::from_slice(&bytes).context("Invalid Codex RPC response");
                        if sender.send(parsed).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        let mut client = Self {
            child,
            input,
            output,
            sequence: 0,
            pending: vec![],
        };
        client.call("initialize", json!({"clientInfo":{"name":"ccsw","title":"CCSW","version":env!("CARGO_PKG_VERSION")},"capabilities":{"experimentalApi":true}}))?;
        client.send(json!({"method":"initialized"}))?;
        Ok(client)
    }
    fn send(&mut self, value: Value) -> Result<()> {
        serde_json::to_writer(&mut self.input, &value)?;
        self.input.write_all(b"\n")?;
        self.input.flush()?;
        Ok(())
    }
    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.sequence += 1;
        let id = self.sequence;
        self.send(json!({"id":id,"method":method,"params":params}))?;
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let value = self
                .output
                .recv_timeout(deadline.saturating_duration_since(Instant::now()))
                .context("Codex RPC timed out or disconnected")??;
            if value["id"].as_u64() == Some(id) && value.get("method").is_none() {
                if value.get("error").is_some() {
                    bail!(
                        "Codex rejected {method}; check login, client version and managed policies"
                    );
                }
                return Ok(value["result"].clone());
            }
            // Notifications can arrive before the login response.
            if self.pending.len() < 256 {
                self.pending.push(value);
            }
        }
    }
    pub fn wait_login(&mut self, login_id: &str, cancel: &AtomicBool) -> Result<()> {
        let deadline = Instant::now() + Duration::from_secs(300);
        loop {
            if cancel.load(Ordering::Relaxed) || Instant::now() >= deadline {
                let _ = self.call("account/login/cancel", json!({"loginId":login_id}));
                bail!("Login cancelled or timed out");
            }
            let value = if !self.pending.is_empty() {
                self.pending.remove(0)
            } else {
                match self.output.recv_timeout(Duration::from_millis(100)) {
                    Ok(value) => value?,
                    Err(mpsc::RecvTimeoutError::Timeout) => continue,
                    Err(_) => bail!("Codex disconnected during login"),
                }
            };
            if value["method"] == "account/login/completed"
                && value["params"]["loginId"] == login_id
            {
                if value["params"]["success"] == true {
                    return Ok(());
                }
                bail!("Codex login failed; retry browser login");
            }
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
