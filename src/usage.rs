//! Local request ledger. Never persist credentials, headers, or conversation bodies.
use anyhow::Result;
use chrono::{FixedOffset, Local, Utc};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    time::Duration,
};

pub const FILE: &str = "usage.sqlite3";

fn open(path: &Path) -> Result<Connection> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut options = std::fs::OpenOptions::new();
    options.create(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(path)?;
    crate::config::set_private(path)?;
    drop(file);
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_millis(500))?;
    db.execute_batch("PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS requests (
          id TEXT PRIMARY KEY, config TEXT NOT NULL, client TEXT NOT NULL,
          provider TEXT NOT NULL, name TEXT NOT NULL, model TEXT NOT NULL,
          kind TEXT NOT NULL, started INTEGER NOT NULL, day TEXT NOT NULL,
          outcome TEXT NOT NULL, input INTEGER, output INTEGER, cache_read INTEGER, cache_write INTEGER);
        CREATE INDEX IF NOT EXISTS requests_summary ON requests(config, day, client, provider);")?;
    db.execute(
        "INSERT OR IGNORE INTO settings VALUES ('offset', ?1)",
        [Local::now().offset().local_minus_utc()],
    )?;
    Ok(db)
}

pub fn recover(path: &Path) -> Result<()> {
    let db = open(path)?;
    db.execute(
        "UPDATE requests SET outcome='interrupted' WHERE outcome='pending'",
        [],
    )?;
    Ok(())
}

#[derive(Clone)]
pub struct Request {
    pub config: PathBuf,
    pub client: &'static str,
    pub provider: String,
    pub name: String,
    pub model: String,
    pub kind: &'static str,
}

pub struct Ticket {
    path: PathBuf,
    id: String,
    pub outcome: &'static str,
    pub tokens: Tokens,
}

impl Ticket {
    pub async fn begin(path: PathBuf, request: Request) -> Option<Self> {
        let id = uuid::Uuid::new_v4().to_string();
        let ticket = Self {
            path: path.clone(),
            id: id.clone(),
            outcome: "failed",
            tokens: Tokens::default(),
        };
        let now = Utc::now();
        let result = tokio::task::spawn_blocking(move || -> Result<()> {
            let db = open(&path)?;
            let offset: i32 = db.query_row("SELECT value FROM settings WHERE key='offset'", [], |r| r.get(0))?;
            let day = now.with_timezone(&FixedOffset::east_opt(offset).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap())).format("%Y-%m-%d").to_string();
            db.execute("INSERT INTO requests (id,config,client,provider,name,model,kind,started,day,outcome) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending')",
                params![id, request.config.to_string_lossy(), request.client, request.provider, request.name, request.model, request.kind, now.timestamp(), day])?;
            Ok(())
        }).await;
        match result {
            Ok(Ok(())) => Some(ticket),
            _ => {
                eprintln!(
                    "CCSW usage: could not record request; check usage database permissions or disk space"
                );
                None
            }
        }
    }
}

fn finish(path: &Path, id: &str, outcome: &str, tokens: &Tokens) -> Result<()> {
    let db = Connection::open(path)?;
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute("UPDATE requests SET outcome=?2,input=?3,output=?4,cache_read=?5,cache_write=?6 WHERE id=?1",
            params![id, outcome, tokens.input, tokens.output, tokens.cache_read, tokens.cache_write])?;
    Ok(())
}

impl Drop for Ticket {
    fn drop(&mut self) {
        // Keep disk IO off the async runtime; runtime shutdown waits for blocking tasks.
        let saved = (
            self.path.clone(),
            self.id.clone(),
            self.outcome,
            self.tokens.clone(),
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(move || {
                if finish(&saved.0, &saved.1, saved.2, &saved.3).is_err() {
                    eprintln!("CCSW usage: could not finalize request statistics");
                }
            });
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Tokens {
    pub input: Option<i64>,
    pub output: Option<i64>,
    pub cache_read: Option<i64>,
    pub cache_write: Option<i64>,
}
impl Tokens {
    pub fn observe(&mut self, value: &Value) {
        let usage = value
            .get("usage")
            .or_else(|| value.pointer("/message/usage"))
            .or_else(|| value.pointer("/response/usage"));
        let Some(u) = usage else {
            return;
        };
        // Stream usage fields are cumulative, not deltas. Do not sum repeated events.
        for (slot, v) in [
            (
                &mut self.input,
                u.get("input_tokens").or_else(|| u.get("prompt_tokens")),
            ),
            (
                &mut self.output,
                u.get("output_tokens")
                    .or_else(|| u.get("completion_tokens")),
            ),
            (
                &mut self.cache_read,
                u.get("cache_read_input_tokens")
                    .or_else(|| u.pointer("/input_tokens_details/cached_tokens"))
                    .or_else(|| u.pointer("/prompt_tokens_details/cached_tokens")),
            ),
            (&mut self.cache_write, u.get("cache_creation_input_tokens")),
        ] {
            if let Some(n) = v.and_then(Value::as_i64).filter(|n| *n >= 0) {
                *slot = Some(n);
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Totals {
    pub calls: i64,
    pub success: i64,
    pub failed: i64,
    pub interrupted: i64,
    pub pending: i64,
    pub input: i64,
    pub output: i64,
    pub unknown: i64,
    pub cache_read: i64,
    pub cache_write: i64,
}
impl Totals {
    pub fn add(&mut self, other: &Self) {
        self.calls += other.calls;
        self.success += other.success;
        self.failed += other.failed;
        self.interrupted += other.interrupted;
        self.pending += other.pending;
        self.input += other.input;
        self.output += other.output;
        self.unknown += other.unknown;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }
    pub fn tokens_label(&self) -> String {
        if self.calls == 0 {
            return "0".into();
        }
        if self.unknown == self.calls {
            return "unknown".into();
        }
        format!(
            "{}{}",
            self.input + self.output,
            if self.unknown > 0 { " + ?" } else { "" }
        )
    }
}

#[derive(Clone, Debug)]
pub struct Row {
    pub hour: u32,
    pub model: String,
    pub day: String,
    pub client: String,
    pub provider: String,
    pub name: String,
    pub kind: String,
    pub totals: Totals,
}
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub rows: Vec<Row>,
    pub offset: i32,
    pub since: Option<i64>,
}
impl Default for Snapshot {
    fn default() -> Self {
        Self {
            rows: vec![],
            offset: Local::now().offset().local_minus_utc(),
            since: None,
        }
    }
}
impl Snapshot {
    pub fn today(&self) -> String {
        Utc::now()
            .with_timezone(
                &FixedOffset::east_opt(self.offset)
                    .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap()),
            )
            .format("%Y-%m-%d")
            .to_string()
    }
    pub fn total(
        &self,
        client: Option<&str>,
        provider: Option<&str>,
        day: Option<&str>,
        kind: &str,
    ) -> Totals {
        let mut total = Totals::default();
        for row in &self.rows {
            if client.is_none_or(|v| v == row.client)
                && provider.is_none_or(|v| v == row.provider)
                && day.is_none_or(|v| v == row.day)
                && kind == row.kind
            {
                total.add(&row.totals);
            }
        }
        total
    }
}

pub fn snapshot(path: &Path, config: &Path) -> Result<Snapshot> {
    if !path.exists() {
        return Ok(Snapshot {
            offset: Local::now().offset().local_minus_utc(),
            ..Default::default()
        });
    }
    let db = Connection::open_with_flags(path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    db.busy_timeout(Duration::from_millis(200))?;
    let offset = db.query_row("SELECT value FROM settings WHERE key='offset'", [], |r| {
        r.get(0)
    })?;
    let config = config.to_string_lossy();
    let since = db.query_row(
        "SELECT MIN(started) FROM requests WHERE config=?1",
        [&config],
        |r| r.get(0),
    )?;
    let mut stmt = db.prepare("SELECT day,client,provider,MAX(name),kind,COUNT(*),
      SUM(outcome='success'),SUM(outcome='failed'),SUM(outcome='interrupted'),SUM(outcome='pending'),
      COALESCE(SUM(input),0),COALESCE(SUM(output),0),SUM(input IS NULL OR output IS NULL),
      COALESCE(SUM(cache_read),0),COALESCE(SUM(cache_write),0),model,
      CAST(strftime('%H', started + ?2, 'unixepoch') AS INTEGER) AS hour
      FROM requests WHERE config=?1 GROUP BY day,client,provider,kind,model,hour ORDER BY day DESC,client,provider,model,hour")?;
    let rows = stmt
        .query_map(params![&config, offset], |r| {
            Ok(Row {
                hour: r.get(16)?,
                model: r.get(15)?,
                day: r.get(0)?,
                client: r.get(1)?,
                provider: r.get(2)?,
                name: r.get(3)?,
                kind: r.get(4)?,
                totals: Totals {
                    calls: r.get(5)?,
                    success: r.get(6)?,
                    failed: r.get(7)?,
                    interrupted: r.get(8)?,
                    pending: r.get(9)?,
                    input: r.get(10)?,
                    output: r.get(11)?,
                    unknown: r.get(12)?,
                    cache_read: r.get(13)?,
                    cache_write: r.get(14)?,
                },
            })
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    Ok(Snapshot {
        rows,
        offset,
        since,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use serde_json::json;

    pub fn request(client: &'static str, provider: &str, kind: &'static str) -> Request {
        Request {
            config: PathBuf::from("test-config"),
            client,
            provider: provider.into(),
            name: provider.into(),
            model: "test-model".into(),
            kind,
        }
    }
    pub async fn settled(path: &Path, calls: i64) -> Snapshot {
        settled_for(path, Path::new("test-config"), calls).await
    }
    pub async fn settled_for(path: &Path, config: &Path, calls: i64) -> Snapshot {
        tokio::time::timeout(Duration::from_secs(5), async {
            loop {
                let snapshot = snapshot(path, config).unwrap();
                let count: i64 = snapshot.rows.iter().map(|r| r.totals.calls).sum();
                if count == calls && snapshot.rows.iter().all(|r| r.totals.pending == 0) {
                    break snapshot;
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await
        .expect("ledger writes must finish")
    }
    #[test]
    fn cumulative_usage_merges_partial_fields_and_preserves_unknown() {
        let mut tokens = Tokens::default();
        tokens.observe(&json!({"message":{"usage":{"input_tokens":10,"cache_read_input_tokens":20,"cache_creation_input_tokens":5}}}));
        tokens.observe(&json!({"usage":{"output_tokens":2}}));
        tokens.observe(&json!({"usage":{"output_tokens":7}}));
        tokens.observe(&json!({"usage":{"output_tokens":null,"input_tokens":-1}}));
        assert_eq!(
            tokens,
            Tokens {
                input: Some(10),
                output: Some(7),
                cache_read: Some(20),
                cache_write: Some(5)
            }
        );
        let mut chat = Tokens::default();
        chat.observe(&json!({"usage":{"prompt_tokens":8,"completion_tokens":0,"prompt_tokens_details":{"cached_tokens":4}}}));
        assert_eq!(chat.input, Some(8));
        assert_eq!(chat.output, Some(0));
        assert_eq!(chat.cache_read, Some(4));
        assert_eq!(Tokens::default().input, None);
    }
    #[tokio::test]
    async fn persistent_ledger_scopes_clients_providers_days_and_compaction() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        recover(&path).unwrap();
        let mut jobs = vec![];
        for i in 0..16 {
            let path = path.clone();
            jobs.push(tokio::spawn(async move {
                let mut ticket = Ticket::begin(
                    path,
                    request(
                        if i % 2 == 0 { "Claude" } else { "Codex" },
                        "same-id",
                        "generation",
                    ),
                )
                .await
                .unwrap();
                ticket.outcome = "success";
                ticket.tokens = Tokens {
                    input: Some(10),
                    output: Some(2),
                    ..Default::default()
                };
            }));
        }
        for job in jobs {
            job.await.unwrap();
        }
        drop(
            Ticket::begin(path.clone(), request("Codex", "same-id", "compact"))
                .await
                .unwrap(),
        );
        drop(
            Ticket::begin(path.clone(), request("Claude", "other", "generation"))
                .await
                .unwrap(),
        );
        let snapshot = settled(&path, 18).await;
        assert_eq!(
            snapshot
                .total(Some("Claude"), Some("same-id"), None, "generation")
                .calls,
            8
        );
        let total = snapshot.total(None, None, None, "generation");
        assert_eq!(
            (
                total.calls,
                total.success,
                total.failed,
                total.input,
                total.output,
                total.unknown
            ),
            (17, 16, 1, 160, 32, 1)
        );
        assert_eq!(snapshot.total(None, None, None, "compact").failed, 1);
        assert_eq!(total.tokens_label(), "192 + ?");
        let db = Connection::open(&path).unwrap();
        db.execute(
            "UPDATE requests SET day='2000-01-01' WHERE client='Codex'",
            [],
        )
        .unwrap();
        drop(db);
        let reloaded = super::snapshot(&path, Path::new("test-config")).unwrap();
        assert_eq!(
            reloaded
                .total(None, None, Some("2000-01-01"), "generation")
                .calls,
            8
        );
        assert_eq!(reloaded.total(None, None, None, "generation").calls, 17);
        assert!(
            super::snapshot(&path, Path::new("different-config"))
                .unwrap()
                .rows
                .is_empty()
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn restart_marks_pending_interrupted_and_preserves_fixed_offset() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        let db = open(&path).unwrap();
        db.execute("UPDATE settings SET value=28800 WHERE key='offset'", [])
            .unwrap();
        db.execute("INSERT INTO requests (id,config,client,provider,name,model,kind,started,day,outcome) VALUES ('r','test-config','Claude','p','p','m','generation',0,'1970-01-01','pending')", []).unwrap();
        drop(db);
        recover(&path).unwrap();
        let s = snapshot(&path, Path::new("test-config")).unwrap();
        assert_eq!(s.offset, 28800);
        assert_eq!(s.rows[0].hour, 8);
        assert_eq!(s.total(None, None, None, "generation").interrupted, 1);
        assert_eq!(
            s.total(None, None, None, "generation").tokens_label(),
            "unknown"
        );
    }

    #[tokio::test]
    async fn model_breakdown_preserves_provider_client_and_daily_totals() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        for (client, provider, model, kind) in [
            ("Claude", "p1", "model-a", "generation"),
            ("Claude", "p1", "model-a", "generation"),
            ("Claude", "p1", "model-b", "generation"),
            ("Claude", "p2", "model-a", "generation"),
            ("Codex", "p1", "model-a", "generation"),
            ("Codex", "p1", "model-a", "compact"),
        ] {
            let mut request = request(client, provider, kind);
            request.model = model.into();
            let mut ticket = Ticket::begin(path.clone(), request).await.unwrap();
            ticket.outcome = "success";
            ticket.tokens = Tokens {
                input: Some(10),
                output: Some(3),
                ..Default::default()
            };
        }
        let s = settled(&path, 6).await;
        assert_eq!(s.rows.len(), 5);
        let a = s
            .rows
            .iter()
            .find(|r| r.client == "Claude" && r.provider == "p1" && r.model == "model-a")
            .unwrap();
        assert_eq!(
            (a.totals.calls, a.totals.input, a.totals.output),
            (2, 20, 6)
        );
        assert_eq!(s.total(None, None, None, "generation").calls, 5);
        assert_eq!(
            s.total(Some("Claude"), Some("p1"), Some(&s.today()), "generation")
                .calls,
            3
        );
        assert_eq!(s.total(None, None, None, "compact").calls, 1);
    }
}
