//! Read-only local session statistics. Never retain prompts or response bodies.
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::{self, File},
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
    time::SystemTime,
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Tokens {
    // Input includes cache reads/writes for both clients.
    pub input: i64,
    pub output: i64,
    pub read: i64,
    pub write: i64,
    pub known: bool,
    pub cache_known: bool,
}
impl Tokens {
    pub fn total(&self) -> i64 {
        self.input.saturating_add(self.output)
    }
    pub fn cache_reuse_percent(&self) -> Option<f64> {
        (self.known
            && self.cache_known
            && self.input > 0
            && self.read >= 0
            && self.read <= self.input)
            .then(|| 100.0 * self.read as f64 / self.input as f64)
    }
    fn add(&mut self, other: &Self) {
        self.input = self.input.saturating_add(other.input);
        self.output = self.output.saturating_add(other.output);
        self.read = self.read.saturating_add(other.read);
        self.write = self.write.saturating_add(other.write);
        self.known |= other.known;
        self.cache_known |= other.cache_known;
    }
    fn parse(v: &Value, claude: bool) -> Self {
        let n = |key| v.get(key).and_then(Value::as_i64).filter(|n| *n >= 0);
        let read = n(if claude {
            "cache_read_input_tokens"
        } else {
            "cached_input_tokens"
        })
        .unwrap_or(0);
        let write = n(if claude {
            "cache_creation_input_tokens"
        } else {
            "cache_write_input_tokens"
        })
        .unwrap_or(0);
        Self {
            input: n("input_tokens").unwrap_or(0).saturating_add(if claude {
                read.saturating_add(write)
            } else {
                0
            }),
            output: n("output_tokens").unwrap_or(0),
            read,
            write,
            known: n("input_tokens").is_some() && n("output_tokens").is_some(),
            cache_known: n(if claude {
                "cache_read_input_tokens"
            } else {
                "cached_input_tokens"
            })
            .is_some(),
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Session {
    pub id: String,
    pub client: &'static str,
    pub project: String,
    pub updated: i64,
    pub tokens: Tokens,
    // Token increments keyed by the log event timestamp, for local-day charts.
    pub activity: BTreeMap<i64, i64>,
    pub models: BTreeSet<String>,
    pub child: bool,
    pub fork: bool,
    pub incomplete: bool,
}

#[derive(Default)]
struct CachedFile {
    offset: u64,
    modified: Option<SystemTime>,
    identity: Option<(u64, u64)>,
    session: Session,
    messages: BTreeMap<String, (Tokens, i64)>,
    // Codex emits cumulative totals. Keep the checkpoints so resumed logs with
    // the same session ID can be merged without a false chart spike.
    codex_totals: BTreeMap<i64, i64>,
}

impl CachedFile {
    fn observe(&mut self, v: &Value, claude: bool) {
        let s = &mut self.session;
        let timestamp = v
            .get("timestamp")
            .and_then(Value::as_str)
            .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
            .map(|t| t.timestamp());
        if let Some(t) = timestamp {
            s.updated = s.updated.max(t);
        }
        if claude {
            if let Some(id) = v.get("sessionId").and_then(Value::as_str)
                && !s.child
            {
                s.id = id.into();
            }
            if let Some(cwd) = v.get("cwd").and_then(Value::as_str) {
                s.project = cwd.into();
            }
            if v.get("type").and_then(Value::as_str) != Some("assistant") {
                return;
            }
            let Some(m) = v.get("message") else {
                return;
            };
            if let Some(model) = m.get("model").and_then(Value::as_str) {
                s.models.insert(model.into());
            }
            let Some(id) = m.get("id").and_then(Value::as_str) else {
                s.incomplete = true;
                return;
            };
            let Some(usage) = m.get("usage") else {
                s.incomplete = true;
                return;
            };
            let tokens = Tokens::parse(usage, true);
            // Multiple content blocks can repeat a message ID. Keep the most complete counters.
            let entry = self.messages.entry(id.into()).or_default();
            entry.0.input = entry.0.input.max(tokens.input);
            entry.0.output = entry.0.output.max(tokens.output);
            entry.0.read = entry.0.read.max(tokens.read);
            entry.0.write = entry.0.write.max(tokens.write);
            entry.0.known |= tokens.known;
            entry.0.cache_known |= tokens.cache_known;
            if entry.1 == 0 {
                entry.1 = timestamp.unwrap_or(0);
            }
            s.incomplete |= !tokens.known;
        } else {
            let Some(p) = v.get("payload") else {
                return;
            };
            match v.get("type").and_then(Value::as_str) {
                Some("session_meta") => {
                    if let Some(id) = p
                        .get("id")
                        .or_else(|| p.get("session_id"))
                        .and_then(Value::as_str)
                    {
                        s.id = id.into();
                    }
                    if let Some(cwd) = p.get("cwd").and_then(Value::as_str) {
                        s.project = cwd.into();
                    }
                    s.fork = p.get("forked_from_id").is_some_and(|v| !v.is_null());
                    s.child = p.get("source").is_some_and(|v| v.get("subagent").is_some())
                        || p.get("thread_source").is_some_and(|v| v == "subagent");
                }
                Some("turn_context") => {
                    if let Some(model) = p.get("model").and_then(Value::as_str) {
                        s.models.insert(model.into());
                    }
                }
                Some("event_msg")
                    if p.get("type").and_then(Value::as_str) == Some("token_count") =>
                {
                    if let Some(usage) = p
                        .pointer("/info/total_token_usage")
                        .filter(|v| v.is_object())
                    {
                        let tokens = Tokens::parse(usage, false);
                        if tokens.known {
                            let total = tokens.total();
                            if let Some(t) = timestamp {
                                let checkpoint = self.codex_totals.entry(t).or_default();
                                *checkpoint = (*checkpoint).max(total);
                            }
                            if !s.tokens.known || total >= s.tokens.total() {
                                s.tokens = tokens;
                            }
                        } else {
                            s.incomplete = true;
                        }
                    }
                }
                _ => {}
            }
        }
    }
    fn read(&mut self, path: &Path, claude: bool) -> std::io::Result<()> {
        let mut file = File::open(path)?;
        let meta = file.metadata()?;
        #[cfg(unix)]
        let identity = {
            use std::os::unix::fs::MetadataExt;
            Some((meta.dev(), meta.ino()))
        };
        #[cfg(not(unix))]
        let identity = None;
        let modified = meta.modified().ok();
        if self.offset > meta.len()
            || self.identity != identity
            || (self.offset == meta.len() && self.modified != modified)
        {
            *self = Self::default();
        }
        if self.offset == meta.len() && self.modified == modified {
            return Ok(());
        }
        self.identity = identity;
        self.modified = modified;
        self.session.client = if claude { "Claude" } else { "Codex" };
        if self.session.id.is_empty() {
            self.session.id = path
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .into();
            self.session.child = claude && path.components().any(|p| p.as_os_str() == "subagents");
            if self.session.child
                && let Some(parent) = path
                    .parent()
                    .and_then(Path::parent)
                    .and_then(Path::file_name)
            {
                self.session.id = format!("{}/{}", parent.to_string_lossy(), self.session.id);
            }
        }
        file.seek(SeekFrom::Start(self.offset))?;
        let mut reader = BufReader::new(file);
        loop {
            let mut line = Vec::new();
            // Bound allocations even if a conversation contains a huge tool result.
            let count = reader
                .by_ref()
                .take(16 * 1024 * 1024)
                .read_until(b'\n', &mut line)?;
            if count == 0 {
                break;
            }
            if !line.ends_with(b"\n") {
                if count < 16 * 1024 * 1024 {
                    break;
                } // Retry a partially written final line.
                let mut skipped = count as u64;
                loop {
                    line.clear();
                    let n = reader
                        .by_ref()
                        .take(64 * 1024)
                        .read_until(b'\n', &mut line)?;
                    skipped += n as u64;
                    if n == 0 || line.ends_with(b"\n") {
                        break;
                    }
                }
                self.offset += skipped;
                self.session.incomplete = true;
                continue;
            }
            self.offset += count as u64;
            match serde_json::from_slice::<Value>(&line) {
                Ok(v) => self.observe(&v, claude),
                Err(_) => self.session.incomplete = true,
            }
        }
        if claude {
            self.session.tokens = Tokens::default();
            self.session.activity.clear();
            for (tokens, timestamp) in self.messages.values() {
                self.session.tokens.add(tokens);
                if *timestamp > 0 && tokens.known {
                    *self.session.activity.entry(*timestamp).or_default() += tokens.total();
                }
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub struct Reader {
    files: BTreeMap<PathBuf, CachedFile>,
}
#[derive(Default)]
pub struct Snapshot {
    pub rows: Vec<Session>,
    pub warnings: usize,
}

impl Reader {
    pub fn read(&mut self, roots: &[(PathBuf, bool)]) -> Snapshot {
        let mut result = Snapshot::default();
        let mut found = BTreeMap::new();
        for (root, claude) in roots {
            discover(root, *claude, &mut found, &mut result.warnings);
        }
        // A missing/deleted log is no longer presented as current data.
        self.files.retain(|path, _| found.contains_key(path));
        let mut failed = Vec::new();
        for (path, claude) in found {
            let cached = self.files.entry(path.clone()).or_default();
            if cached.read(&path, claude).is_err() {
                result.warnings += 1;
                failed.push(path);
            }
        }
        // Never serve a previously cached snapshot after its source became unreadable.
        for path in failed {
            self.files.remove(&path);
        }
        let mut unique: BTreeMap<(&str, &str), Session> = BTreeMap::new();
        let mut codex_tokens: BTreeMap<&str, Tokens> = BTreeMap::new();
        let mut codex_totals: BTreeMap<&str, BTreeMap<i64, i64>> = BTreeMap::new();
        for cached in self.files.values() {
            let s = &cached.session;
            if s.id.is_empty() {
                continue;
            }
            let entry = unique.entry((s.client, &s.id)).or_insert_with(|| s.clone());
            if s.updated > entry.updated {
                *entry = s.clone();
            }
            if s.client == "Codex" {
                let best = codex_tokens.entry(&s.id).or_default();
                if s.tokens.known && (!best.known || s.tokens.total() >= best.total()) {
                    *best = s.tokens.clone();
                }
                for (&timestamp, &total) in &cached.codex_totals {
                    let checkpoint = codex_totals
                        .entry(&s.id)
                        .or_default()
                        .entry(timestamp)
                        .or_default();
                    *checkpoint = (*checkpoint).max(total);
                }
            }
        }
        result.rows = unique.into_values().collect();
        for session in &mut result.rows {
            if session.client != "Codex" {
                continue;
            }
            session.tokens = codex_tokens.remove(session.id.as_str()).unwrap_or_default();
            session.activity.clear();
            let mut high_water = 0;
            if let Some(checkpoints) = codex_totals.remove(session.id.as_str()) {
                for (timestamp, total) in checkpoints {
                    let delta = total.saturating_sub(high_water);
                    if delta > 0 {
                        session.activity.insert(timestamp, delta);
                    }
                    high_water = high_water.max(total);
                }
            }
        }
        result
    }
}

fn discover(dir: &Path, claude: bool, files: &mut BTreeMap<PathBuf, bool>, warnings: &mut usize) {
    let entries = match fs::read_dir(dir) {
        Ok(v) => v,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return,
        Err(_) => {
            *warnings += 1;
            return;
        }
    };
    for entry in entries {
        let Ok(entry) = entry else {
            *warnings += 1;
            continue;
        };
        let Ok(kind) = entry.file_type() else {
            *warnings += 1;
            continue;
        };
        if kind.is_dir() {
            discover(&entry.path(), claude, files, warnings);
        } else if kind.is_file() && entry.path().extension().is_some_and(|e| e == "jsonl") {
            files.insert(entry.path(), claude);
        }
    }
}

pub fn roots() -> anyhow::Result<Vec<(PathBuf, bool)>> {
    let claude = crate::claude_config::settings_path()?;
    let codex = crate::codex::home()?;
    Ok(vec![
        (claude.parent().unwrap().join("projects"), true),
        (codex.join("sessions"), false),
        (codex.join("archived_sessions"), false),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    fn append(path: &Path, value: Value) {
        let mut f = fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .unwrap();
        writeln!(f, "{value}").unwrap();
    }
    fn claude(id: &str, output: i64) -> Value {
        json!({"type":"assistant", "sessionId":"session-a", "cwd":"/work/project",
            "timestamp":"2026-09-22T10:00:00Z", "message":{"id":id,"model":"claude-test",
            "usage":{"input_tokens":10,"output_tokens":output,"cache_read_input_tokens":30,"cache_creation_input_tokens":20}}})
    }
    fn codex(input: i64, output: i64) -> Value {
        json!({"type":"event_msg","timestamp":"2026-09-22T11:00:00Z", "payload":{"type":"token_count",
            "info":{"total_token_usage":{"input_tokens":input,"cached_input_tokens":20,"output_tokens":output},
            "last_token_usage":{"input_tokens":1,"output_tokens":1}}}})
    }

    #[test]
    fn claude_deduplicates_messages_and_keeps_child_separate() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("main.jsonl");
        append(&path, claude("m1", 2));
        append(&path, claude("m1", 5));
        append(&path, claude("m2", 3));
        let child_dir = temp.path().join("session-a/subagents");
        fs::create_dir_all(&child_dir).unwrap();
        append(&child_dir.join("agent-one.jsonl"), claude("child-m1", 9));
        let mut reader = Reader::default();
        let roots = [(temp.path().into(), true)];
        let snapshot = reader.read(&roots);
        assert_eq!(snapshot.rows.len(), 2);
        let main = snapshot.rows.iter().find(|s| !s.child).unwrap();
        assert_eq!(main.id, "session-a");
        assert_eq!(
            main.tokens,
            Tokens {
                input: 120,
                output: 8,
                read: 60,
                write: 40,
                known: true,
                cache_known: true,
            }
        );
        assert_eq!(main.tokens.total(), 128);
        assert_eq!(main.tokens.cache_reuse_percent(), Some(50.0));
        assert_eq!(main.activity.values().sum::<i64>(), main.tokens.total());
        assert_eq!(main.project, "/work/project");
        assert_eq!(main.models.len(), 1);
        let again = reader.read(&roots);
        assert_eq!(
            again.rows.iter().find(|s| !s.child).unwrap().tokens,
            main.tokens
        );
        append(&path, claude("m3", 1));
        assert_eq!(
            reader
                .read(&roots)
                .rows
                .iter()
                .find(|s| !s.child)
                .unwrap()
                .tokens
                .total(),
            189
        );
    }

    #[test]
    fn codex_uses_latest_total_not_sum_and_marks_forks() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("rollout.jsonl");
        append(
            &path,
            json!({"type":"session_meta","payload":{"id":"thread-a","cwd":"/work","forked_from_id":"parent"}}),
        );
        append(
            &path,
            json!({"type":"turn_context","payload":{"model":"codex-test"}}),
        );
        append(&path, codex(100, 10));
        append(&path, codex(100, 10));
        append(&path, codex(150, 20));
        let mut reader = Reader::default();
        let roots = [(temp.path().into(), false)];
        let snapshot = reader.read(&roots);
        let row = &snapshot.rows[0];
        assert_eq!(row.tokens.total(), 170);
        assert_eq!(row.tokens.read, 20);
        assert_eq!(row.tokens.cache_reuse_percent(), Some(100.0 * 20.0 / 150.0));
        assert_eq!(row.activity.values().sum::<i64>(), 170);
        assert!(row.fork);
        assert!(row.models.contains("codex-test"));
        // An info:null event must not clear the last known total.
        append(
            &path,
            json!({"type":"event_msg","payload":{"type":"token_count","info":null}}),
        );
        assert_eq!(reader.read(&roots).rows[0].tokens.total(), 170);
    }

    #[test]
    fn codex_resumed_log_does_not_hide_usage_or_double_count_activity() {
        let temp = tempfile::tempdir().unwrap();
        let old = temp.path().join("old.jsonl");
        let resumed = temp.path().join("resumed.jsonl");
        append(
            &old,
            json!({"type":"session_meta","timestamp":"2026-09-22T10:00:00Z","payload":{"id":"thread-a","cwd":"/work"}}),
        );
        append(&old, codex(100, 10));
        let mut reader = Reader::default();
        let roots = [(temp.path().into(), false)];
        assert_eq!(reader.read(&roots).rows[0].tokens.total(), 110);

        // Codex writes metadata before the first cumulative token checkpoint.
        append(
            &resumed,
            json!({"type":"session_meta","timestamp":"2026-09-22T12:00:00Z","payload":{"id":"thread-a","cwd":"/new-work"}}),
        );
        let row = reader.read(&roots).rows.remove(0);
        assert_eq!(row.project, "/new-work");
        assert!(row.tokens.known);
        assert_eq!(row.tokens.total(), 110);
        assert_eq!(row.activity.values().sum::<i64>(), 110);

        append(
            &resumed,
            json!({"type":"event_msg","timestamp":"2026-09-22T12:01:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150,"output_tokens":20,"cached_input_tokens":30}}}}),
        );
        let row = reader.read(&roots).rows.remove(0);
        assert_eq!(row.tokens.total(), 170);
        assert_eq!(row.activity.values().sum::<i64>(), 170);
        assert_eq!(
            row.activity.get(
                &chrono::DateTime::parse_from_rfc3339("2026-09-22T12:01:00Z")
                    .unwrap()
                    .timestamp()
            ),
            Some(&60)
        );

        // A later partial total is not a reason to erase known usage.
        append(
            &resumed,
            json!({"type":"event_msg","timestamp":"2026-09-22T12:02:00Z","payload":{"type":"token_count","info":{"total_token_usage":{"input_tokens":150}}}}),
        );
        let row = reader.read(&roots).rows.remove(0);
        assert_eq!(row.tokens.total(), 170);
        assert!(row.incomplete);
    }

    #[test]
    fn partial_tail_is_retried_and_truncation_rebuilds() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("session.jsonl");
        let serialized = claude("m1", 5).to_string();
        fs::write(&path, &serialized[..30]).unwrap();
        let mut reader = Reader::default();
        let roots = [(temp.path().into(), true)];
        assert!(!reader.read(&roots).rows[0].tokens.known);
        let mut file = fs::OpenOptions::new().append(true).open(&path).unwrap();
        writeln!(file, "{}", &serialized[30..]).unwrap();
        assert_eq!(reader.read(&roots).rows[0].tokens.total(), 65);
        fs::write(&path, "bad json\n").unwrap();
        let snapshot = reader.read(&roots);
        assert!(!snapshot.rows[0].tokens.known);
        assert!(snapshot.rows[0].incomplete);
        fs::remove_file(path).unwrap();
        assert!(reader.read(&roots).rows.is_empty());
    }

    #[test]
    fn archived_copy_does_not_duplicate_a_thread_and_missing_roots_are_normal() {
        let temp = tempfile::tempdir().unwrap();
        let live = temp.path().join("live.jsonl");
        append(
            &live,
            json!({"type":"session_meta","payload":{"id":"same"}}),
        );
        append(&live, codex(100, 5));
        fs::copy(live, temp.path().join("archived.jsonl")).unwrap();
        let mut reader = Reader::default();
        let snapshot = reader.read(&[
            (temp.path().into(), false),
            (temp.path().join("absent"), true),
        ]);
        assert_eq!(snapshot.rows.len(), 1);
        assert_eq!(snapshot.warnings, 0);
        assert_eq!(snapshot.rows[0].tokens.total(), 105);
    }

    #[test]
    fn cache_reuse_requires_known_total_input_and_never_exceeds_100_percent() {
        assert_eq!(Tokens::default().cache_reuse_percent(), None);
        let missing_cache =
            Tokens::parse(&json!({"input_tokens": 100, "output_tokens": 10}), false);
        assert!(missing_cache.known);
        assert!(!missing_cache.cache_known);
        assert_eq!(missing_cache.cache_reuse_percent(), None);
        let explicit_zero = Tokens::parse(
            &json!({"input_tokens": 100, "output_tokens": 10, "cached_input_tokens": 0}),
            false,
        );
        assert!(explicit_zero.cache_known);
        assert_eq!(explicit_zero.cache_reuse_percent(), Some(0.0));
        assert_eq!(
            Tokens {
                input: 0,
                known: true,
                ..Default::default()
            }
            .cache_reuse_percent(),
            None
        );
        assert_eq!(
            Tokens {
                input: 100,
                read: 120,
                known: true,
                ..Default::default()
            }
            .cache_reuse_percent(),
            None
        );
        assert_eq!(
            Tokens {
                input: 100,
                read: 80,
                write: 15,
                known: true,
                cache_known: true,
                ..Default::default()
            }
            .cache_reuse_percent(),
            Some(80.0)
        );
    }
}
