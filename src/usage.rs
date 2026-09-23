//! Local request ledger. Never persist credentials, headers, or conversation bodies.
use anyhow::Result;
use chrono::{FixedOffset, Local, Utc};
use rusqlite::{Connection, OpenFlags, params};
use serde_json::Value;
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

pub const FILE: &str = "usage.sqlite3";

// One lazily initialized connection per daemon. Handles and in-flight tickets
// share its lock; all access happens on blocking workers, never runtime threads.
#[derive(Clone)]
pub struct Writer {
    path: PathBuf,
    connection: Arc<Mutex<Option<Connection>>>,
}

impl Writer {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            connection: Arc::new(Mutex::new(None)),
        }
    }

    fn with_connection<T>(&self, work: impl FnOnce(&Connection) -> Result<T>) -> Result<T> {
        let mut connection = self.connection.lock().unwrap_or_else(|e| e.into_inner());
        if connection.is_none() {
            *connection = Some(open(&self.path)?);
        }
        let result = work(connection.as_ref().expect("initialized usage connection"));
        // Retry initialization after an IO/SQLite error rather than keeping a
        // failed connection for the lifetime of the daemon.
        if result.is_err() {
            *connection = None;
        }
        result
    }

    pub fn recover(&self) -> Result<()> {
        self.with_connection(|db| {
            db.execute(
                "UPDATE requests SET outcome='interrupted' WHERE outcome='pending'",
                [],
            )?;
            Ok(())
        })
    }
}

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
    db.busy_timeout(Duration::from_secs(2))?;
    db.execute_batch("PRAGMA journal_mode=WAL;
        CREATE TABLE IF NOT EXISTS settings (key TEXT PRIMARY KEY, value INTEGER NOT NULL);
        CREATE TABLE IF NOT EXISTS requests (
          id TEXT PRIMARY KEY, config TEXT NOT NULL, client TEXT NOT NULL,
          provider TEXT NOT NULL, name TEXT NOT NULL, model TEXT NOT NULL,
          kind TEXT NOT NULL, started INTEGER NOT NULL, day TEXT NOT NULL,
          outcome TEXT NOT NULL, input INTEGER, output INTEGER, cache_read INTEGER, cache_write INTEGER);
        CREATE INDEX IF NOT EXISTS requests_summary ON requests(config, day, client, provider);
        CREATE INDEX IF NOT EXISTS requests_started ON requests(config, started);")?;
    db.execute(
        "INSERT OR IGNORE INTO settings VALUES ('offset', ?1)",
        [Local::now().offset().local_minus_utc()],
    )?;
    // Additive migration; old rows remain unknown rather than inventing timings.
    let tx = db.unchecked_transaction()?;
    let columns = request_columns(&tx)?;
    for column in ["cache_input", "output_ms", "request_ms"] {
        if !columns.contains(column) {
            tx.execute_batch(&format!("ALTER TABLE requests ADD COLUMN {column} INTEGER"))?;
        }
    }
    if !columns.contains("upstream_format") {
        tx.execute_batch("ALTER TABLE requests ADD COLUMN upstream_format TEXT")?;
    }
    tx.commit()?;
    Ok(db)
}

fn request_columns(db: &Connection) -> Result<std::collections::BTreeSet<String>> {
    Ok(db
        .prepare("PRAGMA table_info(requests)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<rusqlite::Result<_>>()?)
}

#[derive(Clone)]
pub struct Request {
    pub config: PathBuf,
    pub client: &'static str,
    pub provider: String,
    pub name: String,
    pub model: String,
    pub kind: &'static str,
    pub api_format: crate::config::ApiFormat,
}

pub struct Ticket {
    writer: Writer,
    id: String,
    pub outcome: &'static str,
    pub tokens: Tokens,
    cache_input: Option<i64>,
    api_format: crate::config::ApiFormat,
    request_started: Instant,
    request_ms: Option<i64>,
}

impl Ticket {
    pub async fn begin(writer: Writer, request: Request) -> Option<Self> {
        let id = uuid::Uuid::new_v4().to_string();
        let mut ticket = Self {
            writer: writer.clone(),
            id: id.clone(),
            outcome: "failed",
            tokens: Tokens::default(),
            cache_input: None,
            api_format: request.api_format,
            request_started: Instant::now(),
            request_ms: None,
        };
        let now = Utc::now();
        let result = tokio::task::spawn_blocking(move || writer.with_connection(|db| {
            let offset: i32 = db.query_row("SELECT value FROM settings WHERE key='offset'", [], |r| r.get(0))?;
            let day = now.with_timezone(&FixedOffset::east_opt(offset).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap())).format("%Y-%m-%d").to_string();
            db.prepare_cached("INSERT INTO requests (id,config,client,provider,name,model,kind,started,day,outcome,upstream_format) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,'pending',?10)")?.execute(
                params![id, request.config.to_string_lossy(), request.client, request.provider, request.name, request.model, request.kind, now.timestamp(), day, request.api_format.label()])?;
            Ok(())
        })).await;
        match result {
            Ok(Ok(())) => {
                // Start immediately before the caller sends the upstream request, excluding DB work.
                ticket.request_started = Instant::now();
                Some(ticket)
            }
            _ => {
                eprintln!(
                    "CCSW usage: could not record request; check usage database permissions or disk space"
                );
                None
            }
        }
    }

    pub fn observe_usage(&mut self, value: &Value) {
        self.tokens.observe(value);
        // The upstream protocol determines input semantics. OpenAI-compatible gateways
        // can return Anthropic-named cache fields without changing their input total.
        self.cache_input =
            self.tokens
                .input
                .zip(self.tokens.cache_read)
                .and_then(|(input, read)| {
                    let total = if self.api_format == crate::config::ApiFormat::Anthropic {
                        input
                            .checked_add(read)?
                            .checked_add(self.tokens.cache_write?)?
                    } else {
                        input
                    };
                    (total >= read).then_some(total)
                });
    }
    pub fn output_finished(&mut self) {
        // Full output usage includes hidden reasoning. Measuring only visible deltas
        // can divide thousands of tokens by a buffered final burst of a few ms.
        // This is end-to-end request throughput, not model decode speed.
        self.request_ms = i64::try_from(self.request_started.elapsed().as_millis())
            .ok()
            .filter(|ms| *ms > 0);
    }
}

impl Drop for Ticket {
    fn drop(&mut self) {
        // Keep disk IO off the async runtime; runtime shutdown waits for blocking tasks.
        let saved = (
            self.writer.clone(),
            self.id.clone(),
            self.outcome,
            self.tokens.clone(),
            self.cache_input,
            self.request_ms,
        );
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn_blocking(move || {
                if saved.0.with_connection(|db| {
                    db.prepare_cached("UPDATE requests SET outcome=?2,input=?3,output=?4,cache_read=?5,cache_write=?6,cache_input=?7,request_ms=?8 WHERE id=?1")?.execute(
                        params![saved.1, saved.2, saved.3.input, saved.3.output, saved.3.cache_read, saved.3.cache_write, saved.4, saved.5])?;
                    Ok(())
                }).is_err() {
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
                u.pointer("/input_tokens_details/cached_tokens")
                    .or_else(|| u.pointer("/prompt_tokens_details/cached_tokens"))
                    .or_else(|| u.get("prompt_cache_hit_tokens"))
                    .or_else(|| u.get("cache_read_input_tokens")),
            ),
            (
                &mut self.cache_write,
                u.pointer("/input_tokens_details/cache_write_tokens")
                    .or_else(|| u.pointer("/prompt_tokens_details/cache_write_tokens"))
                    .or_else(|| u.get("cache_creation_input_tokens")),
            ),
        ] {
            if let Some(n) = v.and_then(Value::as_i64).filter(|n| *n >= 0) {
                *slot = Some(n);
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
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
    pub cache_input: i64,
    pub cache_hits: i64,
    pub speed_output: i64,
    pub speed_ms: i64,
    pub speed_samples: i64,
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
        self.cache_input += other.cache_input;
        self.cache_hits += other.cache_hits;
        self.speed_output += other.speed_output;
        self.speed_ms += other.speed_ms;
        self.speed_samples += other.speed_samples;
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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Query {
    // Home only needs today's and all-time provider totals, not model/hour rows.
    Summary,
    // Preserve all-time model totals in undated rows; only the selected range
    // needs daily/hourly detail. Undated rows never match a date-range filter.
    Range { start: String, end: String },
    All,
}

#[cfg(test)]
pub fn snapshot(path: &Path, config: &Path) -> Result<Snapshot> {
    snapshot_for(path, config, &Query::All)
}

fn aggregate_sql(day: &str, model: &str, hour: &str, filter: &str, extended: bool) -> String {
    let extras = if extended {
        "COALESCE(SUM(CASE WHEN upstream_format IS NOT NULL AND outcome='success' AND kind='generation' AND cache_input>0 AND cache_read BETWEEN 0 AND cache_input THEN cache_input ELSE 0 END),0),
         COALESCE(SUM(CASE WHEN upstream_format IS NOT NULL AND outcome='success' AND kind='generation' AND cache_input>0 AND cache_read BETWEEN 0 AND cache_input THEN cache_read ELSE 0 END),0),
         COALESCE(SUM(CASE WHEN outcome='success' AND kind='generation' AND request_ms>0 AND output>0 THEN output ELSE 0 END),0),
         COALESCE(SUM(CASE WHEN outcome='success' AND kind='generation' AND request_ms>0 AND output>0 THEN request_ms ELSE 0 END),0),
         COALESCE(SUM(outcome='success' AND kind='generation' AND request_ms>0 AND output>0),0)"
    } else {
        "0,0,0,0,0"
    };
    format!("SELECT {day} AS day,client,provider,MAX(name) AS name,kind,COUNT(*) AS calls,
      SUM(outcome='success'),SUM(outcome='failed'),SUM(outcome='interrupted'),SUM(outcome='pending'),
      COALESCE(SUM(input),0),COALESCE(SUM(output),0),SUM(input IS NULL OR output IS NULL),
      COALESCE(SUM(cache_read),0),COALESCE(SUM(cache_write),0),{model} AS model,{hour} AS hour,{extras}
      FROM requests WHERE config=?1 {filter} GROUP BY 1,2,3,5,16,17")
}

// Keep a read connection so SQLite's data_version can detect commits, including
// WAL-only writes. No read transaction survives between refreshes.
#[derive(PartialEq, Eq)]
struct DatabaseStamp {
    len: u64,
    modified: Option<std::time::SystemTime>,
    #[cfg(unix)]
    identity: (u64, u64),
}
impl DatabaseStamp {
    fn new(metadata: &std::fs::Metadata) -> Self {
        Self {
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt;
                (metadata.dev(), metadata.ino())
            },
        }
    }
}

pub struct Reader {
    path: PathBuf,
    config: PathBuf,
    connection: Option<Connection>,
    stamp: Option<DatabaseStamp>,
    cached: Option<(Query, i64, String)>,
    offset: i32,
}
impl Reader {
    pub fn new(path: PathBuf, config: PathBuf) -> Self {
        Self {
            path,
            config,
            connection: None,
            stamp: None,
            cached: None,
            offset: Local::now().offset().local_minus_utc(),
        }
    }

    // None means the caller's last snapshot is still current.
    pub fn read(&mut self, query: &Query) -> Result<Option<Snapshot>> {
        let result = self.read_inner(query);
        if result.is_err() {
            self.connection = None;
            self.cached = None;
        }
        result
    }

    fn read_inner(&mut self, query: &Query) -> Result<Option<Snapshot>> {
        let metadata = match std::fs::metadata(&self.path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                self.connection = None;
                self.stamp = None;
                self.cached = None;
                return Ok(Some(Snapshot::default()));
            }
            Err(error) => return Err(error.into()),
        };
        let stamp = DatabaseStamp::new(&metadata);
        // Reopen after replacement/checkpoint changes; never keep serving an
        // unlinked database after an external restore on Unix.
        if self.stamp.as_ref() != Some(&stamp) {
            self.connection = None;
            self.cached = None;
            self.stamp = Some(stamp);
        }
        if self.connection.is_none() {
            let db = Connection::open_with_flags(&self.path, OpenFlags::SQLITE_OPEN_READ_ONLY)?;
            db.busy_timeout(Duration::from_millis(200))?;
            self.connection = Some(db);
        }
        let db = self.connection.as_ref().expect("initialized usage reader");
        // Capture the version before the query: a commit racing the read causes
        // another refresh, rather than being mistaken for already-read data.
        let version: i64 = db.query_row("PRAGMA data_version", [], |r| r.get(0))?;
        let zone =
            FixedOffset::east_opt(self.offset).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
        let now = Utc::now();
        let today = now.with_timezone(&zone).format("%Y-%m-%d").to_string();
        if self.cached.as_ref() == Some(&(query.clone(), version, today)) {
            return Ok(None);
        }
        let snapshot = snapshot_from(db, &self.config, query, now)?;
        self.offset = snapshot.offset;
        let zone = FixedOffset::east_opt(snapshot.offset)
            .unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
        self.cached = Some((
            query.clone(),
            version,
            now.with_timezone(&zone).format("%Y-%m-%d").to_string(),
        ));
        Ok(Some(snapshot))
    }
}

#[cfg(test)]
pub fn snapshot_for(path: &Path, config: &Path, query: &Query) -> Result<Snapshot> {
    Ok(Reader::new(path.to_owned(), config.to_owned())
        .read(query)?
        .expect("initial snapshot"))
}

fn snapshot_from(
    db: &Connection,
    config: &Path,
    query: &Query,
    now: chrono::DateTime<Utc>,
) -> Result<Snapshot> {
    // Keep metadata and both sides of a range query in one consistent snapshot.
    let db = db.unchecked_transaction()?;
    let offset: i32 = db.query_row("SELECT value FROM settings WHERE key='offset'", [], |r| {
        r.get(0)
    })?;
    let config = config.to_string_lossy();
    let since = db.query_row(
        "SELECT MIN(started) FROM requests WHERE config=?1",
        [&config],
        |r| r.get(0),
    )?;
    let mut parameters = vec![rusqlite::types::Value::Text(config.into_owned())];
    let hour = "CAST(strftime('%H', started + ?2, 'unixepoch') AS INTEGER)";
    let columns = request_columns(&db)?;
    let extended = ["cache_input", "request_ms", "upstream_format"]
        .iter()
        .all(|c| columns.contains(*c));
    let sql = match query {
        Query::Summary => {
            let zone =
                FixedOffset::east_opt(offset).unwrap_or_else(|| FixedOffset::east_opt(0).unwrap());
            parameters.push(
                now.with_timezone(&zone)
                    .format("%Y-%m-%d")
                    .to_string()
                    .into(),
            );
            aggregate_sql(
                "CASE WHEN day=?2 THEN day ELSE '' END",
                "''",
                "0",
                "",
                extended,
            )
        }
        Query::All => {
            parameters.push(offset.into());
            aggregate_sql("day", "model", hour, "", extended)
        }
        Query::Range { start, end } => {
            parameters.extend([offset.into(), start.clone().into(), end.clone().into()]);
            let detail = aggregate_sql("day", "model", hour, "AND day>=?3 AND day<=?4", extended);
            let history = aggregate_sql("''", "model", "0", "AND (day<?3 OR day>?4)", extended);
            format!("{detail} UNION ALL {history}")
        }
    };
    let mut stmt = db.prepare(&format!(
        "SELECT * FROM ({sql}) ORDER BY day DESC,client,provider,model,hour"
    ))?;
    let rows = stmt
        .query_map(rusqlite::params_from_iter(parameters), |r| {
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
                    cache_input: r.get(17)?,
                    cache_hits: r.get(18)?,
                    speed_output: r.get(19)?,
                    speed_ms: r.get(20)?,
                    speed_samples: r.get(21)?,
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

    #[tokio::test]
    async fn cache_semantics_follow_upstream_protocol_not_field_names() {
        use crate::config::ApiFormat;
        let temp = tempfile::tempdir().unwrap();
        let writer = Writer::new(temp.path().join(FILE));
        for (format, usage, expected) in [
            // Regression: OpenAI gateway aliases do not change input semantics.
            (
                ApiFormat::OpenaiChat,
                json!({"input_tokens":120675,"cache_read_input_tokens":120448,"cache_creation_input_tokens":0}),
                Some(120675),
            ),
            (
                ApiFormat::Anthropic,
                json!({"input_tokens":227,"cache_read_input_tokens":120448,"cache_creation_input_tokens":0}),
                Some(120675),
            ),
            (
                ApiFormat::OpenaiChat,
                json!({"prompt_tokens":100,"prompt_cache_hit_tokens":90,"prompt_cache_miss_tokens":10}),
                Some(100),
            ),
            (
                ApiFormat::OpenaiResponses,
                json!({"input_tokens":100,"input_tokens_details":{"cached_tokens":0}}),
                Some(100),
            ),
            (ApiFormat::OpenaiChat, json!({"prompt_tokens":100}), None),
            (
                ApiFormat::OpenaiChat,
                json!({"prompt_tokens":100,"prompt_cache_hit_tokens":101}),
                None,
            ),
            (
                ApiFormat::Anthropic,
                json!({"input_tokens":10,"cache_read_input_tokens":90}),
                None,
            ),
        ] {
            let mut req = request("Claude", "p", "generation");
            req.api_format = format;
            let mut t = Ticket::begin(writer.clone(), req).await.unwrap();
            t.observe_usage(&json!({"usage":usage}));
            t.observe_usage(&json!({"usage":{"output_tokens":42}}));
            assert_eq!(t.cache_input, expected);
            if usage.get("prompt_cache_miss_tokens").is_some() {
                assert_eq!(
                    t.tokens.cache_write, None,
                    "cache misses are not cache writes"
                );
                assert_eq!(t.tokens.cache_read, Some(90));
            }
        }
    }

    #[tokio::test]
    async fn historical_metrics_and_incomplete_calls_do_not_pollute_new_rates() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        let writer = Writer::new(path.clone());
        for (provider, kind, outcome) in [
            ("valid", "generation", "success"),
            ("failed", "generation", "failed"),
            ("interrupted", "generation", "interrupted"),
            ("compact", "compact", "success"),
            ("legacy", "generation", "success"),
        ] {
            let mut req = request("Codex", provider, kind);
            req.api_format = crate::config::ApiFormat::OpenaiChat;
            let mut t = Ticket::begin(writer.clone(), req).await.unwrap();
            t.observe_usage(&json!({"usage":{"prompt_tokens":100,"completion_tokens":50,"prompt_cache_hit_tokens":90}}));
            t.request_ms = Some(2000);
            t.outcome = outcome;
        }
        settled(&path, 5).await;
        let db = Connection::open(&path).unwrap();
        db.execute("UPDATE requests SET upstream_format=NULL,request_ms=NULL,output_ms=1,cache_input=190 WHERE provider='legacy'", []).unwrap();
        let total = snapshot(&path, Path::new("test-config")).unwrap().total(
            None,
            None,
            None,
            "generation",
        );
        assert_eq!(total.calls, 4, "historical usage remains visible");
        assert_eq!((total.cache_hits, total.cache_input), (90, 100));
        assert_eq!(
            (total.speed_output, total.speed_ms, total.speed_samples),
            (50, 2000, 1)
        );
    }

    #[tokio::test]
    async fn cache_denominators_and_stream_speed_use_matching_samples() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        let writer = Writer::new(path.clone());
        for (format, usage, duration, outcome) in [
            (
                crate::config::ApiFormat::Anthropic,
                json!({"input_tokens":100,"cache_read_input_tokens":300,"cache_creation_input_tokens":100,"output_tokens":40}),
                Some(1000),
                "success",
            ),
            (
                crate::config::ApiFormat::OpenaiResponses,
                json!({"input_tokens":500,"input_tokens_details":{"cached_tokens":200},"output_tokens":60}),
                Some(3000),
                "success",
            ),
            (
                crate::config::ApiFormat::OpenaiChat,
                json!({"input_tokens":500,"output_tokens":9999}),
                None,
                "success",
            ),
            (
                crate::config::ApiFormat::OpenaiChat,
                json!({"input_tokens":500,"output_tokens":9999}),
                Some(1),
                "failed",
            ),
        ] {
            let mut req = request("Claude", "p", "generation");
            req.api_format = format;
            let mut t = Ticket::begin(writer.clone(), req).await.unwrap();
            t.observe_usage(&json!({"usage":usage}));
            // Partial cumulative events must preserve protocol/cache metadata.
            t.observe_usage(&json!({"usage":{"output_tokens":t.tokens.output}}));
            t.request_ms = duration;
            t.outcome = outcome;
            drop(t);
        }
        let t = settled(&path, 4)
            .await
            .total(None, None, None, "generation");
        assert_eq!((t.cache_hits, t.cache_input), (500, 1000));
        assert_eq!(
            (t.speed_output, t.speed_ms, t.speed_samples),
            (100, 4000, 2)
        );
    }

    #[test]
    fn legacy_ledger_is_readable_and_migration_is_additive() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        seed_history(&path, 4);
        let db = Connection::open(&path).unwrap();
        db.execute_batch("ALTER TABLE requests DROP COLUMN cache_input; ALTER TABLE requests DROP COLUMN output_ms;").unwrap();
        let before = snapshot(&path, Path::new("test-config")).unwrap().total(
            None,
            None,
            None,
            "generation",
        );
        assert_eq!(before.speed_samples, 0);
        assert_eq!(before.cache_input, 0);
        drop(open(&path).unwrap());
        let after = snapshot(&path, Path::new("test-config")).unwrap().total(
            None,
            None,
            None,
            "generation",
        );
        assert_eq!(before, after);
    }

    pub fn request(client: &'static str, provider: &str, kind: &'static str) -> Request {
        Request {
            api_format: crate::config::ApiFormat::Anthropic,
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
                let snapshot = match snapshot(path, config) {
                    Ok(snapshot) => snapshot,
                    Err(_) => {
                        // The async writer may still hold SQLite's schema lock.
                        tokio::time::sleep(Duration::from_millis(10)).await;
                        continue;
                    }
                };
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
    fn seed_history(path: &Path, count: i64) {
        let db = open(path).unwrap();
        // Multiple clients/providers/models, 365 days, all outcome/token states,
        // plus unrelated config data: range compression must preserve each one.
        db.execute("WITH RECURSIVE seq(n) AS (SELECT 0 UNION ALL SELECT n+1 FROM seq WHERE n+1<?1)
            INSERT INTO requests (id,config,client,provider,name,model,kind,started,day,outcome,input,output,cache_read,cache_write)
            SELECT CAST(n AS TEXT), CASE WHEN n%11=0 THEN 'other-config' ELSE 'test-config' END,
              CASE WHEN n%2=0 THEN 'Claude' ELSE 'Codex' END,
              'p' || (n%5), 'Provider ' || (n%5), 'm' || (n%17),
              CASE WHEN n%7=0 THEN 'compact' ELSE 'generation' END,
              unixepoch('now','start of day') - (n%365)*86400 + (n%24)*3600,
              date('now', '-' || (n%365) || ' days'),
              CASE n%4 WHEN 0 THEN 'success' WHEN 1 THEN 'failed' WHEN 2 THEN 'interrupted' ELSE 'pending' END,
              CASE WHEN n%3=0 THEN NULL ELSE 10 END, 3, 2, 1 FROM seq", [count]).unwrap();
        db.execute("UPDATE settings SET value=0 WHERE key='offset'", [])
            .unwrap();
    }

    #[test]
    fn compressed_queries_preserve_range_and_lifetime_totals() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        seed_history(&path, 4000);
        let config = Path::new("test-config");
        let all = snapshot(&path, config).unwrap();
        let today = all.today();
        let start = (chrono::NaiveDate::parse_from_str(&today, "%Y-%m-%d").unwrap()
            - chrono::Duration::days(6))
        .to_string();
        let range = snapshot_for(
            &path,
            config,
            &Query::Range {
                start: start.clone(),
                end: today.clone(),
            },
        )
        .unwrap();
        let summary = snapshot_for(&path, config, &Query::Summary).unwrap();
        for reduced in [&range, &summary] {
            assert_eq!(reduced.since, all.since);
            assert_eq!(reduced.offset, all.offset);
            assert!(reduced.rows.len() < all.rows.len());
            for client in [None, Some("Claude"), Some("Codex")] {
                for provider in [None, Some("p0"), Some("p1")] {
                    for kind in ["generation", "compact"] {
                        assert_eq!(
                            reduced.total(client, provider, None, kind),
                            all.total(client, provider, None, kind)
                        );
                        assert_eq!(
                            reduced.total(client, provider, Some(&today), kind),
                            all.total(client, provider, Some(&today), kind)
                        );
                    }
                }
            }
        }
        // Model lifetime totals and hourly chart bins must survive compression.
        for original in &all.rows {
            let sum = |snapshot: &Snapshot| {
                let mut total = Totals::default();
                for row in &snapshot.rows {
                    if row.client == original.client
                        && row.provider == original.provider
                        && row.model == original.model
                        && row.kind == original.kind
                    {
                        total.add(&row.totals);
                    }
                }
                total
            };
            assert_eq!(sum(&range), sum(&all));
        }
        let detailed: Vec<_> = all
            .rows
            .iter()
            .filter(|r| r.day >= start && r.day <= today)
            .collect();
        let selected: Vec<_> = range.rows.iter().filter(|r| !r.day.is_empty()).collect();
        assert_eq!(detailed.len(), selected.len());
        for (a, b) in detailed.into_iter().zip(selected) {
            assert_eq!(
                (
                    &a.day,
                    &a.client,
                    &a.provider,
                    &a.model,
                    a.hour,
                    &a.kind,
                    &a.totals
                ),
                (
                    &b.day,
                    &b.client,
                    &b.provider,
                    &b.model,
                    b.hour,
                    &b.kind,
                    &b.totals
                )
            );
        }
        assert!(
            snapshot_for(&path, Path::new("missing-config"), &Query::Summary)
                .unwrap()
                .rows
                .is_empty()
        );
        let empty_range = snapshot_for(
            &path,
            config,
            &Query::Range {
                start: "1900-01-01".into(),
                end: "1900-01-02".into(),
            },
        )
        .unwrap();
        assert!(empty_range.rows.iter().all(|r| r.day.is_empty()));
        assert_eq!(
            empty_range.total(None, None, None, "generation"),
            all.total(None, None, None, "generation")
        );
    }

    #[test]
    fn reader_skips_unchanged_data_and_refreshes_after_wal_commits() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        let mut reader = Reader::new(path.clone(), "test-config".into());
        assert!(
            reader
                .read(&Query::Summary)
                .unwrap()
                .unwrap()
                .rows
                .is_empty()
        );
        seed_history(&path, 100);
        // Keep the writer open to ensure the next update stays in the WAL.
        let db = Connection::open(&path).unwrap();
        db.pragma_update(None, "wal_autocheckpoint", 0).unwrap();
        let before = reader.read(&Query::Summary).unwrap().unwrap();
        assert!(reader.read(&Query::Summary).unwrap().is_none());
        db.execute("UPDATE requests SET input=42", []).unwrap();
        let after = reader.read(&Query::Summary).unwrap().unwrap();
        assert!(
            after.total(None, None, None, "generation").input
                > before.total(None, None, None, "generation").input
        );
        assert!(reader.read(&Query::Summary).unwrap().is_none());
        assert!(reader.read(&Query::All).unwrap().is_some());
        assert!(reader.read(&Query::All).unwrap().is_none());
        // Date is part of the cache key even when no requests arrive overnight.
        reader.cached.as_mut().unwrap().2 = "1900-01-01".into();
        assert!(reader.read(&Query::All).unwrap().is_some());
        db.execute("DROP TABLE requests", []).unwrap();
        assert!(reader.read(&Query::All).is_err());
        drop(db);
        let db = open(&path).unwrap();
        assert!(reader.read(&Query::All).unwrap().unwrap().rows.is_empty());
        drop(db);
    }

    #[cfg(unix)]
    #[test]
    fn reader_reopens_replaced_database() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(FILE);
        seed_history(&path, 100);
        // Switch both fixtures to DELETE mode so replacing the database does
        // not accidentally associate the old WAL with the replacement file.
        let db = Connection::open(&path).unwrap();
        db.execute_batch("PRAGMA journal_mode=DELETE").unwrap();
        drop(db);
        let mut reader = Reader::new(path.clone(), "test-config".into());
        assert!(
            !reader
                .read(&Query::Summary)
                .unwrap()
                .unwrap()
                .rows
                .is_empty()
        );
        let replacement = temp.path().join("replacement.sqlite3");
        let db = open(&replacement).unwrap();
        db.execute_batch("PRAGMA journal_mode=DELETE").unwrap();
        drop(db);
        std::fs::rename(&replacement, &path).unwrap();
        assert!(
            reader
                .read(&Query::Summary)
                .unwrap()
                .unwrap()
                .rows
                .is_empty()
        );
        std::fs::remove_file(&path).unwrap();
        assert!(
            reader
                .read(&Query::Summary)
                .unwrap()
                .unwrap()
                .rows
                .is_empty()
        );
    }

    #[tokio::test]
    async fn writer_retries_initialization_and_preserves_inflight_requests() {
        let temp = tempfile::tempdir().unwrap();
        let parent = temp.path().join("blocked");
        std::fs::write(&parent, b"not a directory").unwrap();
        let path = parent.join(FILE);
        let writer = Writer::new(path.clone());
        assert!(
            Ticket::begin(writer.clone(), request("Claude", "p", "generation"))
                .await
                .is_none()
        );
        std::fs::remove_file(&parent).unwrap();
        let first = Ticket::begin(writer.clone(), request("Claude", "p", "generation"))
            .await
            .unwrap();
        let second = Ticket::begin(writer.clone(), request("Claude", "p", "generation"))
            .await
            .unwrap();
        assert_eq!(
            snapshot(&path, Path::new("test-config"))
                .unwrap()
                .total(None, None, None, "generation")
                .pending,
            2
        );
        drop(first);
        drop(second);
        let total = settled(&path, 2)
            .await
            .total(None, None, None, "generation");
        assert_eq!((total.failed, total.interrupted), (2, 0));
        drop(writer);
        // In particular, Windows must be able to remove a closed writer's files.
        std::fs::remove_file(&path).unwrap();
    }

    #[test]
    #[ignore = "manual large-ledger benchmark"]
    fn large_ledger_query_benchmark() {
        for count in [100_000, 1_000_000] {
            let temp = tempfile::tempdir().unwrap();
            let path = temp.path().join(FILE);
            seed_history(&path, count);
            let today = Utc::now().format("%Y-%m-%d").to_string();
            for query in [
                Query::All,
                Query::Summary,
                Query::Range {
                    start: today.clone(),
                    end: today,
                },
            ] {
                let mut reader = Reader::new(path.clone(), "test-config".into());
                let started = std::time::Instant::now();
                let snapshot = reader.read(&query).unwrap().unwrap();
                eprintln!(
                    "{count} records, {query:?}: {:?}, {} rows",
                    started.elapsed(),
                    snapshot.rows.len()
                );
                let started = std::time::Instant::now();
                assert!(reader.read(&query).unwrap().is_none());
                eprintln!("unchanged refresh: {:?}", started.elapsed());
            }
        }
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
        Writer::new(path.clone()).recover().unwrap();
        let writer = Writer::new(path.clone());
        let mut jobs = vec![];
        for i in 0..16 {
            let writer = writer.clone();
            jobs.push(tokio::spawn(async move {
                let mut ticket = Ticket::begin(
                    writer,
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
            Ticket::begin(
                Writer::new(path.clone()),
                request("Codex", "same-id", "compact"),
            )
            .await
            .unwrap(),
        );
        drop(
            Ticket::begin(
                Writer::new(path.clone()),
                request("Claude", "other", "generation"),
            )
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
        Writer::new(path.clone()).recover().unwrap();
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
            let mut ticket = Ticket::begin(Writer::new(path.clone()), request)
                .await
                .unwrap();
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
