//! Installers: merge/remove our entries in `~/.claude/settings.json` (hooks + statusLine)
//! and the Claude desktop app's `claude_desktop_config.json` (MCP server). Ports of
//! `Install/HooksInstaller.swift`, `Install/StatusLineInstaller.swift` and
//! `Install/MCPInstaller.swift` — same manners everywhere:
//!
//! - Only our own keys are touched; foreign entries and unknown shapes are left alone
//!   (a non-object `hooks`/`mcpServers` refuses with a readable error instead of clobbering).
//! - A timestamped backup (`<file>.claude-airou-backup-YYYYmmdd-HHMMSS[-N]`) is written
//!   before any change; no change → no backup, file left byte-for-byte alone.
//! - Idempotent: re-running updates the command path instead of duplicating entries.
//! - Hook command is the shell-single-quoted absolute path of the current executable +
//!   " hook" with timeout 10; entries are recognised by the "claude-airou"/"claude-pet"
//!   markers or by resolving the first word to our own binary (see `isOurHandler`).
//! - Status line install stashes the previous `statusLine` value in
//!   `paths::statusline_passthrough_file()`; uninstall restores it.
//! - MCP entry: `mcpServers["claude-airou"] = {"command": <exe>, "args": ["mcp"]}` (exec
//!   form, no shell). Uninstall removes the key and drops `mcpServers` when empty.
//! - `--print` prints the would-be JSON snippet and changes nothing.
//!
//! Each `run_*` prints the same summary lines as the Swift CLI and returns the exit code.

use crate::cli::Parsed;
use crate::hook_mapper::SUBSCRIBED_EVENT_NAMES;
use crate::state_store::write_atomic;
use serde_json::{Map, Value};
use std::fmt;
use std::path::{Component, Path, PathBuf};

// MARK: - Shared plumbing

/// Marker used to recognise our own entries regardless of where the binary lives.
const COMMAND_MARKER: &str = "claude-airou";
/// Entries written before the rename; recognised so install updates them and uninstall removes them.
const LEGACY_COMMAND_MARKERS: [&str; 1] = ["claude-pet"];
const HOOK_SUBCOMMAND: &str = "hook";
/// Claude Code gained the hook `args` field (exec form) in 2.1.139 (2026-05-11). Older
/// versions ignore `args` and run `command` through a shell — which for us would mean the
/// bare binary with no subcommand, i.e. the overlay instead of the hook, failing silently.
/// So we write exec form only when the installed CLI is new enough.
const EXEC_FORM_MINIMUM_CLAUDE_VERSION: (u32, u32, u32) = (2, 1, 139);
const HOOK_TIMEOUT_SECONDS: i64 = 10;
const STATUSLINE_SUBCOMMAND: &str = "statusline";
const MCP_SERVER_KEY: &str = "claude-airou";
const MCP_SERVER_SUBCOMMAND: &str = "mcp";
/// Keys of the statusLine object that describe *how* to run us and are therefore replaced;
/// everything else (padding, refreshInterval, hideVimModeIndicator, …) is carried over.
const STATUSLINE_REPLACED_KEYS: [&str; 3] = ["type", "command", "args"];

#[derive(Debug)]
struct InstallError {
    message: String,
}

impl InstallError {
    fn new(message: String) -> InstallError {
        InstallError { message }
    }
}

impl fmt::Display for InstallError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

/// `major.minor.patch` out of `claude --version` ("2.1.223 (Claude Code)"), or None when
/// the CLI is missing (desktop-app-only users) or prints something unexpected.
fn installed_claude_code_version() -> Option<(u32, u32, u32)> {
    let output = std::process::Command::new("claude").arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    parse_semantic_version(&String::from_utf8_lossy(&output.stdout))
}

fn parse_semantic_version(text: &str) -> Option<(u32, u32, u32)> {
    let token = text.split_whitespace().find(|word| {
        let mut parts = word.split('.');
        (0..3).all(|_| parts.next().is_some_and(|part| !part.is_empty() && part.chars().all(|c| c.is_ascii_digit())))
            && parts.next().is_none()
    })?;
    let mut parts = token.split('.').map(|part| part.parse::<u32>().ok());
    Some((parts.next()??, parts.next()??, parts.next()??))
}

/// Exec form needs no shell, so quoting stops mattering — the reason Windows needs it:
/// there hooks run through PowerShell when Git Bash is absent, and PowerShell does not
/// execute a single-quoted path without the call operator.
fn should_write_exec_form_hooks() -> bool {
    installed_claude_code_version().is_some_and(|version| version >= EXEC_FORM_MINIMUM_CLAUDE_VERSION)
}

fn contains_our_marker(text: &str) -> bool {
    text.contains(COMMAND_MARKER) || LEGACY_COMMAND_MARKERS.iter().any(|m| text.contains(m))
}

/// Single-quoted for `sh -c`, so spaces, `$`, backticks and double quotes are all literal.
fn shell_single_quoted(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

/// Absolute path of this binary as invoked. Symlinks are kept on purpose where the OS
/// reports them: if the user installed `~/.local/bin/claude-airou -> <build dir>`, the
/// symlink is the stable address (resolution only happens for *comparisons*).
fn current_executable_path() -> String {
    std::env::current_exe()
        .ok()
        .map(|path| path.display().to_string())
        .or_else(|| std::env::args().next())
        .unwrap_or_else(|| COMMAND_MARKER.to_string())
}

/// Strips `'` and `"` from both ends (Swift: `trimmingCharacters(in: "'\"")`).
fn trim_quotes(word: &str) -> &str {
    word.trim_matches(|c| c == '\'' || c == '"')
}

/// Lexical `.`/`..` cleanup (Swift's `standardizedFileURL` for paths that may not exist).
fn lexically_normalized(path: &Path) -> PathBuf {
    let mut result = PathBuf::new();
    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                let popped = result.pop();
                if !popped && !path.is_absolute() {
                    result.push("..");
                }
            }
            other => result.push(other.as_os_str()),
        }
    }
    result
}

/// Resolves symlinks for equality checks (Swift: `resolvingSymlinksInPath().standardizedFileURL`).
/// Falls back to a lexical cleanup when the path does not exist.
fn resolve_for_comparison(path: &str) -> PathBuf {
    let path = PathBuf::from(path);
    std::fs::canonicalize(&path).unwrap_or_else(|_| lexically_normalized(&path))
}

/// Missing file / empty file → `{}`. Valid JSON that is not an object (and unparseable
/// content) is refused rather than clobbered.
fn load_json_object(path: &Path) -> Result<Map<String, Value>, InstallError> {
    if !path.exists() {
        return Ok(Map::new());
    }
    let data = std::fs::read(path)
        .map_err(|error| InstallError::new(format!("could not read {}: {error}", path.display())))?;
    if data.is_empty() {
        return Ok(Map::new());
    }
    let not_an_object =
        || InstallError::new(format!("{} is not a JSON object; refusing to modify it.", path.display()));
    let value: Value = serde_json::from_slice(&data).map_err(|_| not_an_object())?;
    match value {
        Value::Object(map) => Ok(map),
        _ => Err(not_an_object()),
    }
}

/// Pretty JSON (serde_json sorts object keys, matching Swift's `.sortedKeys`).
fn pretty_json(object: &Map<String, Value>) -> Vec<u8> {
    serde_json::to_vec_pretty(object).unwrap_or_else(|_| b"{}".to_vec())
}

/// Pretty-printed with a trailing newline, written atomically.
fn write_json_object(path: &Path, object: &Map<String, Value>) -> Result<(), InstallError> {
    if let Some(parent) = path.parent() {
        crate::paths::ensure_dir(parent)
            .map_err(|error| InstallError::new(format!("could not create {}: {error}", parent.display())))?;
    }
    let mut data = pretty_json(object);
    data.push(b'\n');
    write_atomic(path, &data)
        .map_err(|error| InstallError::new(format!("could not write {}: {error}", path.display())))
}

/// Copies `path` to `<filename>.claude-airou-backup-YYYYmmdd-HHMMSS` (with `-N` on
/// collision). Only called when a change is about to be written; no file → no backup.
fn backup_if_needed(path: &Path) -> Result<Option<String>, InstallError> {
    if !path.exists() {
        return Ok(None);
    }
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S").to_string();
    backup_with_stamp(path, &stamp).map(Some)
}

fn backup_with_stamp(path: &Path, stamp: &str) -> Result<String, InstallError> {
    let file_name = path
        .file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default();
    let base_name = format!("{file_name}.claude-airou-backup-{stamp}");
    let directory = path.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
    let mut backup_path = directory.join(&base_name);
    let mut suffix = 1;
    while backup_path.exists() {
        backup_path = directory.join(format!("{base_name}-{suffix}"));
        suffix += 1;
    }
    std::fs::copy(path, &backup_path)
        .map_err(|error| InstallError::new(format!("could not back up {}: {error}", path.display())))?;
    Ok(backup_path.display().to_string())
}

/// Swift's `as? [[String: Any]]`: the value must be an array and *every* element an object.
fn array_of_objects(value: &Value) -> Option<Vec<Map<String, Value>>> {
    let array = value.as_array()?;
    let mut result = Vec::with_capacity(array.len());
    for element in array {
        result.push(element.as_object()?.clone());
    }
    Some(result)
}

fn objects_to_value(objects: Vec<Map<String, Value>>) -> Value {
    Value::Array(objects.into_iter().map(Value::Object).collect())
}

fn settings_path_from(parsed: &Parsed) -> PathBuf {
    match parsed.option("settings") {
        Some(path) => crate::paths::expand_tilde(path),
        None => crate::paths::claude_settings_file(),
    }
}

fn mcp_config_path_from(parsed: &Parsed) -> PathBuf {
    match parsed.option("config") {
        Some(path) => crate::paths::expand_tilde(path),
        None => crate::paths::claude_desktop_config_file(),
    }
}

// MARK: - Hooks installer (port of HooksInstaller.swift)

#[derive(Debug, Default)]
struct HooksReport {
    settings_path: String,
    backup_path: Option<String>,
    added_events: Vec<String>,
    updated_events: Vec<String>,
    removed_events: Vec<String>,
    unchanged_events: Vec<String>,
    hook_command: String,
}

impl HooksReport {
    fn summary_text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("Settings: {}", self.settings_path));
        if let Some(backup_path) = &self.backup_path {
            lines.push(format!("Backup:   {backup_path}"));
        }
        lines.push(format!("Command:  {}", self.hook_command));
        if !self.added_events.is_empty() {
            lines.push(format!("Added:    {}", self.added_events.join(", ")));
        }
        if !self.updated_events.is_empty() {
            lines.push(format!("Updated:  {}", self.updated_events.join(", ")));
        }
        if !self.removed_events.is_empty() {
            lines.push(format!("Removed:  {}", self.removed_events.join(", ")));
        }
        if !self.unchanged_events.is_empty() {
            lines.push(format!("Unchanged: {} event(s)", self.unchanged_events.len()));
        }
        lines.join("\n")
    }
}

struct HooksInstaller {
    settings_path: PathBuf,
    executable_path: String,
    /// Write `command` + `args` (no shell) instead of a single-quoted shell string.
    uses_exec_form: bool,
}

impl HooksInstaller {
    fn new(settings_path: PathBuf) -> HooksInstaller {
        HooksInstaller {
            settings_path,
            executable_path: current_executable_path(),
            uses_exec_form: should_write_exec_form_hooks(),
        }
    }

    /// Overrides the auto-detected form (`--hook-format exec|shell`), for users whose
    /// `claude` is not on PATH and for reproducing either shape in a bug report.
    fn with_form(mut self, uses_exec_form: bool) -> HooksInstaller {
        self.uses_exec_form = uses_exec_form;
        self
    }

    #[cfg(test)]
    fn with_executable(settings_path: PathBuf, executable_path: &str) -> HooksInstaller {
        HooksInstaller {
            settings_path,
            executable_path: executable_path.to_string(),
            uses_exec_form: false,
        }
    }

    fn settings_path_string(&self) -> String {
        self.settings_path.display().to_string()
    }

    /// The shell command Claude Code will run (`sh -c`).
    fn hook_command(&self) -> String {
        format!("{} {HOOK_SUBCOMMAND}", shell_single_quoted(&self.executable_path))
    }

    /// What the report prints for the entry that was actually written.
    fn hook_command_display(&self) -> String {
        if self.uses_exec_form {
            format!("{} {HOOK_SUBCOMMAND}   (exec form: no shell)", self.executable_path)
        } else {
            self.hook_command()
        }
    }

    fn our_handler(&self) -> Map<String, Value> {
        let mut handler = Map::new();
        handler.insert("type".to_string(), Value::String("command".to_string()));
        if self.uses_exec_form {
            // No shell: the path is one argument however many spaces or quotes it holds.
            handler.insert("command".to_string(), Value::String(self.executable_path.clone()));
            handler.insert(
                "args".to_string(),
                Value::Array(vec![Value::String(HOOK_SUBCOMMAND.to_string())]),
            );
        } else {
            handler.insert("command".to_string(), Value::String(self.hook_command()));
        }
        handler.insert("timeout".to_string(), Value::from(HOOK_TIMEOUT_SECONDS));
        handler
    }

    fn install(&self) -> Result<HooksReport, InstallError> {
        let mut settings = load_json_object(&self.settings_path)?;
        let mut hooks = self.hooks_object(&settings)?;
        let mut report = HooksReport {
            settings_path: self.settings_path_string(),
            hook_command: self.hook_command_display(),
            ..HooksReport::default()
        };

        for event_name in SUBSCRIBED_EVENT_NAMES {
            let mut groups = self.hook_groups(&hooks, event_name)?;
            let mut found_ours = false;
            let mut changed = false;

            for (group_index, group) in groups.iter_mut().enumerate() {
                let mut handlers = self.handlers(group, event_name, group_index)?;
                for handler in handlers.iter_mut() {
                    if is_our_hook_handler(handler, &self.executable_path) {
                        found_ours = true;
                        let desired = self.our_handler();
                        if *handler != desired {
                            *handler = desired;
                            changed = true;
                        }
                    }
                }
                group.insert("hooks".to_string(), objects_to_value(handlers));
            }

            if !found_ours {
                let mut group = Map::new();
                group.insert("hooks".to_string(), objects_to_value(vec![self.our_handler()]));
                groups.push(group);
                report.added_events.push(event_name.to_string());
            } else if changed {
                report.updated_events.push(event_name.to_string());
            } else {
                report.unchanged_events.push(event_name.to_string());
            }
            hooks.insert(event_name.to_string(), objects_to_value(groups));
        }

        // Nothing to do: leave the file byte-for-byte alone (no backup, no reformatting).
        if report.added_events.is_empty() && report.updated_events.is_empty() {
            return Ok(report);
        }

        settings.insert("hooks".to_string(), Value::Object(hooks));
        report.backup_path = backup_if_needed(&self.settings_path)?;
        write_json_object(&self.settings_path, &settings)?;
        Ok(report)
    }

    fn uninstall(&self) -> Result<HooksReport, InstallError> {
        let mut settings = load_json_object(&self.settings_path)?;
        let mut report = HooksReport {
            settings_path: self.settings_path_string(),
            hook_command: self.hook_command_display(),
            ..HooksReport::default()
        };
        if !settings.contains_key("hooks") {
            return Ok(report);
        }
        let mut hooks = self.hooks_object(&settings)?;

        // serde_json::Map iterates keys in sorted order, matching Swift's `keys.sorted()`.
        let event_names: Vec<String> = hooks.keys().cloned().collect();
        for event_name in event_names {
            let Some(groups_value) = hooks.get(&event_name) else { continue };
            let Some(groups) = array_of_objects(groups_value) else { continue }; // foreign shape: leave alone
            let mut removed_any = false;
            let mut kept_groups: Vec<Map<String, Value>> = Vec::new();
            for mut group in groups {
                let handlers = group.get("hooks").and_then(array_of_objects);
                let Some(handlers) = handlers else {
                    kept_groups.push(group); // foreign shape: leave alone
                    continue;
                };
                let kept: Vec<Map<String, Value>> = handlers
                    .iter()
                    .filter(|handler| !is_our_hook_handler(handler, &self.executable_path))
                    .cloned()
                    .collect();
                if kept.len() != handlers.len() {
                    removed_any = true;
                }
                if kept.is_empty() && !handlers.is_empty() {
                    continue; // group only held our handler
                }
                group.insert("hooks".to_string(), objects_to_value(kept));
                kept_groups.push(group);
            }
            if removed_any {
                report.removed_events.push(event_name.clone());
            }
            if kept_groups.is_empty() {
                hooks.remove(&event_name);
            } else {
                hooks.insert(event_name, objects_to_value(kept_groups));
            }
        }

        if report.removed_events.is_empty() {
            return Ok(report);
        }

        if hooks.is_empty() {
            settings.remove("hooks");
        } else {
            settings.insert("hooks".to_string(), Value::Object(hooks));
        }
        report.backup_path = backup_if_needed(&self.settings_path)?;
        write_json_object(&self.settings_path, &settings)?;
        Ok(report)
    }

    /// `settings.hooks` must be an object if present. Anything else is refused rather than clobbered.
    fn hooks_object(&self, settings: &Map<String, Value>) -> Result<Map<String, Value>, InstallError> {
        let Some(raw) = settings.get("hooks") else {
            return Ok(Map::new());
        };
        match raw.as_object() {
            Some(object) => Ok(object.clone()),
            None => Err(InstallError::new(format!(
                "\"hooks\" in {} is not a JSON object; fix or remove it and re-run.",
                self.settings_path_string()
            ))),
        }
    }

    fn hook_groups(
        &self,
        hooks: &Map<String, Value>,
        event_name: &str,
    ) -> Result<Vec<Map<String, Value>>, InstallError> {
        let Some(raw) = hooks.get(event_name) else {
            return Ok(Vec::new());
        };
        array_of_objects(raw).ok_or_else(|| {
            InstallError::new(format!(
                "hooks.{event_name} in {} is not an array of hook groups; fix or remove it and re-run.",
                self.settings_path_string()
            ))
        })
    }

    fn handlers(
        &self,
        group: &Map<String, Value>,
        event_name: &str,
        group_index: usize,
    ) -> Result<Vec<Map<String, Value>>, InstallError> {
        let Some(raw) = group.get("hooks") else {
            return Ok(Vec::new());
        };
        array_of_objects(raw).ok_or_else(|| {
            InstallError::new(format!(
                "hooks.{event_name}[{group_index}].hooks in {} is not an array; fix or remove it and re-run.",
                self.settings_path_string()
            ))
        })
    }

    /// The JSON a user would paste by hand (printed by `claude-airou install-hooks --print`).
    fn snippet_json(&self) -> String {
        let mut hooks = Map::new();
        for event_name in SUBSCRIBED_EVENT_NAMES {
            let mut group = Map::new();
            group.insert("hooks".to_string(), objects_to_value(vec![self.our_handler()]));
            hooks.insert(event_name.to_string(), objects_to_value(vec![group]));
        }
        let mut root = Map::new();
        root.insert("hooks".to_string(), Value::Object(hooks));
        String::from_utf8(pretty_json(&root)).unwrap_or_default()
    }
}

/// Recognises our own entries regardless of where the binary lives:
/// shell form `'.../claude-airou' hook` (what we write) or exec form
/// `command: ".../claude-airou", args: ["hook"]`.
fn is_our_hook_handler(handler: &Map<String, Value>, executable_path: &str) -> bool {
    if handler.get("type").and_then(Value::as_str) != Some("command") {
        return false;
    }
    let Some(command) = handler.get("command").and_then(Value::as_str) else {
        return false;
    };
    let trimmed = command.trim();
    if !contains_our_marker(trimmed) {
        // Renamed/symlinked binary: same file, different name.
        let words: Vec<&str> = trimmed.split(' ').filter(|word| !word.is_empty()).collect();
        if words.len() < 2 || words[words.len() - 1] != HOOK_SUBCOMMAND {
            return false;
        }
        let first_word = trim_quotes(words[0]);
        return resolve_for_comparison(first_word) == resolve_for_comparison(executable_path);
    }
    if trimmed.ends_with(&format!(" {HOOK_SUBCOMMAND}")) {
        return true;
    }
    if let Some(args) = handler.get("args").and_then(Value::as_array) {
        if args.len() == 1 && args[0].as_str() == Some(HOOK_SUBCOMMAND) {
            let mut markers = vec![COMMAND_MARKER];
            markers.extend(LEGACY_COMMAND_MARKERS);
            return markers.iter().any(|marker| {
                trimmed.ends_with(marker)
                    || trimmed.ends_with(&format!("{marker}'"))
                    || trimmed.ends_with(&format!("{marker}\""))
            });
        }
    }
    false
}

// MARK: - Status line installer (port of StatusLineInstaller.swift)

#[derive(Debug, Default)]
struct StatusLineReport {
    settings_path: String,
    backup_path: Option<String>,
    action: String,
    passthrough_command: Option<String>,
}

impl StatusLineReport {
    fn summary_text(&self) -> String {
        let mut lines = vec![format!("Settings: {}", self.settings_path)];
        if let Some(backup_path) = &self.backup_path {
            lines.push(format!("Backup:   {backup_path}"));
        }
        lines.push(format!("Action:   {}", self.action));
        if let Some(passthrough_command) = &self.passthrough_command {
            lines.push(format!("Then runs: {passthrough_command}"));
        }
        lines.join("\n")
    }
}

struct StatusLineInstaller {
    settings_path: PathBuf,
    executable_path: String,
    /// `paths::claude_settings_file()` in production; injectable for tests.
    default_settings_path: PathBuf,
    /// `paths::root_dir()` in production (where the passthrough files live).
    airou_root: PathBuf,
}

impl StatusLineInstaller {
    fn new(settings_path: PathBuf) -> StatusLineInstaller {
        StatusLineInstaller {
            settings_path,
            executable_path: current_executable_path(),
            default_settings_path: crate::paths::claude_settings_file(),
            airou_root: crate::paths::root_dir(),
        }
    }

    #[cfg(test)]
    fn for_test(
        settings_path: PathBuf,
        executable_path: &str,
        default_settings_path: PathBuf,
        airou_root: PathBuf,
    ) -> StatusLineInstaller {
        StatusLineInstaller {
            settings_path,
            executable_path: executable_path.to_string(),
            default_settings_path,
            airou_root,
        }
    }

    fn settings_path_string(&self) -> String {
        self.settings_path.display().to_string()
    }

    fn is_default_settings(&self) -> bool {
        lexically_normalized(&self.settings_path) == lexically_normalized(&self.default_settings_path)
    }

    /// The passthrough file for this settings file. One file per settings file, so
    /// `--settings` targets don't clobber each other (Swift: `StatusLineCommand.passthroughFile`).
    fn passthrough_file(&self) -> PathBuf {
        if self.is_default_settings() {
            // == paths::statusline_passthrough_file() when airou_root == paths::root_dir().
            return self.airou_root.join("statusline-passthrough.json");
        }
        let digest = settings_path_digest(&self.settings_path_string());
        self.airou_root.join(format!("statusline-passthrough-{digest}.json"))
    }

    fn our_command(&self) -> String {
        let mut command = format!(
            "{} {STATUSLINE_SUBCOMMAND}",
            shell_single_quoted(&self.executable_path)
        );
        if !self.is_default_settings() {
            command.push_str(&format!(
                " --settings {}",
                shell_single_quoted(&self.settings_path_string())
            ));
        }
        command
    }

    /// Our statusLine object, keeping any extra keys the user had (padding etc.).
    fn our_status_line_object(&self, original: Option<&Map<String, Value>>) -> Map<String, Value> {
        let mut object: Map<String, Value> = match original {
            Some(original) => original
                .iter()
                .filter(|(key, _)| !STATUSLINE_REPLACED_KEYS.contains(&key.as_str()))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect(),
            None => Map::new(),
        };
        object.insert("type".to_string(), Value::String("command".to_string()));
        object.insert("command".to_string(), Value::String(self.our_command()));
        object
    }

    fn install(&self) -> Result<StatusLineReport, InstallError> {
        let mut settings = load_json_object(&self.settings_path)?;
        let mut report = StatusLineReport {
            settings_path: self.settings_path_string(),
            ..StatusLineReport::default()
        };

        if let Some(existing) = settings.get("statusLine") {
            let Some(existing_object) = existing.as_object().cloned() else {
                return Err(InstallError::new(format!(
                    "\"statusLine\" in {} is not a JSON object; fix or remove it and re-run.",
                    self.settings_path_string()
                )));
            };
            if is_our_status_line(&existing_object, &self.executable_path) {
                let desired = self.our_status_line_object(Some(&existing_object));
                if existing_object == desired {
                    report.action = "already installed".to_string();
                    report.passthrough_command = self.stored_passthrough_command();
                    return Ok(report);
                }
                // Same feature, binary moved/renamed: update the command, keep the stored passthrough.
                settings.insert("statusLine".to_string(), Value::Object(desired));
                report.action = "updated command path".to_string();
                report.passthrough_command = self.stored_passthrough_command();
            } else {
                let existing_type = existing_object
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("command");
                if existing_type != "command" {
                    return Err(InstallError::new(format!(
                        "statusLine in {} has type \"{existing_type}\", which claude-airou cannot pass through. Remove it (or switch it to a command) and re-run.",
                        self.settings_path_string()
                    )));
                }
                if let Some(existing_command) = existing_object.get("command").and_then(Value::as_str) {
                    if is_self_invocation(existing_command) {
                        return Err(InstallError::new(format!(
                            "statusLine already invokes claude-airou ({existing_command}); refusing to store it as its own passthrough."
                        )));
                    }
                }
                self.store_passthrough(Some(&existing_object))?;
                settings.insert(
                    "statusLine".to_string(),
                    Value::Object(self.our_status_line_object(Some(&existing_object))),
                );
                report.action = "installed (original status line kept as passthrough)".to_string();
                report.passthrough_command = existing_object
                    .get("command")
                    .and_then(Value::as_str)
                    .map(str::to_string);
            }
        } else {
            self.store_passthrough(None)?;
            settings.insert(
                "statusLine".to_string(),
                Value::Object(self.our_status_line_object(None)),
            );
            report.action = "installed (there was no status line before)".to_string();
        }

        report.backup_path = backup_if_needed(&self.settings_path)?;
        write_json_object(&self.settings_path, &settings)?;
        Ok(report)
    }

    fn uninstall(&self) -> Result<StatusLineReport, InstallError> {
        let mut settings = load_json_object(&self.settings_path)?;
        let mut report = StatusLineReport {
            settings_path: self.settings_path_string(),
            ..StatusLineReport::default()
        };
        let existing = settings
            .get("statusLine")
            .and_then(Value::as_object)
            .filter(|object| is_our_status_line(object, &self.executable_path))
            .cloned();
        let Some(existing) = existing else {
            report.action = "nothing to do (status line is not claude-airou's)".to_string();
            return Ok(report);
        };
        if self.passthrough_file_is_corrupt() {
            return Err(InstallError::new(format!(
                "{} is unreadable; not touching settings.statusLine so your original status line is not lost. Fix or delete that file (your original statusLine is also in the settings.json.claude-airou-backup-* files) and re-run.",
                self.passthrough_file().display()
            )));
        }
        let original = self.stored_passthrough_object().unwrap_or_default();
        // Extra keys the user may have changed while ours was installed win over the stored copy.
        let extras: Vec<(String, Value)> = existing
            .iter()
            .filter(|(key, _)| !STATUSLINE_REPLACED_KEYS.contains(&key.as_str()))
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect();
        let original_command = original
            .get("command")
            .and_then(Value::as_str)
            .filter(|command| !command.is_empty())
            .map(str::to_string);
        if let Some(original_command) = original_command {
            let mut restored = original;
            for (key, value) in extras {
                if !restored.contains_key(&key) {
                    restored.insert(key, value);
                }
            }
            settings.insert("statusLine".to_string(), Value::Object(restored));
            report.action = "restored the original status line".to_string();
            report.passthrough_command = Some(original_command);
        } else {
            settings.remove("statusLine");
            report.action = "removed (there was no status line before)".to_string();
        }
        report.backup_path = backup_if_needed(&self.settings_path)?;
        write_json_object(&self.settings_path, &settings)?;
        let _ = std::fs::remove_file(self.passthrough_file());
        Ok(report)
    }

    /// The user's original statusLine object, kept while ours is installed
    /// (pretty JSON, no trailing newline — same bytes as the Swift app writes).
    fn store_passthrough(&self, object: Option<&Map<String, Value>>) -> Result<(), InstallError> {
        crate::paths::ensure_dir(&self.airou_root).map_err(|error| {
            InstallError::new(format!("could not create {}: {error}", self.airou_root.display()))
        })?;
        let empty = Map::new();
        let data = pretty_json(object.unwrap_or(&empty));
        let path = self.passthrough_file();
        write_atomic(&path, &data)
            .map_err(|error| InstallError::new(format!("could not write {}: {error}", path.display())))
    }

    fn stored_passthrough_object(&self) -> Option<Map<String, Value>> {
        let data = std::fs::read(self.passthrough_file()).ok()?;
        let value: Value = serde_json::from_slice(&data).ok()?;
        match value {
            Value::Object(object) => Some(object),
            _ => None,
        }
    }

    fn stored_passthrough_command(&self) -> Option<String> {
        let object = self.stored_passthrough_object()?;
        let type_name = object.get("type").and_then(Value::as_str).unwrap_or("command");
        if type_name != "command" {
            return None;
        }
        object.get("command").and_then(Value::as_str).map(str::to_string)
    }

    /// True when a passthrough file exists but cannot be read as a JSON object.
    fn passthrough_file_is_corrupt(&self) -> bool {
        if !self.passthrough_file().exists() {
            return false;
        }
        self.stored_passthrough_object().is_none()
    }
}

/// Recognises our own entry: a `... statusline` command whose text carries our marker (current
/// or legacy name) or whose executable resolves to this very binary (renamed/symlinked installs).
fn is_our_status_line(status_line: &Map<String, Value>, executable_path: &str) -> bool {
    let type_is_command = status_line
        .get("type")
        .and_then(Value::as_str)
        .map_or(true, |type_name| type_name == "command");
    if !type_is_command {
        return false;
    }
    let Some(command) = status_line.get("command").and_then(Value::as_str) else {
        return false;
    };
    let trimmed = command.trim();
    let words: Vec<&str> = trimmed.split(' ').filter(|word| !word.is_empty()).collect();
    if words.len() < 2 || !words[1..].contains(&STATUSLINE_SUBCOMMAND) {
        return false;
    }
    if contains_our_marker(trimmed) {
        return true;
    }
    // Same file, different name: compare resolved executable paths.
    let first_word = trim_quotes(words[0]);
    resolve_for_comparison(first_word) == resolve_for_comparison(executable_path)
}

/// True when `command` is one of our own status line commands (would recurse forever).
/// Port of `StatusLineCommand.isSelfInvocation`.
fn is_self_invocation(command: &str) -> bool {
    let trimmed = command.trim();
    if !contains_our_marker(trimmed) {
        return false;
    }
    trimmed.ends_with(" statusline") || trimmed.contains(" statusline ")
}

/// Swift's djb2-flavoured digest for per-settings passthrough files
/// (`StatusLineCommand.passthroughFile`): u64 wrapping hash, rendered in base 36.
fn settings_path_digest(settings_path: &str) -> String {
    let mut hash: u64 = 5381;
    for byte in settings_path.as_bytes() {
        hash = hash.wrapping_mul(33).wrapping_add(u64::from(*byte));
    }
    to_base36(hash)
}

fn to_base36(mut value: u64) -> String {
    const DIGITS: &[u8; 36] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if value == 0 {
        return "0".to_string();
    }
    let mut out: Vec<u8> = Vec::new();
    while value > 0 {
        out.push(DIGITS[(value % 36) as usize]);
        value /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap_or_default()
}

// MARK: - MCP installer (port of MCPInstaller.swift)

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum McpOutcome {
    Added,
    Updated,
    Unchanged,
    Removed,
    Absent,
}

#[derive(Debug)]
struct McpReport {
    config_path: String,
    backup_path: Option<String>,
    outcome: McpOutcome,
    server_command: String,
}

impl McpReport {
    fn summary_text(&self) -> String {
        let mut lines: Vec<String> = Vec::new();
        lines.push(format!("Config:  {}", self.config_path));
        if let Some(backup_path) = &self.backup_path {
            lines.push(format!("Backup:  {backup_path}"));
        }
        lines.push(format!("Command: {}", self.server_command));
        lines.push(match self.outcome {
            McpOutcome::Added => format!("Added:   mcpServers.{MCP_SERVER_KEY}"),
            McpOutcome::Updated => format!("Updated: mcpServers.{MCP_SERVER_KEY}"),
            McpOutcome::Unchanged => "Already installed; nothing changed.".to_string(),
            McpOutcome::Removed => format!("Removed: mcpServers.{MCP_SERVER_KEY}"),
            McpOutcome::Absent => "Not installed; nothing changed.".to_string(),
        });
        lines.join("\n")
    }
}

struct McpInstaller {
    config_path: PathBuf,
    executable_path: String,
}

impl McpInstaller {
    fn new(config_path: PathBuf) -> McpInstaller {
        McpInstaller {
            config_path,
            executable_path: current_executable_path(),
        }
    }

    #[cfg(test)]
    fn with_executable(config_path: PathBuf, executable_path: &str) -> McpInstaller {
        McpInstaller {
            config_path,
            executable_path: executable_path.to_string(),
        }
    }

    fn config_path_string(&self) -> String {
        self.config_path.display().to_string()
    }

    fn server_command(&self) -> String {
        format!("{} {MCP_SERVER_SUBCOMMAND}", self.executable_path)
    }

    /// The entry the desktop app spawns: `<binary> mcp` (exec form, no shell involved).
    fn server_entry(&self) -> Map<String, Value> {
        let mut entry = Map::new();
        entry.insert("command".to_string(), Value::String(self.executable_path.clone()));
        entry.insert(
            "args".to_string(),
            Value::Array(vec![Value::String(MCP_SERVER_SUBCOMMAND.to_string())]),
        );
        entry
    }

    fn install(&self) -> Result<McpReport, InstallError> {
        let mut config = load_json_object(&self.config_path)?;
        let mut servers = self.servers_object(&config)?;
        let mut report = McpReport {
            config_path: self.config_path_string(),
            backup_path: None,
            outcome: McpOutcome::Unchanged,
            server_command: self.server_command(),
        };
        let desired = self.server_entry();

        match servers.get(MCP_SERVER_KEY).and_then(Value::as_object) {
            Some(existing) => {
                if *existing == desired {
                    report.outcome = McpOutcome::Unchanged;
                    return Ok(report);
                }
                report.outcome = McpOutcome::Updated;
            }
            None => report.outcome = McpOutcome::Added,
        }

        servers.insert(MCP_SERVER_KEY.to_string(), Value::Object(desired));
        config.insert("mcpServers".to_string(), Value::Object(servers));
        report.backup_path = backup_if_needed(&self.config_path)?;
        write_json_object(&self.config_path, &config)?;
        Ok(report)
    }

    fn uninstall(&self) -> Result<McpReport, InstallError> {
        let mut config = load_json_object(&self.config_path)?;
        let mut report = McpReport {
            config_path: self.config_path_string(),
            backup_path: None,
            outcome: McpOutcome::Absent,
            server_command: self.server_command(),
        };
        if !config.contains_key("mcpServers") {
            return Ok(report);
        }
        let mut servers = self.servers_object(&config)?;
        if servers.remove(MCP_SERVER_KEY).is_none() {
            return Ok(report);
        }
        report.outcome = McpOutcome::Removed;

        if servers.is_empty() {
            config.remove("mcpServers");
        } else {
            config.insert("mcpServers".to_string(), Value::Object(servers));
        }
        report.backup_path = backup_if_needed(&self.config_path)?;
        write_json_object(&self.config_path, &config)?;
        Ok(report)
    }

    /// `mcpServers` must be an object if present. Anything else is refused rather than clobbered.
    fn servers_object(&self, config: &Map<String, Value>) -> Result<Map<String, Value>, InstallError> {
        let Some(raw) = config.get("mcpServers") else {
            return Ok(Map::new());
        };
        match raw.as_object() {
            Some(object) => Ok(object.clone()),
            None => Err(InstallError::new(format!(
                "\"mcpServers\" in {} is not a JSON object; fix or remove it and re-run.",
                self.config_path_string()
            ))),
        }
    }

    /// The JSON a user would paste by hand (printed by `claude-airou install-mcp --print`).
    fn snippet_json(&self) -> String {
        let mut servers = Map::new();
        servers.insert(MCP_SERVER_KEY.to_string(), Value::Object(self.server_entry()));
        let mut root = Map::new();
        root.insert("mcpServers".to_string(), Value::Object(servers));
        String::from_utf8(pretty_json(&root)).unwrap_or_default()
    }
}

// MARK: - Default-path entry points (overlay tray menu + `claude-airou setup`)
// The Swift app only needed the two install halves (AppDelegate.installHooks /
// installStatusLine); `setup` and `uninstall` drive the same installers headlessly.

/// Installs the hooks into the default Claude settings; Ok carries the report summary
/// (shown in an alert by the overlay, printed by `setup`), Err the failure text.
pub fn install_hooks_at_default_paths() -> Result<String, String> {
    HooksInstaller::new(crate::paths::claude_settings_file())
        .install()
        .map(|report| report.summary_text())
        .map_err(|error| error.to_string())
}

/// Wires the status line feed into the default Claude settings (see `install_hooks_at_default_paths`).
pub fn install_statusline_at_default_paths() -> Result<String, String> {
    StatusLineInstaller::new(crate::paths::claude_settings_file())
        .install()
        .map(|report| report.summary_text())
        .map_err(|error| error.to_string())
}

/// Registers the MCP server in the default Claude desktop app config.
pub fn install_mcp_at_default_paths() -> Result<String, String> {
    McpInstaller::new(crate::paths::claude_desktop_config_file())
        .install()
        .map(|report| report.summary_text())
        .map_err(|error| error.to_string())
}

pub fn uninstall_hooks_at_default_paths() -> Result<String, String> {
    HooksInstaller::new(crate::paths::claude_settings_file())
        .uninstall()
        .map(|report| report.summary_text())
        .map_err(|error| error.to_string())
}

pub fn uninstall_statusline_at_default_paths() -> Result<String, String> {
    StatusLineInstaller::new(crate::paths::claude_settings_file())
        .uninstall()
        .map(|report| report.summary_text())
        .map_err(|error| error.to_string())
}

pub fn uninstall_mcp_at_default_paths() -> Result<String, String> {
    McpInstaller::new(crate::paths::claude_desktop_config_file())
        .uninstall()
        .map(|report| report.summary_text())
        .map_err(|error| error.to_string())
}

// MARK: - CLI wrappers (port of the run* helpers in CommandLineInterface.swift)

pub fn run_install_hooks(parsed: &Parsed) -> i32 {
    let installer = match parsed.option("hook-format") {
        Some("exec") => HooksInstaller::new(settings_path_from(parsed)).with_form(true),
        Some("shell") => HooksInstaller::new(settings_path_from(parsed)).with_form(false),
        Some(other) => {
            crate::logging::eprint_line(&format!(
                "claude-airou: --hook-format must be \"exec\" or \"shell\" (got \"{other}\")"
            ));
            return 2;
        }
        None => HooksInstaller::new(settings_path_from(parsed)),
    };
    if parsed.has_flag("print") {
        println!("{}", installer.snippet_json());
        return 0;
    }
    match installer.install() {
        Ok(report) => {
            println!("Claude Code hooks installed.");
            println!("{}", report.summary_text());
            println!("\nRestart running Claude Code sessions (or start a new one) for hooks to take effect.");
            0
        }
        Err(error) => {
            crate::logging::eprint_line(&format!("claude-airou: install failed: {error}"));
            1
        }
    }
}

pub fn run_uninstall_hooks(parsed: &Parsed) -> i32 {
    let installer = HooksInstaller::new(settings_path_from(parsed));
    match installer.uninstall() {
        Ok(report) => {
            println!("Claude Code hooks removed.");
            println!("{}", report.summary_text());
            0
        }
        Err(error) => {
            crate::logging::eprint_line(&format!("claude-airou: uninstall failed: {error}"));
            1
        }
    }
}

pub fn run_install_statusline(parsed: &Parsed) -> i32 {
    let installer = StatusLineInstaller::new(settings_path_from(parsed));
    match installer.install() {
        Ok(report) => {
            println!("Claude Code status line wired to claude-airou.");
            println!("{}", report.summary_text());
            println!("\nNew Claude Code sessions will feed the usage gauge; your own status line keeps rendering through the passthrough.");
            0
        }
        Err(error) => {
            crate::logging::eprint_line(&format!("claude-airou: install-statusline failed: {error}"));
            1
        }
    }
}

pub fn run_uninstall_statusline(parsed: &Parsed) -> i32 {
    let installer = StatusLineInstaller::new(settings_path_from(parsed));
    match installer.uninstall() {
        Ok(report) => {
            println!("{}", report.summary_text());
            0
        }
        Err(error) => {
            crate::logging::eprint_line(&format!("claude-airou: uninstall-statusline failed: {error}"));
            1
        }
    }
}

pub fn run_install_mcp(parsed: &Parsed) -> i32 {
    let installer = McpInstaller::new(mcp_config_path_from(parsed));
    if parsed.has_flag("print") {
        println!("{}", installer.snippet_json());
        return 0;
    }
    match installer.install() {
        Ok(report) => {
            println!("Claude desktop app MCP server registered.");
            println!("{}", report.summary_text());
            println!("\nQuit the Claude desktop app completely (Cmd-Q) and reopen it for the server to load.");
            0
        }
        Err(error) => {
            crate::logging::eprint_line(&format!("claude-airou: install-mcp failed: {error}"));
            1
        }
    }
}

pub fn run_uninstall_mcp(parsed: &Parsed) -> i32 {
    let installer = McpInstaller::new(mcp_config_path_from(parsed));
    match installer.uninstall() {
        Ok(report) => {
            println!("Claude desktop app MCP server removed.");
            println!("{}", report.summary_text());
            0
        }
        Err(error) => {
            crate::logging::eprint_line(&format!("claude-airou: uninstall-mcp failed: {error}"));
            1
        }
    }
}

// MARK: - Tests

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const EXE: &str = "/tmp/fake exe/claude-airou";

    fn exec_form_installer(settings_path: PathBuf) -> HooksInstaller {
        HooksInstaller::with_executable(settings_path, EXE).with_form(true)
    }

    #[test]
    fn parses_the_version_claude_prints() {
        assert_eq!(parse_semantic_version("2.1.223 (Claude Code)"), Some((2, 1, 223)));
        assert_eq!(parse_semantic_version("  2.1.139\n"), Some((2, 1, 139)));
        assert_eq!(parse_semantic_version("Claude Code 10.0.2 (build 7)"), Some((10, 0, 2)));
        // Nothing version-shaped, or not three numeric parts: we cannot tell, so no exec form.
        assert_eq!(parse_semantic_version("unknown"), None);
        assert_eq!(parse_semantic_version("2.1"), None);
        assert_eq!(parse_semantic_version("2.1.x"), None);
        assert_eq!(parse_semantic_version("2.1.223.4"), None);
    }

    #[test]
    fn exec_form_needs_the_version_that_understands_args() {
        // Tuple ordering is the comparison used against EXEC_FORM_MINIMUM_CLAUDE_VERSION.
        assert!((2, 1, 139) >= EXEC_FORM_MINIMUM_CLAUDE_VERSION);
        assert!((2, 1, 223) >= EXEC_FORM_MINIMUM_CLAUDE_VERSION);
        assert!((2, 2, 0) >= EXEC_FORM_MINIMUM_CLAUDE_VERSION);
        assert!((3, 0, 0) >= EXEC_FORM_MINIMUM_CLAUDE_VERSION);
        assert!(!((2, 1, 138) >= EXEC_FORM_MINIMUM_CLAUDE_VERSION));
        assert!(!((2, 0, 999) >= EXEC_FORM_MINIMUM_CLAUDE_VERSION));
        assert!(!((1, 9, 9) >= EXEC_FORM_MINIMUM_CLAUDE_VERSION));
    }

    #[test]
    fn exec_form_writes_command_and_args_without_quoting() {
        let dir = tempdir();
        let settings = dir.path().join("settings.json");
        exec_form_installer(settings.clone()).install().unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        let handler = &written["hooks"]["PreToolUse"][0]["hooks"][0];
        // The path keeps its space and carries no quotes: there is no shell to strip them.
        assert_eq!(handler["command"], json!(EXE));
        assert_eq!(handler["args"], json!(["hook"]));
        assert_eq!(handler["timeout"], json!(10));
    }

    #[test]
    fn switching_form_migrates_the_existing_entry_in_place() {
        let dir = tempdir();
        let settings = dir.path().join("settings.json");

        // A machine set up before exec form existed.
        HooksInstaller::with_executable(settings.clone(), EXE).install().unwrap();
        let shell_form: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        assert_eq!(shell_form["hooks"]["Stop"][0]["hooks"][0]["command"], json!("'/tmp/fake exe/claude-airou' hook"));
        assert!(shell_form["hooks"]["Stop"][0]["hooks"][0].get("args").is_none());

        // Upgrading Claude Code and re-running setup replaces it rather than adding a second.
        let report = exec_form_installer(settings.clone()).install().unwrap();
        assert!(report.added_events.is_empty(), "the entry is updated, not duplicated");
        assert!(!report.updated_events.is_empty());

        let exec_form: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        let handlers = exec_form["hooks"]["Stop"][0]["hooks"].as_array().unwrap();
        assert_eq!(handlers.len(), 1, "no leftover shell-form entry");
        assert_eq!(handlers[0]["command"], json!(EXE));
        assert_eq!(handlers[0]["args"], json!(["hook"]));
    }

    #[test]
    fn uninstall_removes_exec_form_entries_too() {
        let dir = tempdir();
        let settings = dir.path().join("settings.json");
        exec_form_installer(settings.clone()).install().unwrap();
        exec_form_installer(settings.clone()).uninstall().unwrap();

        let written: Value = serde_json::from_slice(&std::fs::read(&settings).unwrap()).unwrap();
        let leftover = written["hooks"].as_object().map(|hooks| {
            hooks.values().filter(|value| !value.as_array().map(|a| a.is_empty()).unwrap_or(true)).count()
        });
        assert_eq!(leftover.unwrap_or(0), 0, "every exec-form entry is gone");
    }

    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("tempdir")
    }

    fn read_json(path: &Path) -> Value {
        let data = std::fs::read(path).expect("read settings");
        serde_json::from_slice(&data).expect("parse settings")
    }

    fn write_settings(path: &Path, value: &Value) {
        std::fs::write(path, serde_json::to_vec_pretty(value).expect("serialize")).expect("write");
    }

    fn dir_file_names(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = std::fs::read_dir(dir)
            .expect("read dir")
            .map(|entry| entry.expect("entry").file_name().to_string_lossy().to_string())
            .collect();
        names.sort();
        names
    }

    fn handler_map(command: &str, timeout: Option<i64>) -> Map<String, Value> {
        let mut map = Map::new();
        map.insert("type".to_string(), json!("command"));
        map.insert("command".to_string(), json!(command));
        if let Some(timeout) = timeout {
            map.insert("timeout".to_string(), json!(timeout));
        }
        map
    }

    // ---- hooks: install ----

    #[test]
    fn fresh_install_adds_all_events() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let report = installer.install().expect("install");

        assert_eq!(report.added_events.len(), SUBSCRIBED_EVENT_NAMES.len());
        assert!(report.updated_events.is_empty());
        assert!(report.unchanged_events.is_empty());
        assert_eq!(report.backup_path, None); // no pre-existing file, no backup
        assert_eq!(report.hook_command, "'/tmp/fake exe/claude-airou' hook");

        let written = std::fs::read(&settings_path).expect("read");
        assert!(written.ends_with(b"\n"), "file must end with a newline");
        let value = read_json(&settings_path);
        let hooks = value["hooks"].as_object().expect("hooks object");
        assert_eq!(hooks.len(), SUBSCRIBED_EVENT_NAMES.len());
        for event in SUBSCRIBED_EVENT_NAMES {
            let groups = hooks[event].as_array().expect("groups");
            assert_eq!(groups.len(), 1);
            let handlers = groups[0]["hooks"].as_array().expect("handlers");
            assert_eq!(handlers.len(), 1);
            assert_eq!(
                handlers[0],
                json!({"type": "command", "command": "'/tmp/fake exe/claude-airou' hook", "timeout": 10})
            );
        }
    }

    #[test]
    fn reinstall_is_byte_identical_noop_without_backup() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        installer.install().expect("first install");
        let before = std::fs::read(&settings_path).expect("read");

        let report = installer.install().expect("second install");
        assert!(report.added_events.is_empty());
        assert!(report.updated_events.is_empty());
        assert_eq!(report.unchanged_events.len(), SUBSCRIBED_EVENT_NAMES.len());
        assert_eq!(report.backup_path, None);

        let after = std::fs::read(&settings_path).expect("read");
        assert_eq!(before, after, "re-run must be a byte-identical no-op");
        assert_eq!(dir_file_names(dir.path()), vec!["settings.json"], "no backup file");
    }

    #[test]
    fn install_preserves_foreign_entries_and_updates_ours() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        let foreign = json!({"type": "command", "command": "other-tool hook-me"});
        write_settings(
            &settings_path,
            &json!({
                "model": "opus",
                "hooks": {
                    "PreToolUse": [
                        {"matcher": "Bash", "hooks": [foreign.clone()]},
                        {"hooks": [{"type": "command", "command": "'/old/claude-airou' hook", "timeout": 10}]}
                    ]
                }
            }),
        );
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let report = installer.install().expect("install");

        assert!(report.updated_events.contains(&"PreToolUse".to_string()));
        assert_eq!(report.added_events.len(), SUBSCRIBED_EVENT_NAMES.len() - 1);
        assert!(report.backup_path.is_some(), "changed an existing file, backup expected");

        let value = read_json(&settings_path);
        assert_eq!(value["model"], json!("opus"), "foreign top-level keys kept");
        let groups = value["hooks"]["PreToolUse"].as_array().expect("groups");
        assert_eq!(groups.len(), 2, "no duplicate group added");
        assert_eq!(groups[0]["matcher"], json!("Bash"));
        assert_eq!(groups[0]["hooks"][0], foreign, "foreign handler untouched");
        assert_eq!(
            groups[1]["hooks"][0]["command"],
            json!("'/tmp/fake exe/claude-airou' hook"),
            "our stale command path updated in place"
        );
    }

    #[test]
    fn install_recognizes_legacy_claude_pet_marker() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        write_settings(
            &settings_path,
            &json!({
                "hooks": {
                    "Stop": [{"hooks": [{"type": "command", "command": "'/x/claude-pet' hook", "timeout": 10}]}]
                }
            }),
        );
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let report = installer.install().expect("install");
        assert!(report.updated_events.contains(&"Stop".to_string()));
        let value = read_json(&settings_path);
        let handlers = value["hooks"]["Stop"][0]["hooks"].as_array().expect("handlers");
        assert_eq!(handlers.len(), 1, "legacy entry replaced, not duplicated");
        assert_eq!(handlers[0]["command"], json!("'/tmp/fake exe/claude-airou' hook"));
    }

    #[test]
    fn install_refuses_non_object_hooks() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        write_settings(&settings_path, &json!({"hooks": "nope"}));
        let before = std::fs::read(&settings_path).expect("read");
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let error = installer.install().expect_err("must refuse");
        assert_eq!(
            error.to_string(),
            format!(
                "\"hooks\" in {} is not a JSON object; fix or remove it and re-run.",
                settings_path.display()
            )
        );
        assert_eq!(std::fs::read(&settings_path).expect("read"), before, "file untouched");
    }

    #[test]
    fn install_refuses_non_object_settings() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        std::fs::write(&settings_path, b"[1, 2]").expect("write");
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let error = installer.install().expect_err("must refuse");
        assert_eq!(
            error.to_string(),
            format!("{} is not a JSON object; refusing to modify it.", settings_path.display())
        );
    }

    #[test]
    fn install_refuses_malformed_group_shapes() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        write_settings(&settings_path, &json!({"hooks": {"Stop": {"not": "an array"}}}));
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let error = installer.install().expect_err("must refuse");
        assert_eq!(
            error.to_string(),
            format!(
                "hooks.Stop in {} is not an array of hook groups; fix or remove it and re-run.",
                settings_path.display()
            )
        );

        write_settings(&settings_path, &json!({"hooks": {"Stop": [{"hooks": "bad"}]}}));
        let error = installer.install().expect_err("must refuse");
        assert_eq!(
            error.to_string(),
            format!(
                "hooks.Stop[0].hooks in {} is not an array; fix or remove it and re-run.",
                settings_path.display()
            )
        );
    }

    // ---- hooks: uninstall ----

    #[test]
    fn uninstall_removes_only_ours_and_drops_empty_containers() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        let foreign = json!({"type": "command", "command": "other-tool run"});
        write_settings(
            &settings_path,
            &json!({
                "model": "opus",
                "hooks": {
                    "PreToolUse": [
                        {"matcher": "Bash", "hooks": [foreign.clone(), {"type": "command", "command": "'/tmp/fake exe/claude-airou' hook", "timeout": 10}]}
                    ],
                    "Stop": [
                        {"hooks": [{"type": "command", "command": "'/tmp/fake exe/claude-airou' hook", "timeout": 10}]}
                    ]
                }
            }),
        );
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let report = installer.uninstall().expect("uninstall");

        // Sorted event order (Swift iterates hooks.keys.sorted()).
        assert_eq!(report.removed_events, vec!["PreToolUse".to_string(), "Stop".to_string()]);
        assert!(report.backup_path.is_some());

        let value = read_json(&settings_path);
        assert_eq!(value["model"], json!("opus"));
        let hooks = value["hooks"].as_object().expect("hooks");
        assert!(!hooks.contains_key("Stop"), "event with only our group dropped");
        let handlers = hooks["PreToolUse"][0]["hooks"].as_array().expect("handlers");
        assert_eq!(handlers.len(), 1);
        assert_eq!(handlers[0], foreign, "foreign handler kept");
    }

    #[test]
    fn uninstall_drops_hooks_key_when_everything_was_ours() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        installer.install().expect("install");
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.removed_events.len(), SUBSCRIBED_EVENT_NAMES.len());
        let value = read_json(&settings_path);
        assert!(
            !value.as_object().expect("object").contains_key("hooks"),
            "empty hooks container removed"
        );
    }

    #[test]
    fn uninstall_without_hooks_key_is_noop() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        write_settings(&settings_path, &json!({"model": "opus"}));
        let before = std::fs::read(&settings_path).expect("read");
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let report = installer.uninstall().expect("uninstall");
        assert!(report.removed_events.is_empty());
        assert_eq!(report.backup_path, None);
        assert_eq!(std::fs::read(&settings_path).expect("read"), before);
    }

    #[test]
    fn uninstall_leaves_foreign_shapes_alone() {
        let dir = tempdir();
        let settings_path = dir.path().join("settings.json");
        write_settings(
            &settings_path,
            &json!({
                "hooks": {
                    "SessionStart": "weird shape",
                    "Stop": [{"hooks": [{"type": "command", "command": "'/tmp/fake exe/claude-airou' hook", "timeout": 10}]}]
                }
            }),
        );
        let installer = HooksInstaller::with_executable(settings_path.clone(), EXE);
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.removed_events, vec!["Stop".to_string()]);
        let value = read_json(&settings_path);
        assert_eq!(value["hooks"]["SessionStart"], json!("weird shape"), "foreign shape verbatim");
        assert!(!value["hooks"].as_object().expect("hooks").contains_key("Stop"));
    }

    // ---- hooks: recognition ----

    #[test]
    fn our_handler_recognition_variants() {
        // What we write.
        assert!(is_our_hook_handler(&handler_map("'/x/claude-airou' hook", Some(10)), EXE));
        // Legacy marker.
        assert!(is_our_hook_handler(&handler_map("'/x/claude-pet' hook", None), EXE));
        // Exec form: command carries the marker, args == ["hook"].
        let mut exec_form = handler_map("/usr/local/bin/claude-airou", None);
        exec_form.insert("args".to_string(), json!(["hook"]));
        assert!(is_our_hook_handler(&exec_form, EXE));
        // Exec form with a quoted path.
        let mut quoted = handler_map("'/usr/local/bin/claude-airou'", None);
        quoted.insert("args".to_string(), json!(["hook"]));
        assert!(is_our_hook_handler(&quoted, EXE));

        // Not ours: foreign command.
        assert!(!is_our_hook_handler(&handler_map("other-tool hook", None), EXE));
        // Not ours: marker but wrong subcommand and no args.
        assert!(!is_our_hook_handler(&handler_map("'/x/claude-airou' statusline", None), EXE));
        // Not ours: missing type.
        let mut no_type = Map::new();
        no_type.insert("command".to_string(), json!("'/x/claude-airou' hook"));
        assert!(!is_our_hook_handler(&no_type, EXE));
        // Not ours: wrong type.
        let mut wrong_type = handler_map("'/x/claude-airou' hook", None);
        wrong_type.insert("type".to_string(), json!("script"));
        assert!(!is_our_hook_handler(&wrong_type, EXE));
    }

    #[cfg(unix)]
    #[test]
    fn renamed_binary_resolves_via_canonicalize() {
        let dir = tempdir();
        let real_exe = dir.path().join("real-binary");
        std::fs::write(&real_exe, b"#!/bin/sh\n").expect("write exe");
        let symlink = dir.path().join("my-little-pet");
        std::os::unix::fs::symlink(&real_exe, &symlink).expect("symlink");

        // No marker anywhere, but the first word resolves to the same file as the exe.
        let command = format!("'{}' hook", symlink.display());
        let handler = handler_map(&command, Some(10));
        assert!(is_our_hook_handler(&handler, &real_exe.display().to_string()));

        // Different file: not ours.
        let other = dir.path().join("other-binary");
        std::fs::write(&other, b"#!/bin/sh\n").expect("write other");
        assert!(!is_our_hook_handler(&handler, &other.display().to_string()));
    }

    // ---- backups + quoting ----

    #[test]
    fn backup_naming_and_collision_suffix() {
        let dir = tempdir();
        let file = dir.path().join("settings.json");
        std::fs::write(&file, b"{}").expect("write");
        let first = backup_with_stamp(&file, "20260818-101112").expect("backup");
        assert!(first.ends_with("settings.json.claude-airou-backup-20260818-101112"));
        let second = backup_with_stamp(&file, "20260818-101112").expect("backup");
        assert!(second.ends_with("settings.json.claude-airou-backup-20260818-101112-1"));
        let third = backup_with_stamp(&file, "20260818-101112").expect("backup");
        assert!(third.ends_with("settings.json.claude-airou-backup-20260818-101112-2"));
        assert_eq!(std::fs::read(PathBuf::from(&first)).expect("read"), b"{}");
    }

    #[test]
    fn shell_single_quoting_escapes_embedded_quotes() {
        assert_eq!(shell_single_quoted("/plain/path"), "'/plain/path'");
        assert_eq!(shell_single_quoted("a'b"), "'a'\\''b'");
        assert_eq!(shell_single_quoted("sp ace$`\""), "'sp ace$`\"'");
    }

    #[test]
    fn hooks_snippet_json_shape() {
        let installer = HooksInstaller::with_executable(PathBuf::from("/nope/settings.json"), EXE);
        let snippet = installer.snippet_json();
        let value: Value = serde_json::from_str(&snippet).expect("snippet parses");
        let hooks = value["hooks"].as_object().expect("hooks");
        assert_eq!(hooks.len(), SUBSCRIBED_EVENT_NAMES.len());
        assert_eq!(
            hooks["SessionStart"][0]["hooks"][0]["command"],
            json!("'/tmp/fake exe/claude-airou' hook")
        );
    }

    // ---- status line ----

    /// Default-settings-mode installer rooted in a temp dir.
    fn statusline_installer(dir: &Path, exe: &str) -> StatusLineInstaller {
        let settings_path = dir.join("settings.json");
        StatusLineInstaller::for_test(settings_path.clone(), exe, settings_path, dir.join("airou-home"))
    }

    #[test]
    fn statusline_fresh_install_no_prior() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        let report = installer.install().expect("install");
        assert_eq!(report.action, "installed (there was no status line before)");
        assert_eq!(report.passthrough_command, None);
        assert_eq!(report.backup_path, None);

        let value = read_json(&installer.settings_path);
        assert_eq!(
            value["statusLine"],
            json!({"type": "command", "command": "'/tmp/fake exe/claude-airou' statusline"})
        );
        // Empty stash marker written.
        let stash = std::fs::read(installer.passthrough_file()).expect("stash exists");
        assert_eq!(stash, b"{}");
    }

    #[test]
    fn statusline_install_over_foreign_stashes_passthrough() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        let original = json!({"type": "command", "command": "my-status --fancy", "args": ["x"], "padding": 0});
        write_settings(&installer.settings_path, &json!({"statusLine": original}));

        let report = installer.install().expect("install");
        assert_eq!(report.action, "installed (original status line kept as passthrough)");
        assert_eq!(report.passthrough_command.as_deref(), Some("my-status --fancy"));
        assert!(report.backup_path.is_some());

        let value = read_json(&installer.settings_path);
        // type/command replaced, args dropped, padding carried over.
        assert_eq!(
            value["statusLine"],
            json!({"type": "command", "command": "'/tmp/fake exe/claude-airou' statusline", "padding": 0})
        );
        // Stash holds the original object verbatim.
        let stash: Value =
            serde_json::from_slice(&std::fs::read(installer.passthrough_file()).expect("stash")).expect("json");
        assert_eq!(stash, original);
    }

    #[test]
    fn statusline_reinstall_is_noop() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        write_settings(
            &installer.settings_path,
            &json!({"statusLine": {"type": "command", "command": "my-status"}}),
        );
        installer.install().expect("first install");
        let before = std::fs::read(&installer.settings_path).expect("read");
        let files_before = dir_file_names(dir.path());

        let report = installer.install().expect("second install");
        assert_eq!(report.action, "already installed");
        assert_eq!(report.passthrough_command.as_deref(), Some("my-status"));
        assert_eq!(report.backup_path, None);
        assert_eq!(std::fs::read(&installer.settings_path).expect("read"), before);
        assert_eq!(dir_file_names(dir.path()), files_before, "no extra backup on no-op");
    }

    #[test]
    fn statusline_updates_command_path_and_keeps_stash() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        write_settings(
            &installer.settings_path,
            &json!({"statusLine": {"type": "command", "command": "my-status"}}),
        );
        installer.install().expect("install with old exe");

        let moved = StatusLineInstaller::for_test(
            installer.settings_path.clone(),
            "/new home/claude-airou",
            installer.settings_path.clone(),
            dir.path().join("airou-home"),
        );
        let report = moved.install().expect("install with new exe");
        assert_eq!(report.action, "updated command path");
        assert_eq!(report.passthrough_command.as_deref(), Some("my-status"), "stash survives");
        let value = read_json(&moved.settings_path);
        assert_eq!(
            value["statusLine"]["command"],
            json!("'/new home/claude-airou' statusline")
        );
        let stash: Value =
            serde_json::from_slice(&std::fs::read(moved.passthrough_file()).expect("stash")).expect("json");
        assert_eq!(stash["command"], json!("my-status"));
    }

    #[test]
    fn statusline_stash_restore_round_trip() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        let original = json!({"type": "command", "command": "my-status", "padding": 0});
        write_settings(&installer.settings_path, &json!({"statusLine": original, "model": "opus"}));

        installer.install().expect("install");
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.action, "restored the original status line");
        assert_eq!(report.passthrough_command.as_deref(), Some("my-status"));

        let value = read_json(&installer.settings_path);
        assert_eq!(value["statusLine"], original, "round-trip restores the original");
        assert_eq!(value["model"], json!("opus"));
        assert!(!installer.passthrough_file().exists(), "stash removed after restore");
    }

    #[test]
    fn statusline_uninstall_merges_user_extras_into_restored() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        // Original had no padding.
        write_settings(
            &installer.settings_path,
            &json!({"statusLine": {"type": "command", "command": "my-status"}}),
        );
        installer.install().expect("install");
        // User added padding while ours was installed.
        let mut value = read_json(&installer.settings_path);
        value["statusLine"]["padding"] = json!(4);
        write_settings(&installer.settings_path, &value);

        installer.uninstall().expect("uninstall");
        let restored = read_json(&installer.settings_path);
        assert_eq!(
            restored["statusLine"],
            json!({"type": "command", "command": "my-status", "padding": 4}),
            "user's extra key survives the restore"
        );
    }

    #[test]
    fn statusline_uninstall_removes_when_no_prior() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        installer.install().expect("install");
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.action, "removed (there was no status line before)");
        let value = read_json(&installer.settings_path);
        assert!(!value.as_object().expect("object").contains_key("statusLine"));
        assert!(!installer.passthrough_file().exists());
    }

    #[test]
    fn statusline_uninstall_foreign_is_nothing_to_do() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        write_settings(
            &installer.settings_path,
            &json!({"statusLine": {"type": "command", "command": "someone-else"}}),
        );
        let before = std::fs::read(&installer.settings_path).expect("read");
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.action, "nothing to do (status line is not claude-airou's)");
        assert_eq!(report.backup_path, None);
        assert_eq!(std::fs::read(&installer.settings_path).expect("read"), before);
    }

    #[test]
    fn statusline_uninstall_corrupt_stash_refuses() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        installer.install().expect("install");
        std::fs::write(installer.passthrough_file(), b"not json {{{").expect("corrupt");
        let before = std::fs::read(&installer.settings_path).expect("read");

        let error = installer.uninstall().expect_err("must refuse");
        assert!(error.to_string().starts_with(&format!(
            "{} is unreadable; not touching settings.statusLine",
            installer.passthrough_file().display()
        )));
        assert_eq!(std::fs::read(&installer.settings_path).expect("read"), before, "settings untouched");
    }

    #[test]
    fn statusline_install_refuses_non_command_type() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        write_settings(&installer.settings_path, &json!({"statusLine": {"type": "static", "text": "hi"}}));
        let error = installer.install().expect_err("must refuse");
        assert_eq!(
            error.to_string(),
            format!(
                "statusLine in {} has type \"static\", which claude-airou cannot pass through. Remove it (or switch it to a command) and re-run.",
                installer.settings_path.display()
            )
        );
    }

    #[test]
    fn statusline_install_refuses_non_object_status_line() {
        let dir = tempdir();
        let installer = statusline_installer(dir.path(), EXE);
        write_settings(&installer.settings_path, &json!({"statusLine": "just a string"}));
        let error = installer.install().expect_err("must refuse");
        assert_eq!(
            error.to_string(),
            format!(
                "\"statusLine\" in {} is not a JSON object; fix or remove it and re-run.",
                installer.settings_path.display()
            )
        );
    }

    #[test]
    fn statusline_custom_settings_gets_digest_file_and_settings_flag() {
        let dir = tempdir();
        let settings_path = dir.path().join("work").join("settings.json");
        std::fs::create_dir_all(settings_path.parent().expect("parent")).expect("mkdir");
        let installer = StatusLineInstaller::for_test(
            settings_path.clone(),
            EXE,
            dir.path().join("default").join("settings.json"), // different → non-default mode
            dir.path().join("airou-home"),
        );

        let file_name = installer
            .passthrough_file()
            .file_name()
            .expect("name")
            .to_string_lossy()
            .to_string();
        assert!(file_name.starts_with("statusline-passthrough-"));
        assert!(file_name.ends_with(".json"));
        assert_ne!(file_name, "statusline-passthrough.json");
        // Deterministic digest.
        assert_eq!(installer.passthrough_file(), installer.passthrough_file());

        installer.install().expect("install");
        let value = read_json(&settings_path);
        assert_eq!(
            value["statusLine"]["command"],
            json!(format!(
                "'/tmp/fake exe/claude-airou' statusline --settings '{}'",
                settings_path.display()
            ))
        );
        assert!(installer.passthrough_file().exists());
    }

    #[test]
    fn is_self_invocation_variants() {
        assert!(is_self_invocation("'/x/claude-airou' statusline"));
        assert!(is_self_invocation("claude-pet statusline --settings '/x'"));
        assert!(!is_self_invocation("my-status statusline")); // no marker
        assert!(!is_self_invocation("'/x/claude-airou' hook")); // wrong subcommand
        assert!(!is_self_invocation("claude-airou-statusline")); // no separate word
    }

    #[test]
    fn is_our_status_line_variants() {
        let map = |v: Value| v.as_object().expect("object").clone();
        assert!(is_our_status_line(
            &map(json!({"type": "command", "command": "'/x/claude-airou' statusline"})),
            EXE
        ));
        // Type defaults to "command" when missing.
        assert!(is_our_status_line(&map(json!({"command": "'/x/claude-pet' statusline"})), EXE));
        assert!(!is_our_status_line(
            &map(json!({"type": "static", "command": "'/x/claude-airou' statusline"})),
            EXE
        ));
        assert!(!is_our_status_line(&map(json!({"type": "command", "command": "other statusline-x"})), EXE));
        assert!(!is_our_status_line(&map(json!({"type": "command"})), EXE));
    }

    #[test]
    fn base36_digest_matches_swift_formula() {
        // djb2 over "a": 5381 * 33 + 97 = 177670.
        let mut expected: u64 = 5381;
        expected = expected.wrapping_mul(33).wrapping_add(97);
        assert_eq!(settings_path_digest("a"), to_base36(expected));
        assert_eq!(to_base36(0), "0");
        assert_eq!(to_base36(35), "z");
        assert_eq!(to_base36(36), "10");
        // Different paths → different files.
        assert_ne!(settings_path_digest("/a/settings.json"), settings_path_digest("/b/settings.json"));
    }

    // ---- MCP ----

    #[test]
    fn mcp_fresh_install_adds_entry() {
        let dir = tempdir();
        let config_path = dir.path().join("claude_desktop_config.json");
        let installer = McpInstaller::with_executable(config_path.clone(), EXE);
        let report = installer.install().expect("install");
        assert_eq!(report.outcome, McpOutcome::Added);
        assert_eq!(report.backup_path, None);
        assert_eq!(report.server_command, "/tmp/fake exe/claude-airou mcp");

        let value = read_json(&config_path);
        assert_eq!(
            value["mcpServers"]["claude-airou"],
            json!({"command": "/tmp/fake exe/claude-airou", "args": ["mcp"]})
        );
        assert!(std::fs::read(&config_path).expect("read").ends_with(b"\n"));
    }

    #[test]
    fn mcp_reinstall_unchanged_is_byte_identical_noop() {
        let dir = tempdir();
        let config_path = dir.path().join("claude_desktop_config.json");
        let installer = McpInstaller::with_executable(config_path.clone(), EXE);
        installer.install().expect("first");
        let before = std::fs::read(&config_path).expect("read");
        let report = installer.install().expect("second");
        assert_eq!(report.outcome, McpOutcome::Unchanged);
        assert_eq!(report.backup_path, None);
        assert_eq!(std::fs::read(&config_path).expect("read"), before);
        assert_eq!(dir_file_names(dir.path()), vec!["claude_desktop_config.json"]);
    }

    #[test]
    fn mcp_update_rewrites_command_with_backup() {
        let dir = tempdir();
        let config_path = dir.path().join("claude_desktop_config.json");
        write_settings(
            &config_path,
            &json!({"mcpServers": {"claude-airou": {"command": "/old/claude-airou", "args": ["mcp"]}}}),
        );
        let installer = McpInstaller::with_executable(config_path.clone(), EXE);
        let report = installer.install().expect("install");
        assert_eq!(report.outcome, McpOutcome::Updated);
        assert!(report.backup_path.is_some());
        let value = read_json(&config_path);
        assert_eq!(value["mcpServers"]["claude-airou"]["command"], json!(EXE));
    }

    #[test]
    fn mcp_uninstall_removes_and_drops_empty_container() {
        let dir = tempdir();
        let config_path = dir.path().join("claude_desktop_config.json");
        let installer = McpInstaller::with_executable(config_path.clone(), EXE);
        installer.install().expect("install");
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.outcome, McpOutcome::Removed);
        let value = read_json(&config_path);
        assert!(!value.as_object().expect("object").contains_key("mcpServers"));
    }

    #[test]
    fn mcp_uninstall_preserves_foreign_servers() {
        let dir = tempdir();
        let config_path = dir.path().join("claude_desktop_config.json");
        write_settings(
            &config_path,
            &json!({
                "mcpServers": {
                    "claude-airou": {"command": EXE, "args": ["mcp"]},
                    "other": {"command": "/bin/other"}
                }
            }),
        );
        let installer = McpInstaller::with_executable(config_path.clone(), EXE);
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.outcome, McpOutcome::Removed);
        let value = read_json(&config_path);
        assert_eq!(value["mcpServers"]["other"], json!({"command": "/bin/other"}));
        assert!(!value["mcpServers"].as_object().expect("servers").contains_key("claude-airou"));
    }

    #[test]
    fn mcp_uninstall_absent_is_noop() {
        let dir = tempdir();
        let config_path = dir.path().join("claude_desktop_config.json");
        write_settings(&config_path, &json!({"mcpServers": {"other": {"command": "/bin/other"}}}));
        let before = std::fs::read(&config_path).expect("read");
        let installer = McpInstaller::with_executable(config_path.clone(), EXE);
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.outcome, McpOutcome::Absent);
        assert_eq!(std::fs::read(&config_path).expect("read"), before);

        // Missing container entirely.
        let empty_path = dir.path().join("empty.json");
        let installer = McpInstaller::with_executable(empty_path.clone(), EXE);
        let report = installer.uninstall().expect("uninstall");
        assert_eq!(report.outcome, McpOutcome::Absent);
        assert!(!empty_path.exists(), "no file created by a no-op uninstall");
    }

    #[test]
    fn mcp_refuses_non_object_servers() {
        let dir = tempdir();
        let config_path = dir.path().join("claude_desktop_config.json");
        write_settings(&config_path, &json!({"mcpServers": ["not", "an", "object"]}));
        let before = std::fs::read(&config_path).expect("read");
        let installer = McpInstaller::with_executable(config_path.clone(), EXE);
        let error = installer.install().expect_err("must refuse");
        assert_eq!(
            error.to_string(),
            format!(
                "\"mcpServers\" in {} is not a JSON object; fix or remove it and re-run.",
                config_path.display()
            )
        );
        assert_eq!(std::fs::read(&config_path).expect("read"), before);
    }

    #[test]
    fn mcp_snippet_json_shape() {
        let installer = McpInstaller::with_executable(PathBuf::from("/nope.json"), EXE);
        let value: Value = serde_json::from_str(&installer.snippet_json()).expect("parses");
        assert_eq!(
            value,
            json!({"mcpServers": {"claude-airou": {"command": EXE, "args": ["mcp"]}}})
        );
    }

    // ---- summaries ----

    #[test]
    fn summary_texts_match_swift_shapes() {
        let report = HooksReport {
            settings_path: "/s.json".to_string(),
            backup_path: Some("/s.json.claude-airou-backup-20260818-101112".to_string()),
            added_events: vec!["A".to_string(), "B".to_string()],
            updated_events: vec![],
            removed_events: vec![],
            unchanged_events: vec!["C".to_string()],
            hook_command: "'/x' hook".to_string(),
        };
        assert_eq!(
            report.summary_text(),
            "Settings: /s.json\nBackup:   /s.json.claude-airou-backup-20260818-101112\nCommand:  '/x' hook\nAdded:    A, B\nUnchanged: 1 event(s)"
        );

        let report = StatusLineReport {
            settings_path: "/s.json".to_string(),
            backup_path: None,
            action: "already installed".to_string(),
            passthrough_command: Some("my-status".to_string()),
        };
        assert_eq!(
            report.summary_text(),
            "Settings: /s.json\nAction:   already installed\nThen runs: my-status"
        );

        let report = McpReport {
            config_path: "/c.json".to_string(),
            backup_path: None,
            outcome: McpOutcome::Added,
            server_command: "/x mcp".to_string(),
        };
        assert_eq!(
            report.summary_text(),
            "Config:  /c.json\nCommand: /x mcp\nAdded:   mcpServers.claude-airou"
        );
    }
}
