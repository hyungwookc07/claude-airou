//! Core data types, byte-compatible on disk with the Swift app's `Models/PetState.swift`,
//! `Models/SessionUsageSnapshot.swift` and `State/AppConfig.swift`. Field names and enum raw
//! values must never drift from the Swift side: both implementations read and write the same
//! `~/.claude-airou` files.

use serde::{Deserialize, Serialize};

/// The mood/state a pet can display.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PetState {
    Hello,
    Idle,
    Thinking,
    Working,
    WaitingApproval,
    NeedsInput,
    Done,
    Error,
}

impl PetState {
    pub const ALL: [PetState; 8] = [
        PetState::Hello,
        PetState::Idle,
        PetState::Thinking,
        PetState::Working,
        PetState::WaitingApproval,
        PetState::NeedsInput,
        PetState::Done,
        PetState::Error,
    ];

    pub fn raw(self) -> &'static str {
        match self {
            PetState::Hello => "hello",
            PetState::Idle => "idle",
            PetState::Thinking => "thinking",
            PetState::Working => "working",
            PetState::WaitingApproval => "waiting_approval",
            PetState::NeedsInput => "needs_input",
            PetState::Done => "done",
            PetState::Error => "error",
        }
    }

    pub fn is_busy(self) -> bool {
        matches!(self, PetState::Thinking | PetState::Working)
    }

    pub fn is_attention_needed(self) -> bool {
        matches!(self, PetState::WaitingApproval | PetState::NeedsInput)
    }

    /// Seconds after which the state decays back to `idle` if no newer event arrives.
    /// `None` means the state is sticky. Mirrors `PetState.transientDurationSeconds`.
    pub fn transient_duration_secs(self) -> Option<f64> {
        match self {
            PetState::Hello => Some(4.0),
            PetState::Done => Some(6.0),
            PetState::Error => Some(8.0),
            PetState::WaitingApproval | PetState::NeedsInput => Some(20.0 * 60.0),
            PetState::Thinking | PetState::Working => Some(15.0 * 60.0),
            PetState::Idle => None,
        }
    }

    /// Fallback chain used when a pet JSON lacks frames for a state.
    pub fn fallback_states(self) -> &'static [PetState] {
        match self {
            PetState::Hello => &[PetState::Done, PetState::Idle],
            PetState::Working => &[PetState::Thinking, PetState::Idle],
            PetState::WaitingApproval => &[PetState::NeedsInput, PetState::Idle],
            PetState::NeedsInput => &[PetState::WaitingApproval, PetState::Idle],
            PetState::Thinking | PetState::Done | PetState::Error => &[PetState::Idle],
            PetState::Idle => &[],
        }
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay-only today
    pub fn display_label(self) -> &'static str {
        match self {
            PetState::Hello => "Hello",
            PetState::Idle => "Idle",
            PetState::Thinking => "Thinking",
            PetState::Working => "Working",
            PetState::WaitingApproval => "Waiting for approval",
            PetState::NeedsInput => "Needs your input",
            PetState::Done => "Done",
            PetState::Error => "Error",
        }
    }

    /// Accepts both `waiting_approval` and `waitingApproval` spellings plus the same
    /// aliases as the Swift CLI (`ok`, `busy`, `failed`, …).
    pub fn parse(text: &str) -> Option<PetState> {
        let normalized = text.replace('-', "_").to_lowercase();
        match normalized.as_str() {
            "hello" => Some(PetState::Hello),
            "idle" => Some(PetState::Idle),
            "thinking" => Some(PetState::Thinking),
            "working" | "busy" => Some(PetState::Working),
            "waiting_approval" | "waitingapproval" | "waiting" | "approval" | "permission" => {
                Some(PetState::WaitingApproval)
            }
            "needs_input" | "needsinput" | "input" | "question" => Some(PetState::NeedsInput),
            "done" | "ok" | "success" | "complete" | "completed" | "finished" => {
                Some(PetState::Done)
            }
            "error" | "fail" | "failed" | "failure" => Some(PetState::Error),
            _ => None,
        }
    }
}

pub fn now_epoch_secs() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs_f64())
        .unwrap_or(0.0)
}

/// One Claude Code / Claude chat session as last reported by the hook or the MCP server.
/// On-disk key names match Swift's `SessionSnapshot` (Codable camelCase).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionSnapshot {
    pub session_id: String,
    pub cwd: String,
    pub state: PetState,
    pub message: String,
    pub last_event_name: String,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub tool_name: Option<String>,
    pub updated_at_epoch_seconds: f64,
    /// While waiting on the user for a specific tool call, the id of that call.
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub pending_tool_use_id: Option<String>,
}

impl SessionSnapshot {
    pub fn project_name(&self) -> String {
        let name = std::path::Path::new(&self.cwd)
            .file_name()
            .map(|component| component.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            self.cwd.clone()
        } else {
            name
        }
    }

    pub fn age_secs(&self) -> f64 {
        now_epoch_secs() - self.updated_at_epoch_seconds
    }

    /// The state to actually show right now, after transient decay.
    pub fn effective_state(&self) -> PetState {
        if let Some(duration) = self.state.transient_duration_secs() {
            if self.age_secs() > duration {
                return PetState::Idle;
            }
        }
        self.state
    }
}

/// Usage figures for one session (`<session>.usage.json`). Mirrors `SessionUsageSnapshot`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionUsageSnapshot {
    pub session_id: String,
    pub source: UsageSource,
    pub updated_at_epoch_seconds: f64,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_used_percentage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_window_size: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub context_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_input_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub total_output_tokens: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub model_display_name: Option<String>,

    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub five_hour_used_percentage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub five_hour_resets_at_epoch_seconds: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub seven_day_used_percentage: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none", default)]
    pub seven_day_resets_at_epoch_seconds: Option<f64>,

    #[serde(rename = "totalCostUSD", skip_serializing_if = "Option::is_none", default)]
    pub total_cost_usd: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum UsageSource {
    #[serde(rename = "status_line")]
    StatusLine,
    #[serde(rename = "transcript")]
    Transcript,
}

impl UsageSource {
    pub fn raw(self) -> &'static str {
        match self {
            UsageSource::StatusLine => "status_line",
            UsageSource::Transcript => "transcript",
        }
    }
}

impl SessionUsageSnapshot {
    /// The status line is authoritative; a transcript estimate must not overwrite a recent reading.
    pub const STATUS_LINE_AUTHORITY_WINDOW_SECS: f64 = 120.0;
    /// Identical transcript estimates are re-written at most this often.
    pub const TRANSCRIPT_REWRITE_INTERVAL_SECS: f64 = 300.0;

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay gauge
    pub fn context_remaining_percentage(&self) -> Option<f64> {
        self.context_used_percentage
            .map(|used| (100.0 - used).clamp(0.0, 100.0))
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay gauge
    pub fn five_hour_remaining_percentage(&self) -> Option<f64> {
        self.five_hour_used_percentage
            .map(|used| (100.0 - used).clamp(0.0, 100.0))
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay gauge
    pub fn seven_day_remaining_percentage(&self) -> Option<f64> {
        self.seven_day_used_percentage
            .map(|used| (100.0 - used).clamp(0.0, 100.0))
    }

    /// True when the two snapshots carry the same figures (timestamps ignored).
    pub fn has_same_figures(&self, other: &SessionUsageSnapshot) -> bool {
        let mut a = self.clone();
        let mut b = other.clone();
        a.updated_at_epoch_seconds = 0.0;
        b.updated_at_epoch_seconds = 0.0;
        a == b
    }

    fn inherit_status_line_only_fields(&mut self, previous: &SessionUsageSnapshot) {
        if self.five_hour_used_percentage.is_none() {
            self.five_hour_used_percentage = previous.five_hour_used_percentage;
        }
        if self.five_hour_resets_at_epoch_seconds.is_none() {
            self.five_hour_resets_at_epoch_seconds = previous.five_hour_resets_at_epoch_seconds;
        }
        if self.seven_day_used_percentage.is_none() {
            self.seven_day_used_percentage = previous.seven_day_used_percentage;
        }
        if self.seven_day_resets_at_epoch_seconds.is_none() {
            self.seven_day_resets_at_epoch_seconds = previous.seven_day_resets_at_epoch_seconds;
        }
        if self.total_cost_usd.is_none() {
            self.total_cost_usd = previous.total_cost_usd;
        }
        if self.total_input_tokens.is_none() {
            self.total_input_tokens = previous.total_input_tokens;
        }
        if self.total_output_tokens.is_none() {
            self.total_output_tokens = previous.total_output_tokens;
        }
        if self.model_display_name.is_none() {
            self.model_display_name = previous.model_display_name.clone();
        }
    }

    /// What should be on disk after `candidate` arrives on top of `self`; `None` = leave the
    /// file alone. Mirrors Swift's `merged(with:now:)`.
    pub fn merged(&self, candidate: &SessionUsageSnapshot, now_secs: f64) -> Option<SessionUsageSnapshot> {
        let mut result = candidate.clone();
        match (self.source, candidate.source) {
            (UsageSource::StatusLine, UsageSource::Transcript) => {
                if now_secs - self.updated_at_epoch_seconds < Self::STATUS_LINE_AUTHORITY_WINDOW_SECS {
                    return None;
                }
                if let (Some(window), Some(tokens)) = (self.context_window_size, candidate.context_tokens) {
                    if window > 0 {
                        result.context_window_size = Some(window);
                        result.context_used_percentage =
                            Some((tokens as f64 / window as f64 * 100.0).min(100.0));
                    }
                }
                result.inherit_status_line_only_fields(self);
            }
            (UsageSource::Transcript, UsageSource::Transcript) => {
                if self.has_same_figures(candidate)
                    && now_secs - self.updated_at_epoch_seconds < Self::TRANSCRIPT_REWRITE_INTERVAL_SECS
                {
                    return None;
                }
                result.inherit_status_line_only_fields(self);
            }
            (_, UsageSource::StatusLine) => {
                if result.context_window_size.is_none() {
                    result.context_window_size = self.context_window_size;
                }
                if result.model_display_name.is_none() {
                    result.model_display_name = self.model_display_name.clone();
                }
            }
        }
        Some(result)
    }
}

/// Which figure the battery gauge shows. Raw values match Swift's `GaugeMetric`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum GaugeMetric {
    #[default]
    #[serde(rename = "context_remaining")]
    ContextRemaining,
    #[serde(rename = "five_hour_remaining")]
    FiveHourRemaining,
    #[serde(rename = "seven_day_remaining")]
    SevenDayRemaining,
    #[serde(rename = "off")]
    Off,
}

impl GaugeMetric {
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay gauge
    pub fn value(self, usage: Option<&SessionUsageSnapshot>) -> Option<f64> {
        let usage = usage?;
        match self {
            GaugeMetric::ContextRemaining => usage.context_remaining_percentage(),
            GaugeMetric::FiveHourRemaining => usage.five_hour_remaining_percentage(),
            GaugeMetric::SevenDayRemaining => usage.seven_day_remaining_percentage(),
            GaugeMetric::Off => None,
        }
    }
}

/// User preferences at `~/.claude-airou/config.json`, shared with the Swift app.
/// Every key is optional on decode so partially written / older files load fine.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AppConfig {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub selected_pet_id: Option<String>,
    pub pixel_scale: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_origin_x: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub window_origin_y: Option<f64>,
    pub is_speech_bubble_hidden: bool,
    pub is_click_through: bool,
    pub is_pet_hidden: bool,
    pub is_sessions_always_expanded: bool,
    pub gauge_metric: GaugeMetric,
}

impl Default for AppConfig {
    fn default() -> Self {
        AppConfig {
            selected_pet_id: None,
            pixel_scale: 5.0,
            window_origin_x: None,
            window_origin_y: None,
            is_speech_bubble_hidden: false,
            is_click_through: false,
            is_pet_hidden: false,
            is_sessions_always_expanded: false,
            gauge_metric: GaugeMetric::ContextRemaining,
        }
    }
}

impl AppConfig {
    pub fn load() -> AppConfig {
        Self::load_from(&crate::paths::config_file())
    }

    pub fn load_from(path: &std::path::Path) -> AppConfig {
        std::fs::read(path)
            .ok()
            .and_then(|data| serde_json::from_slice::<AppConfig>(&data).ok())
            .map(|mut config| {
                if !config.pixel_scale.is_finite() {
                    config.pixel_scale = 5.0;
                }
                config.pixel_scale = config.pixel_scale.clamp(1.0, 12.0);
                config
            })
            .unwrap_or_default()
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay persists config
    pub fn save(&self) {
        self.save_to(&crate::paths::config_file());
    }

    pub fn save_to(&self, path: &std::path::Path) {
        if let Some(parent) = path.parent() {
            let _ = crate::paths::ensure_dir(parent);
        }
        if let Ok(data) = serde_json::to_vec_pretty(self) {
            let _ = crate::state_store::write_atomic(path, &data);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pet_state_raw_values_match_swift() {
        assert_eq!(
            serde_json::to_string(&PetState::WaitingApproval).unwrap(),
            "\"waiting_approval\""
        );
        assert_eq!(serde_json::to_string(&PetState::NeedsInput).unwrap(), "\"needs_input\"");
        assert_eq!(serde_json::to_string(&PetState::Hello).unwrap(), "\"hello\"");
        for state in PetState::ALL {
            assert_eq!(serde_json::to_string(&state).unwrap(), format!("\"{}\"", state.raw()));
        }
    }

    #[test]
    fn snapshot_round_trips_swift_json() {
        // Exactly what the Swift hook writes.
        let swift_json = r#"{"cwd":"/Users/me/project","lastEventName":"PreToolUse","message":"Reading main.swift","pendingToolUseId":"toolu_123","sessionId":"abc-123","state":"waiting_approval","toolName":"Read","updatedAtEpochSeconds":1755500000.25}"#;
        let snapshot: SessionSnapshot = serde_json::from_str(swift_json).unwrap();
        assert_eq!(snapshot.session_id, "abc-123");
        assert_eq!(snapshot.state, PetState::WaitingApproval);
        assert_eq!(snapshot.project_name(), "project");
        let reencoded = serde_json::to_value(&snapshot).unwrap();
        let original: serde_json::Value = serde_json::from_str(swift_json).unwrap();
        assert_eq!(reencoded, original);
    }

    #[test]
    fn snapshot_tolerates_missing_optionals() {
        let json = r#"{"cwd":"Claude Chat","lastEventName":"mcp:initialize","message":"Hi","sessionId":"claude-chat-1","state":"hello","updatedAtEpochSeconds":1.0}"#;
        let snapshot: SessionSnapshot = serde_json::from_str(json).unwrap();
        assert!(snapshot.tool_name.is_none());
        assert!(snapshot.pending_tool_use_id.is_none());
        assert_eq!(snapshot.project_name(), "Claude Chat");
    }

    #[test]
    fn usage_snapshot_keys_match_swift() {
        let usage = SessionUsageSnapshot {
            session_id: "s".into(),
            source: UsageSource::StatusLine,
            updated_at_epoch_seconds: 1.0,
            context_used_percentage: Some(40.0),
            context_window_size: Some(200000),
            context_tokens: None,
            total_input_tokens: None,
            total_output_tokens: None,
            model_display_name: None,
            five_hour_used_percentage: None,
            five_hour_resets_at_epoch_seconds: None,
            seven_day_used_percentage: None,
            seven_day_resets_at_epoch_seconds: None,
            total_cost_usd: Some(1.5),
        };
        let value = serde_json::to_value(&usage).unwrap();
        assert_eq!(value["source"], "status_line");
        assert!(value.get("contextUsedPercentage").is_some());
        assert!(value.get("totalCostUSD").is_some());
        assert!(value.get("sessionId").is_some());
    }

    #[test]
    fn effective_state_decays() {
        let snapshot = SessionSnapshot {
            session_id: "s".into(),
            cwd: "/tmp".into(),
            state: PetState::Hello,
            message: String::new(),
            last_event_name: "SessionStart".into(),
            tool_name: None,
            updated_at_epoch_seconds: now_epoch_secs() - 10.0,
            pending_tool_use_id: None,
        };
        assert_eq!(snapshot.effective_state(), PetState::Idle);
    }

    #[test]
    fn parse_aliases() {
        assert_eq!(PetState::parse("waitingApproval"), Some(PetState::WaitingApproval));
        assert_eq!(PetState::parse("ok"), Some(PetState::Done));
        assert_eq!(PetState::parse("busy"), Some(PetState::Working));
        assert_eq!(PetState::parse("nope"), None);
    }
}
