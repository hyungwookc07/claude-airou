//! File-based state exchange between writers (hook, MCP server) and the overlay (reader).
//! One JSON file per session under `~/.claude-airou/state/`. Mirrors Swift's
//! `SessionStateStore` including the sanitising rules and stale-file pruning, so the Rust
//! and Swift binaries can share a state directory.

use crate::model::{SessionSnapshot, SessionUsageSnapshot};
use std::io;
use std::path::{Path, PathBuf};

/// Session/usage files whose mtime is older than this are deleted on read. Normal sessions
/// remove their file on `SessionEnd`; this only catches sessions that died without one
/// (killed terminal, crash). Two hours keeps a lunch break alive and drops yesterday's
/// ghosts; a live session that stayed silent that long reappears on its next event.
pub const STALE_AFTER_SECS: f64 = 2.0 * 60.0 * 60.0;
pub const USAGE_FILE_SUFFIX: &str = ".usage.json";

pub struct StateStore {
    pub directory: PathBuf,
}

impl Default for StateStore {
    fn default() -> Self {
        StateStore {
            directory: crate::paths::state_dir(),
        }
    }
}

/// Writes to a temporary file in the same directory and renames it into place
/// (same guarantee as Foundation's `.atomic`).
pub fn write_atomic(path: &Path, data: &[u8]) -> io::Result<()> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut temp = directory.join(format!(
        ".{}.tmp-{}",
        path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default(),
        std::process::id()
    ));
    // Extremely unlikely collision (two writers, same pid namespace); pick a fresh name.
    let mut attempt = 0;
    while temp.exists() {
        attempt += 1;
        temp = directory.join(format!(".airou-tmp-{}-{attempt}", std::process::id()));
    }
    std::fs::write(&temp, data)?;
    match std::fs::rename(&temp, path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = std::fs::remove_file(&temp);
            Err(error)
        }
    }
}

impl StateStore {
    #[allow(dead_code)] // constructor for tests and non-default directories
    pub fn new(directory: PathBuf) -> Self {
        StateStore { directory }
    }

    /// Session ids are UUIDs in practice, but never trust an id as a filename.
    pub fn sanitize_session_id(session_id: &str) -> String {
        let cleaned: String = session_id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '-' || *c == '_')
            .take(80)
            .collect();
        if cleaned.is_empty() {
            "unknown-session".to_string()
        } else {
            cleaned
        }
    }

    pub fn file_url(&self, session_id: &str) -> PathBuf {
        self.directory
            .join(format!("{}.json", Self::sanitize_session_id(session_id)))
    }

    pub fn usage_file_url(&self, session_id: &str) -> PathBuf {
        self.directory
            .join(format!("{}{}", Self::sanitize_session_id(session_id), USAGE_FILE_SUFFIX))
    }

    // MARK: writing

    pub fn write(&self, snapshot: &SessionSnapshot) -> io::Result<()> {
        crate::paths::ensure_dir(&self.directory)?;
        let data = serde_json::to_vec(snapshot).map_err(io::Error::other)?;
        write_atomic(&self.file_url(&snapshot.session_id), &data)
    }

    pub fn remove(&self, session_id: &str) {
        let _ = std::fs::remove_file(self.file_url(session_id));
        let _ = std::fs::remove_file(self.usage_file_url(session_id));
    }

    /// The last snapshot written for a session (used by the hook to merge, not just overwrite).
    pub fn read(&self, session_id: &str) -> Option<SessionSnapshot> {
        let data = std::fs::read(self.file_url(session_id)).ok()?;
        serde_json::from_slice(&data).ok()
    }

    // MARK: usage

    pub fn write_usage(&self, usage: &SessionUsageSnapshot) -> io::Result<()> {
        crate::paths::ensure_dir(&self.directory)?;
        let data = serde_json::to_vec(usage).map_err(io::Error::other)?;
        write_atomic(&self.usage_file_url(&usage.session_id), &data)
    }

    pub fn read_usage(&self, session_id: &str) -> Option<SessionUsageSnapshot> {
        let data = std::fs::read(self.usage_file_url(session_id)).ok()?;
        serde_json::from_slice(&data).ok()
    }

    /// Merges `candidate` into what is on disk (status-line readings stay authoritative
    /// for a while; transcript estimates keep the status line's window / limits / cost).
    pub fn merge_usage(&self, candidate: &SessionUsageSnapshot) -> io::Result<()> {
        match self.read_usage(&candidate.session_id) {
            None => self.write_usage(candidate),
            Some(existing) => {
                if let Some(merged) = existing.merged(candidate, crate::model::now_epoch_secs()) {
                    self.write_usage(&merged)
                } else {
                    Ok(())
                }
            }
        }
    }

    // MARK: reading (overlay side)

    /// Loads every readable snapshot, newest first. Files older than `STALE_AFTER_SECS`
    /// (by mtime) are deleted; undecodable-but-fresh files are skipped, never deleted.
    pub fn load_all(&self) -> Vec<SessionSnapshot> {
        let mut snapshots: Vec<SessionSnapshot> = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return snapshots;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') || !name.ends_with(".json") || name.ends_with(USAGE_FILE_SUFFIX) {
                continue;
            }
            if Self::prune_if_stale(&path) {
                continue;
            }
            if let Ok(data) = std::fs::read(&path) {
                if let Ok(snapshot) = serde_json::from_slice::<SessionSnapshot>(&data) {
                    snapshots.push(snapshot);
                }
            }
        }
        snapshots.sort_by(|a, b| {
            b.updated_at_epoch_seconds
                .partial_cmp(&a.updated_at_epoch_seconds)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        snapshots
    }

    pub fn load_all_usage(&self) -> Vec<SessionUsageSnapshot> {
        let mut result = Vec::new();
        let Ok(entries) = std::fs::read_dir(&self.directory) else {
            return result;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            if name.starts_with('.') || !name.ends_with(USAGE_FILE_SUFFIX) {
                continue;
            }
            if Self::prune_if_stale(&path) {
                continue;
            }
            if let Ok(data) = std::fs::read(&path) {
                if let Ok(usage) = serde_json::from_slice::<SessionUsageSnapshot>(&data) {
                    result.push(usage);
                }
            }
        }
        result
    }

    /// Deletes the file and reports true when its mtime is older than the stale window.
    fn prune_if_stale(path: &Path) -> bool {
        let Ok(metadata) = std::fs::metadata(path) else {
            return false;
        };
        let Ok(modified) = metadata.modified() else {
            return false;
        };
        let age = std::time::SystemTime::now()
            .duration_since(modified)
            .map(|duration| duration.as_secs_f64())
            .unwrap_or(0.0);
        if age > STALE_AFTER_SECS {
            let _ = std::fs::remove_file(path);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{now_epoch_secs, PetState};

    fn snapshot(id: &str, state: PetState) -> SessionSnapshot {
        SessionSnapshot {
            session_id: id.to_string(),
            cwd: "/tmp/project".to_string(),
            state,
            message: "hi".to_string(),
            last_event_name: "test".to_string(),
            tool_name: None,
            updated_at_epoch_seconds: now_epoch_secs(),
            pending_tool_use_id: None,
            active_agent_ids: Vec::new(),
        }
    }

    #[test]
    fn write_read_roundtrip_and_sanitize() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().to_path_buf());
        let snap = snapshot("../../evil id!", PetState::Working);
        store.write(&snap).unwrap();
        // Sanitized filename, no traversal.
        assert!(dir.path().join("evilid.json").exists());
        let all = store.load_all();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].state, PetState::Working);
        store.remove("../../evil id!");
        assert!(store.load_all().is_empty());
    }

    #[test]
    fn load_all_sorts_newest_first_and_skips_usage_files() {
        let dir = tempfile::tempdir().unwrap();
        let store = StateStore::new(dir.path().to_path_buf());
        let mut old = snapshot("old", PetState::Idle);
        old.updated_at_epoch_seconds -= 100.0;
        store.write(&old).unwrap();
        store.write(&snapshot("new", PetState::Done)).unwrap();
        std::fs::write(dir.path().join("x.usage.json"), b"{}").unwrap();
        let all = store.load_all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].session_id, "new");
    }
}
