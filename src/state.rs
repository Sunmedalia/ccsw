use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
};

use anyhow::{Context, Result};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use tempfile::NamedTempFile;

use crate::config::set_private;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct State {
    #[serde(default)]
    pub projects: BTreeMap<String, ProjectState>,
    #[serde(default)]
    pub sessions: BTreeMap<String, SessionRecord>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProjectState {
    pub profile_id: String,
    pub model_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    pub session_id: String,
    pub cwd: String,
    pub profile_id: String,
    pub model_id: String,
    pub updated_at: u64,
}

impl State {
    pub fn latest_session(&self, cwd: &str, profile_id: &str) -> Option<&SessionRecord> {
        self.sessions
            .values()
            .filter(|session| session.cwd == cwd && session.profile_id == profile_id)
            .max_by_key(|session| session.updated_at)
    }
}

pub fn load(path: &Path) -> Result<State> {
    if !path.exists() {
        return Ok(State::default());
    }
    serde_json::from_slice(&fs::read(path)?).context("failed to parse CCSW state")
}

pub fn update(path: &Path, edit: impl FnOnce(&mut State)) -> Result<State> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let lock_path = path.with_extension("json.lock");
    let lock = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(&lock_path)?;
    lock.lock_exclusive()?;
    let mut state = load(path).unwrap_or_default();
    edit(&mut state);
    let encoded = serde_json::to_vec_pretty(&state)?;
    let mut temp = NamedTempFile::new_in(path.parent().context("state path has no parent")?)?;
    temp.write_all(&encoded)?;
    temp.as_file().sync_all()?;
    set_private(temp.path())?;
    temp.persist(path).map_err(|error| error.error)?;
    set_private(path)?;
    FileExt::unlock(&lock).ok();
    Ok(state)
}

pub fn canonical_project(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

pub fn now_epoch() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn latest_session_is_scoped_to_profile() {
        let mut state = State::default();
        for (id, profile, updated) in [("a", "route-a", 1), ("b", "route-b", 2)] {
            state.sessions.insert(
                id.into(),
                SessionRecord {
                    session_id: id.into(),
                    cwd: "/project".into(),
                    profile_id: profile.into(),
                    model_id: "model".into(),
                    updated_at: updated,
                },
            );
        }
        assert_eq!(
            state
                .latest_session("/project", "route-a")
                .map(|session| session.session_id.as_str()),
            Some("a")
        );
    }
}
