//! Core data types, format-compatible on disk with the Swift app's `Models/PetState.swift`,
//! `Models/SessionUsageSnapshot.swift` and `State/AppConfig.swift`. Field names and enum raw
//! values must never drift from the Swift side: both implementations read and write the same
//! `~/.claude-airou` files. ("Format-compatible" = each side decodes the other's files;
//! key order and number formatting may differ — Swift sorts keys and prints whole doubles
//! without a fraction, serde_json keeps declaration order and prints `1.0`.)

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

    /// How long the speech bubble of a sticky done / error result stays up before only the
    /// badge remains (see `transient_duration_secs`).
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay bubble timing
    pub const RESULT_BUBBLE_LINGER_SECS: f64 = 30.0;

    /// Seconds after which the state decays back to `idle` if no newer event arrives.
    /// `None` means the state is sticky. Mirrors `PetState.transientDurationSeconds`.
    pub fn transient_duration_secs(self) -> Option<f64> {
        match self {
            PetState::Hello => Some(4.0),
            // done / error stay until the next event (a new prompt, SessionEnd, …): the
            // result should still be on screen when the user comes back to look — a
            // 6-second flash was gone before anyone saw it. Only the bubble hides after
            // `RESULT_BUBBLE_LINGER_SECS`; the badge and label icon remain.
            PetState::Done | PetState::Error => None,
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
    /// Subagents working for this session, each with the last time we heard from it. The
    /// overlay draws one shadow clone per agent still inside `AGENT_ALIVE_WINDOW_SECS`.
    ///
    /// Timestamps rather than a plain list because the retire signal is unreliable: a
    /// `SubagentStop` can be dropped by the merge policy, lost to a race between two hook
    /// processes writing the same file, or never sent at all. An id nobody has heard from
    /// in a while simply stops counting, so every one of those leaks heals itself.
    #[serde(skip_serializing_if = "Vec::is_empty", default)]
    pub active_agents: Vec<ActiveAgent>,
}

/// One subagent working for a session.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActiveAgent {
    pub id: String,
    pub last_seen_epoch_seconds: f64,
}

impl SessionSnapshot {
    /// More than this many concurrent agents changes nothing on screen, so the stalest is
    /// dropped rather than the list grown without bound.
    pub const MAXIMUM_TRACKED_AGENTS: usize = 8;

    /// An agent nobody has heard from for this long is treated as finished. Agents that are
    /// genuinely working emit events far more often than this (every tool call), so the
    /// window only ever expires the ones whose stop went missing.
    pub const AGENT_ALIVE_WINDOW_SECS: f64 = 90.0;

    /// Agents still counting as alive at `now`.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay-only
    pub fn live_agent_count(&self, now_secs: f64) -> usize {
        self.active_agents
            .iter()
            .filter(|agent| now_secs - agent.last_seen_epoch_seconds <= Self::AGENT_ALIVE_WINDOW_SECS)
            .count()
    }

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

    /// True for a session that has only been opened (`SessionStart` from Claude Code, or the
    /// MCP server's `initialize`) and has done nothing since — once its hello wave has
    /// decayed there is nothing to show, so the overlay hides it until the first real
    /// event (prompt, tool call, …) rewrites the file. Merely clicking through past
    /// sessions in the desktop app resumes each one and would otherwise leave a pet behind.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay row filtering
    pub fn is_opened_without_activity(&self) -> bool {
        matches!(self.last_event_name.as_str(), "SessionStart" | "mcp:initialize")
            && self.effective_state() == PetState::Idle
    }
}

/// Reasoning effort for a session, as the Claude Code status line reports it
/// (`effort.level`). Absent when the model has no effort parameter, so every consumer
/// treats "unknown" as "draw nothing".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EffortLevel {
    Low,
    Medium,
    High,
    #[serde(rename = "xhigh")]
    XHigh,
    Max,
}

impl EffortLevel {
    pub fn from_raw(raw: &str) -> Option<EffortLevel> {
        match raw.trim().to_ascii_lowercase().as_str() {
            "low" => Some(EffortLevel::Low),
            "medium" => Some(EffortLevel::Medium),
            "high" => Some(EffortLevel::High),
            "xhigh" => Some(EffortLevel::XHigh),
            "max" => Some(EffortLevel::Max),
            _ => None, // a numeric token budget or a level we do not know yet
        }
    }

    pub fn raw(self) -> &'static str {
        match self {
            EffortLevel::Low => "low",
            EffortLevel::Medium => "medium",
            EffortLevel::High => "high",
            EffortLevel::XHigh => "xhigh",
            EffortLevel::Max => "max",
        }
    }

    /// The aura behind the pet: (inner radius as a multiple of the sprite's half-height,
    /// how much of the available room the halo fills, peak opacity).
    ///
    /// The outer edge is a *fraction of the room that exists* rather than a fixed multiple
    /// of the sprite, because the halo has to fit whatever the layout could reserve: the
    /// radius grows with the pet (Small/Medium/Large) while the panel cannot grow with it
    /// without eating the desktop. Spending a share of the real room keeps every level
    /// distinct at every size and makes a mid-slope cut impossible.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay-only
    pub fn aura_inner_scale_band_and_opacity(self) -> (f32, f32, f32) {
        match self {
            EffortLevel::Low => (0.78, 0.34, 0.14),
            EffortLevel::Medium => (0.84, 0.52, 0.21),
            EffortLevel::High => (0.90, 0.68, 0.28),
            EffortLevel::XHigh => (0.96, 0.85, 0.36),
            EffortLevel::Max => (1.02, 1.00, 0.45),
        }
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
    pub effort_level: Option<EffortLevel>,

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
        // Without this the aura blinks off every time an update arrives from a source that
        // did not see the level (an older transcript line, a model without the parameter).
        if self.effort_level.is_none() {
            self.effort_level = previous.effort_level;
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
    pub fn from_raw(raw: &str) -> Option<GaugeMetric> {
        match raw {
            "context_remaining" => Some(GaugeMetric::ContextRemaining),
            "five_hour_remaining" => Some(GaugeMetric::FiveHourRemaining),
            "seven_day_remaining" => Some(GaugeMetric::SevenDayRemaining),
            "off" => Some(GaugeMetric::Off),
            _ => None,
        }
    }

    /// Swift `shortLabel`: the tiny metric tag next to the gauge percentage.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // overlay gauge
    pub fn short_label(self) -> &'static str {
        match self {
            GaugeMetric::ContextRemaining => "ctx",
            GaugeMetric::FiveHourRemaining => "5h",
            GaugeMetric::SevenDayRemaining => "7d",
            GaugeMetric::Off => "",
        }
    }

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
    pub is_effort_aura_hidden: bool,
    pub is_agent_shadows_hidden: bool,
    #[serde(deserialize_with = "lenient_gauge_metric")]
    pub gauge_metric: GaugeMetric,
}

/// Unknown or null `gaugeMetric` string values fall back to the default while the rest of
/// the config is kept — mirrors Swift's AppConfig ("unknown value → default, keep the rest
/// of the config"). A non-string value still fails the whole decode, also like Swift.
fn lenient_gauge_metric<'de, D>(deserializer: D) -> Result<GaugeMetric, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let raw = Option::<String>::deserialize(deserializer)?;
    Ok(raw.as_deref().and_then(GaugeMetric::from_raw).unwrap_or_default())
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
            is_effort_aura_hidden: false,
            is_agent_shadows_hidden: false,
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
    fn a_merge_never_loses_a_known_effort_level() {
        let base = SessionUsageSnapshot {
            session_id: "s1".into(),
            source: UsageSource::Transcript,
            effort_level: Some(EffortLevel::High),
            updated_at_epoch_seconds: 0.0,
            context_used_percentage: Some(10.0),
            context_window_size: None,
            context_tokens: Some(1),
            total_input_tokens: None,
            total_output_tokens: None,
            model_display_name: None,
            five_hour_used_percentage: None,
            five_hour_resets_at_epoch_seconds: None,
            seven_day_used_percentage: None,
            seven_day_resets_at_epoch_seconds: None,
            total_cost_usd: None,
        };
        // A later reading that saw no level keeps the one we already knew: otherwise the
        // aura would blink off on every refresh from a line without the key.
        let mut newer = base.clone();
        newer.effort_level = None;
        newer.context_tokens = Some(2);
        newer.context_used_percentage = Some(20.0);
        let merged = base.merged(&newer, 1.0).expect("merge writes");
        assert_eq!(merged.effort_level, Some(EffortLevel::High));

        // A newer reading that saw a level replaces it (mid-session /effort changes).
        let mut changed = base.clone();
        changed.effort_level = Some(EffortLevel::Low);
        changed.context_tokens = Some(3);
        changed.context_used_percentage = Some(30.0);
        let merged = base.merged(&changed, 1.0).expect("merge writes");
        assert_eq!(merged.effort_level, Some(EffortLevel::Low));
    }

    #[test]
    fn aura_grows_and_brightens_with_effort() {
        let mut previous = (0.0f32, 0.0f32, 0.0f32);
        for level in [
            EffortLevel::Low,
            EffortLevel::Medium,
            EffortLevel::High,
            EffortLevel::XHigh,
            EffortLevel::Max,
        ] {
            let (inner, band, opacity) = level.aura_inner_scale_band_and_opacity();
            assert!(inner > previous.0, "{level:?} should start further out than the level below");
            assert!(band > previous.1, "{level:?} should reach further than the level below");
            assert!(opacity > previous.2, "{level:?} should be brighter than the level below");
            assert!(opacity <= 0.45, "{level:?} must stay a glow, not a lamp");
            // A band is a share of the room that exists, so it can never exceed it. (The old
            // form of this check compared a radius ratio against a point budget and passed
            // while the halo was being cut in half.)
            assert!(band > 0.0 && band <= 1.0, "{level:?} must spend a share of the room, not more");
            previous = (inner, band, opacity);
        }
    }

    #[test]
    fn effort_level_parses_what_the_status_line_sends() {
        assert_eq!(EffortLevel::from_raw("high"), Some(EffortLevel::High));
        assert_eq!(EffortLevel::from_raw("XHIGH"), Some(EffortLevel::XHigh));
        assert_eq!(EffortLevel::from_raw(" max "), Some(EffortLevel::Max));
        // A numeric token budget, or a level from a newer Claude Code: no aura, no crash.
        assert_eq!(EffortLevel::from_raw("31999"), None);
        assert_eq!(EffortLevel::from_raw(""), None);
        for level in [EffortLevel::Low, EffortLevel::XHigh, EffortLevel::Max] {
            assert_eq!(EffortLevel::from_raw(level.raw()), Some(level), "raw() round-trips");
        }
    }

    #[test]
    fn usage_snapshot_keys_match_swift() {
        let usage = SessionUsageSnapshot {
            session_id: "s".into(),
            source: UsageSource::StatusLine,
            effort_level: None,
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
            active_agents: Vec::new(),
        };
        assert_eq!(snapshot.effective_state(), PetState::Idle);
    }

    #[test]
    fn done_and_error_are_sticky_until_the_next_event() {
        for state in [PetState::Done, PetState::Error] {
            let snapshot = SessionSnapshot {
                session_id: "s".into(),
                cwd: "/tmp".into(),
                state,
                message: "Done!".into(),
                last_event_name: "Stop".into(),
                tool_name: None,
                updated_at_epoch_seconds: now_epoch_secs() - 3.0 * 60.0 * 60.0,
                pending_tool_use_id: None,
                active_agents: Vec::new(),
            };
            assert_eq!(snapshot.effective_state(), state, "{state:?} must not decay");
        }
    }

    #[test]
    fn opened_without_activity_only_after_the_hello_wave_decays() {
        let mut snapshot = SessionSnapshot {
            session_id: "s".into(),
            cwd: "/w/p".into(),
            state: PetState::Hello,
            message: "Welcome back!".into(),
            last_event_name: "SessionStart".into(),
            tool_name: None,
            updated_at_epoch_seconds: now_epoch_secs(),
            pending_tool_use_id: None,
            active_agents: Vec::new(),
        };
        // Fresh resume: the wave is still showing.
        assert!(!snapshot.is_opened_without_activity());
        // Wave decayed, nothing else happened: hidden.
        snapshot.updated_at_epoch_seconds = now_epoch_secs() - 60.0;
        assert!(snapshot.is_opened_without_activity());
        // The MCP server's initialize counts the same way.
        snapshot.last_event_name = "mcp:initialize".into();
        assert!(snapshot.is_opened_without_activity());
        // Any real event (even one that decayed to idle) keeps the session visible.
        snapshot.last_event_name = "UserPromptSubmit".into();
        assert!(!snapshot.is_opened_without_activity());
        snapshot.last_event_name = "SessionStart".into();
        snapshot.state = PetState::Working;
        snapshot.updated_at_epoch_seconds = now_epoch_secs();
        assert!(!snapshot.is_opened_without_activity());
    }

    #[test]
    fn unknown_gauge_metric_keeps_the_rest_of_the_config() {
        // Swift AppConfig: "unknown value → default, keep the rest of the config".
        let json = r#"{"pixelScale": 7.0, "selectedPetId": "mochi-cat", "gaugeMetric": "some_future_metric"}"#;
        let config: AppConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.gauge_metric, GaugeMetric::ContextRemaining);
        assert_eq!(config.pixel_scale, 7.0);
        assert_eq!(config.selected_pet_id.as_deref(), Some("mochi-cat"));

        let null_json = r#"{"pixelScale": 3.0, "gaugeMetric": null}"#;
        let config: AppConfig = serde_json::from_str(null_json).unwrap();
        assert_eq!(config.gauge_metric, GaugeMetric::ContextRemaining);
        assert_eq!(config.pixel_scale, 3.0);

        // Non-string still fails the whole decode (Swift throws there too); load_from then
        // falls back to a full default config.
        assert!(serde_json::from_str::<AppConfig>(r#"{"gaugeMetric": 3}"#).is_err());

        let known = r#"{"gaugeMetric": "five_hour_remaining"}"#;
        let config: AppConfig = serde_json::from_str(known).unwrap();
        assert_eq!(config.gauge_metric, GaugeMetric::FiveHourRemaining);
    }

    #[test]
    fn parse_aliases() {
        assert_eq!(PetState::parse("waitingApproval"), Some(PetState::WaitingApproval));
        assert_eq!(PetState::parse("ok"), Some(PetState::Done));
        assert_eq!(PetState::parse("busy"), Some(PetState::Working));
        assert_eq!(PetState::parse("nope"), None);
    }
}
