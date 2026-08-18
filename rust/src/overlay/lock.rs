//! Single-instance lock for the overlay, at `paths::overlay_lock_file()`.
//!
//! The Swift app uses `flock` (the lock vanishes with the process). Without `libc`
//! there is no portable `flock`/`kill(pid, 0)` in std, so the Rust overlay uses a
//! pragmatic pid file instead: the lock file holds the owner's pid; a lock is treated
//! as **live** when it parses to a different pid AND its mtime is younger than one day.
//! Anything else (our own pid after a crash-restart, unparsable content, or an old
//! mtime) is considered stale and replaced. Limitation, documented in the report: a
//! crashed overlay can hold the lock for up to a day unless the file is deleted; a
//! second instance started more than a day later than the first one's last write would
//! wrongly steal the lock (the running overlay refreshes its mtime every tick to
//! prevent that).

use std::path::Path;
use std::time::SystemTime;

/// A lock younger than this (by mtime) with a foreign pid counts as "another overlay
/// is running".
pub const LOCK_STALE_AFTER_SECS: f64 = 24.0 * 60.0 * 60.0;

#[derive(Debug, PartialEq, Eq)]
pub enum LockOutcome {
    Acquired,
    AlreadyRunning,
}

/// Tries to take the overlay lock. Never panics; I/O trouble counts as "acquired"
/// (like the Swift version, which does not block startup over lock problems).
pub fn acquire(path: &Path, our_pid: u32, now: SystemTime) -> LockOutcome {
    if is_held_by_other(path, our_pid, now) {
        return LockOutcome::AlreadyRunning;
    }
    write_pid(path, our_pid);
    LockOutcome::Acquired
}

fn is_held_by_other(path: &Path, our_pid: u32, now: SystemTime) -> bool {
    let Ok(contents) = std::fs::read_to_string(path) else {
        return false; // no lock (or unreadable): treat as free
    };
    let Ok(pid) = contents.trim().parse::<u32>() else {
        return false; // garbage content: stale
    };
    if pid == our_pid {
        return false; // our own leftover (pid reuse of ourselves): stale
    }
    let Ok(metadata) = std::fs::metadata(path) else {
        return false;
    };
    let Ok(modified) = metadata.modified() else {
        return false;
    };
    let age_secs = now
        .duration_since(modified)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0); // mtime in the future: treat as fresh (age 0)
    age_secs < LOCK_STALE_AFTER_SECS
}

/// (Re)writes our pid into the lock. Also used every tick to refresh the mtime so the
/// stale-window heuristic keeps other instances out while we are alive.
pub fn write_pid(path: &Path, pid: u32) {
    if let Some(parent) = path.parent() {
        let _ = crate::paths::ensure_dir(parent);
    }
    let _ = crate::state_store::write_atomic(path, format!("{pid}\n").as_bytes());
}

/// Refreshes the lock's mtime without rewriting when possible (cheap tick-side touch).
pub fn touch(path: &Path, pid: u32) {
    // write_atomic is already cheap for a <16-byte file and refreshes the mtime.
    write_pid(path, pid);
}

/// Removes the lock if it still belongs to `our_pid` (best effort, quit path).
pub fn release(path: &Path, our_pid: u32) {
    if let Ok(contents) = std::fs::read_to_string(path) {
        if contents.trim().parse::<u32>() == Ok(our_pid) {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn acquires_when_no_lock_exists() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlay.lock");
        assert_eq!(acquire(&path, 42, SystemTime::now()), LockOutcome::Acquired);
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "42");
    }

    #[test]
    fn refuses_when_fresh_lock_held_by_other_pid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlay.lock");
        write_pid(&path, 1000);
        assert_eq!(acquire(&path, 42, SystemTime::now()), LockOutcome::AlreadyRunning);
        // The original owner's pid must still be in the file.
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "1000");
    }

    #[test]
    fn steals_lock_older_than_a_day() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlay.lock");
        write_pid(&path, 1000);
        let future = SystemTime::now() + Duration::from_secs(25 * 60 * 60);
        assert_eq!(acquire(&path, 42, future), LockOutcome::Acquired);
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "42");
    }

    #[test]
    fn steals_lock_with_garbage_or_own_pid() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlay.lock");
        std::fs::write(&path, "not-a-pid").unwrap();
        assert_eq!(acquire(&path, 42, SystemTime::now()), LockOutcome::Acquired);

        write_pid(&path, 42); // our own pid, e.g. after a crash + pid reuse
        assert_eq!(acquire(&path, 42, SystemTime::now()), LockOutcome::Acquired);
    }

    #[test]
    fn release_only_removes_own_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("overlay.lock");
        write_pid(&path, 1000);
        release(&path, 42);
        assert!(path.exists(), "someone else's lock must survive");
        release(&path, 1000);
        assert!(!path.exists());
    }
}
