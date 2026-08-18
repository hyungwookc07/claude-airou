//! Tiny append-only logs (`hook.log`, `mcp.log`, `overlay.log`), truncated when large —
//! same behaviour as the Swift app so shared directories stay tidy.

use std::io::Write;
use std::path::Path;

pub const LOG_MAX_BYTES: u64 = 512 * 1024;

/// Appends one timestamped line; deletes the file first when it grew past `LOG_MAX_BYTES`.
/// Best-effort: never panics, never writes to stdout.
pub fn append(path: &Path, line: &str) {
    let write = || -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            crate::paths::ensure_dir(parent)?;
        }
        if let Ok(metadata) = std::fs::metadata(path) {
            if metadata.len() > LOG_MAX_BYTES {
                let _ = std::fs::remove_file(path);
            }
        }
        let mut file = std::fs::OpenOptions::new().create(true).append(true).open(path)?;
        let stamp = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
        writeln!(file, "{stamp} {line}")
    };
    let _ = write();
}

/// stderr helper mirroring Swift's `StandardError.print` (never stdout: hook stdout is
/// injected into the model context by Claude Code, MCP stdout belongs to the protocol).
pub fn eprint_line(message: &str) {
    eprintln!("{message}");
}
