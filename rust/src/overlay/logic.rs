//! Pure overlay state logic: which session the pet represents, what it says, which
//! animation frame is showing. Port of the non-layout parts of
//! `Sources/ClaudeAirou/UI/PetViewModel.swift` (fan-out and per-session pinning are
//! deferred in the Rust v0.1 overlay, so the "pinned session wins" branch is absent —
//! the rest of the focus rule is identical: attention-needed > busy > most recent).

use crate::model::{GaugeMetric, PetState, SessionSnapshot, SessionUsageSnapshot};
use std::collections::HashMap;

/// Seconds a pet-click reaction phrase stays on screen (Swift: `petReactionDurationSeconds`).
pub const PET_REACTION_DURATION_SECS: f64 = 2.5;

pub struct OverlayModel {
    pub sessions: Vec<SessionSnapshot>,
    pub usage_by_session: HashMap<String, SessionUsageSnapshot>,
    pub focused: Option<SessionSnapshot>,
    pub display_state: PetState,
    pub display_message: String,
    pub frame_index: usize,
    frame_accumulator_secs: f64,
    pub pet_reaction_message: Option<String>,
    pet_reaction_expires_at_secs: Option<f64>,
    previous_display_state: PetState,
    previous_focused_session_id: Option<String>,
}

impl Default for OverlayModel {
    fn default() -> Self {
        OverlayModel {
            sessions: Vec::new(),
            usage_by_session: HashMap::new(),
            focused: None,
            display_state: PetState::Idle,
            display_message: String::new(),
            frame_index: 0,
            frame_accumulator_secs: 0.0,
            pet_reaction_message: None,
            pet_reaction_expires_at_secs: None,
            previous_display_state: PetState::Idle,
            previous_focused_session_id: None,
        }
    }
}

impl OverlayModel {
    pub fn new() -> OverlayModel {
        OverlayModel::default()
    }

    /// Ingests a fresh `StateStore::load_all()` result (already newest-first) and the
    /// usage snapshots. Mirrors `PetViewModel.reloadSessions()`:
    /// attention-needed > busy > most recently updated; the message shows only while
    /// the effective state is not idle; a state or focus change restarts the animation.
    pub fn reload(&mut self, sessions: Vec<SessionSnapshot>, usage: Vec<SessionUsageSnapshot>) {
        self.usage_by_session = usage
            .into_iter()
            .map(|snapshot| (snapshot.session_id.clone(), snapshot))
            .collect();

        let focused = sessions
            .iter()
            .find(|session| session.effective_state().is_attention_needed())
            .or_else(|| sessions.iter().find(|session| session.effective_state().is_busy()))
            .or_else(|| sessions.first())
            .cloned();
        self.sessions = sessions;

        let new_state = focused
            .as_ref()
            .map(|session| session.effective_state())
            .unwrap_or(PetState::Idle);
        let new_message = match &focused {
            Some(session) if new_state != PetState::Idle => session.message.clone(),
            _ => String::new(),
        };

        let focused_id = focused.as_ref().map(|session| session.session_id.clone());
        let focus_changed = focused_id != self.previous_focused_session_id;
        let state_changed = new_state != self.previous_display_state;
        if state_changed || focus_changed {
            self.frame_index = 0;
            self.frame_accumulator_secs = 0.0;
        }
        self.previous_display_state = new_state;
        self.previous_focused_session_id = focused_id;
        self.focused = focused;
        self.display_state = new_state;
        self.display_message = new_message;
    }

    /// Advances the sprite animation by `dt` seconds at `fps` frames per second.
    /// Unlike Swift's 0.1 s tick (at most one frame per tick), the Rust overlay ticks
    /// every 0.3 s and may advance several frames at once so high-fps pets keep pace.
    pub fn advance_frames(&mut self, dt_secs: f64, fps: f64) {
        if !(fps > 0.0) || !dt_secs.is_finite() || dt_secs <= 0.0 {
            return;
        }
        self.frame_accumulator_secs += dt_secs;
        let frame_duration = 1.0 / fps;
        while self.frame_accumulator_secs >= frame_duration {
            self.frame_accumulator_secs -= frame_duration;
            self.frame_index = self.frame_index.wrapping_add(1);
        }
    }

    /// The user clicked the pet: pick a phrase (seeded externally so this stays pure).
    pub fn pet_clicked(&mut self, phrases: &[String], seed: u64, now_secs: f64) {
        if phrases.is_empty() {
            return;
        }
        let index = (seed % phrases.len() as u64) as usize;
        self.pet_reaction_message = Some(phrases[index].clone());
        self.pet_reaction_expires_at_secs = Some(now_secs + PET_REACTION_DURATION_SECS);
    }

    pub fn expire_pet_reaction_if_due(&mut self, now_secs: f64) {
        if let Some(expires_at) = self.pet_reaction_expires_at_secs {
            if now_secs >= expires_at {
                self.pet_reaction_expires_at_secs = None;
                self.pet_reaction_message = None;
            }
        }
    }

    /// Swift `isSpeechBubbleVisible`: a pet reaction always shows, even when bubbles are
    /// hidden; otherwise the bubble shows while there is a session message.
    pub fn is_speech_bubble_visible(&self, bubbles_hidden: bool) -> bool {
        if self.pet_reaction_message.is_some() {
            return true;
        }
        if bubbles_hidden {
            return false;
        }
        !self.display_message.is_empty()
    }

    /// Swift `speechBubbleText`: the reaction phrase wins over the session message.
    pub fn speech_text(&self) -> &str {
        self.pet_reaction_message.as_deref().unwrap_or(&self.display_message)
    }

    /// Swift `collapsedLabel`: "project", "project +N", or "no session".
    pub fn collapsed_label(&self) -> String {
        match &self.focused {
            None => "no session".to_string(),
            Some(focused) => {
                let extra = self.sessions.len().saturating_sub(1);
                if extra > 0 {
                    format!("{} +{extra}", focused.project_name())
                } else {
                    focused.project_name()
                }
            }
        }
    }

    /// Swift menu header: "<project>: <state label>" part (the caller prepends the pet name).
    pub fn menu_header(&self, pet_name: &str) -> String {
        match &self.focused {
            Some(session) => format!(
                "{pet_name} · {}: {}",
                session.project_name(),
                self.display_state.display_label()
            ),
            None => format!("{pet_name} · waiting for a Claude Code session"),
        }
    }

    /// Swift `usageSummary(for:)`: "ctx 62% left · 5h 71% left · 7d 30% left · $0.42".
    pub fn usage_summary(&self, session_id: Option<&str>) -> Option<String> {
        let usage = self.usage_by_session.get(session_id?)?;
        let mut parts: Vec<String> = Vec::new();
        if let Some(ctx) = usage.context_remaining_percentage() {
            parts.push(format!("ctx {}% left", ctx.round() as i64));
        }
        if let Some(five) = usage.five_hour_remaining_percentage() {
            parts.push(format!("5h {}% left", five.round() as i64));
        }
        if let Some(seven) = usage.seven_day_remaining_percentage() {
            parts.push(format!("7d {}% left", seven.round() as i64));
        }
        if let Some(cost) = usage.total_cost_usd {
            parts.push(format!("${cost:.2}"));
        }
        if parts.is_empty() {
            None
        } else {
            Some(parts.join(" · "))
        }
    }

    /// Remaining percentage for the focused session's gauge, or None when unknown.
    pub fn gauge_value(&self, metric: GaugeMetric) -> Option<f64> {
        let focused = self.focused.as_ref()?;
        metric.value(self.usage_by_session.get(&focused.session_id))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{now_epoch_secs, UsageSource};

    fn snapshot(id: &str, cwd: &str, state: PetState, age_secs: f64) -> SessionSnapshot {
        SessionSnapshot {
            session_id: id.to_string(),
            cwd: cwd.to_string(),
            state,
            message: format!("msg-{id}"),
            last_event_name: "test".to_string(),
            tool_name: None,
            updated_at_epoch_seconds: now_epoch_secs() - age_secs,
            pending_tool_use_id: None,
        }
    }

    fn usage(id: &str, ctx_used: Option<f64>, cost: Option<f64>) -> SessionUsageSnapshot {
        SessionUsageSnapshot {
            session_id: id.to_string(),
            source: UsageSource::StatusLine,
            updated_at_epoch_seconds: now_epoch_secs(),
            context_used_percentage: ctx_used,
            context_window_size: None,
            context_tokens: None,
            total_input_tokens: None,
            total_output_tokens: None,
            model_display_name: None,
            five_hour_used_percentage: None,
            five_hour_resets_at_epoch_seconds: None,
            seven_day_used_percentage: None,
            seven_day_resets_at_epoch_seconds: None,
            total_cost_usd: cost,
        }
    }

    #[test]
    fn focus_prefers_attention_then_busy_then_recent() {
        let mut model = OverlayModel::new();
        // Newest-first ordering like StateStore::load_all.
        model.reload(
            vec![
                snapshot("recent", "/w/recent", PetState::Idle, 1.0),
                snapshot("busy", "/w/busy", PetState::Working, 5.0),
                snapshot("waiting", "/w/waiting", PetState::WaitingApproval, 10.0),
            ],
            vec![],
        );
        assert_eq!(model.focused.as_ref().unwrap().session_id, "waiting");
        assert_eq!(model.display_state, PetState::WaitingApproval);
        assert_eq!(model.display_message, "msg-waiting");

        model.reload(
            vec![
                snapshot("recent", "/w/recent", PetState::Idle, 1.0),
                snapshot("busy", "/w/busy", PetState::Working, 5.0),
            ],
            vec![],
        );
        assert_eq!(model.focused.as_ref().unwrap().session_id, "busy");

        model.reload(vec![snapshot("recent", "/w/recent", PetState::Idle, 1.0)], vec![]);
        assert_eq!(model.focused.as_ref().unwrap().session_id, "recent");
        // Idle focused session shows no message.
        assert_eq!(model.display_message, "");
    }

    #[test]
    fn transient_decay_applies_before_focus_choice() {
        let mut model = OverlayModel::new();
        // A "done" snapshot older than its 6 s window has decayed to idle, so the
        // working session must win focus.
        model.reload(
            vec![
                snapshot("stale-done", "/w/done", PetState::Done, 60.0),
                snapshot("busy", "/w/busy", PetState::Working, 5.0),
            ],
            vec![],
        );
        assert_eq!(model.focused.as_ref().unwrap().session_id, "busy");
        assert_eq!(model.display_state, PetState::Working);
    }

    #[test]
    fn no_sessions_means_idle_no_message() {
        let mut model = OverlayModel::new();
        model.reload(vec![], vec![]);
        assert!(model.focused.is_none());
        assert_eq!(model.display_state, PetState::Idle);
        assert_eq!(model.display_message, "");
        assert_eq!(model.collapsed_label(), "no session");
        assert_eq!(model.menu_header("Airou"), "Airou · waiting for a Claude Code session");
    }

    #[test]
    fn state_or_focus_change_resets_frame_index() {
        let mut model = OverlayModel::new();
        model.reload(vec![snapshot("a", "/w/a", PetState::Working, 0.0)], vec![]);
        model.advance_frames(1.0, 3.0);
        assert_eq!(model.frame_index, 3);
        // Same state, same focus: frame index survives a reload.
        model.reload(vec![snapshot("a", "/w/a", PetState::Working, 0.0)], vec![]);
        assert_eq!(model.frame_index, 3);
        // State change resets.
        model.reload(vec![snapshot("a", "/w/a", PetState::Done, 0.0)], vec![]);
        assert_eq!(model.frame_index, 0);
        // Focus change resets too.
        model.advance_frames(1.0, 3.0);
        model.reload(vec![snapshot("b", "/w/b", PetState::Done, 0.0)], vec![]);
        assert_eq!(model.frame_index, 0);
    }

    #[test]
    fn advance_frames_accumulates_fractional_ticks() {
        let mut model = OverlayModel::new();
        // 3 fps -> one frame every 1/3 s; a 0.3 s tick advances 0 frames the first
        // tick, then catches up.
        model.advance_frames(0.3, 3.0);
        assert_eq!(model.frame_index, 0);
        model.advance_frames(0.3, 3.0);
        assert_eq!(model.frame_index, 1);
        // 12 fps over one 0.3 s tick advances multiple frames.
        let mut fast = OverlayModel::new();
        fast.advance_frames(0.3, 12.0);
        assert_eq!(fast.frame_index, 3);
        // Degenerate inputs are ignored.
        fast.advance_frames(f64::NAN, 12.0);
        fast.advance_frames(0.3, 0.0);
        assert_eq!(fast.frame_index, 3);
    }

    #[test]
    fn collapsed_label_counts_extras() {
        let mut model = OverlayModel::new();
        model.reload(
            vec![
                snapshot("a", "/work/alpha", PetState::Working, 0.0),
                snapshot("b", "/work/beta", PetState::Idle, 1.0),
                snapshot("c", "/work/gamma", PetState::Idle, 2.0),
            ],
            vec![],
        );
        assert_eq!(model.collapsed_label(), "alpha +2");
        model.reload(vec![snapshot("a", "/work/alpha", PetState::Working, 0.0)], vec![]);
        assert_eq!(model.collapsed_label(), "alpha");
        assert_eq!(model.menu_header("Airou"), "Airou · alpha: Working");
    }

    #[test]
    fn pet_reaction_overrides_message_and_expires() {
        let mut model = OverlayModel::new();
        model.reload(vec![snapshot("a", "/w/a", PetState::Working, 0.0)], vec![]);
        assert_eq!(model.speech_text(), "msg-a");
        assert!(model.is_speech_bubble_visible(false));
        assert!(!model.is_speech_bubble_visible(true));

        let phrases = vec!["purr".to_string(), "meow".to_string()];
        model.pet_clicked(&phrases, 3, 100.0); // 3 % 2 == 1 -> "meow"
        assert_eq!(model.speech_text(), "meow");
        // Reaction shows even when bubbles are hidden (Swift behaviour).
        assert!(model.is_speech_bubble_visible(true));

        model.expire_pet_reaction_if_due(100.0 + PET_REACTION_DURATION_SECS - 0.1);
        assert_eq!(model.speech_text(), "meow");
        model.expire_pet_reaction_if_due(100.0 + PET_REACTION_DURATION_SECS + 0.1);
        assert_eq!(model.speech_text(), "msg-a");
        assert!(model.pet_reaction_message.is_none());
    }

    #[test]
    fn usage_summary_matches_swift_format() {
        let mut model = OverlayModel::new();
        let mut full = usage("a", Some(38.4), Some(0.416));
        full.five_hour_used_percentage = Some(29.0);
        full.seven_day_used_percentage = Some(70.2);
        model.reload(
            vec![snapshot("a", "/w/a", PetState::Working, 0.0)],
            vec![full, usage("b", None, None)],
        );
        assert_eq!(
            model.usage_summary(Some("a")).unwrap(),
            "ctx 62% left · 5h 71% left · 7d 30% left · $0.42"
        );
        // No figures at all -> None.
        assert_eq!(model.usage_summary(Some("b")), None);
        assert_eq!(model.usage_summary(None), None);
        assert_eq!(model.usage_summary(Some("missing")), None);
    }

    #[test]
    fn gauge_value_uses_focused_session() {
        let mut model = OverlayModel::new();
        model.reload(
            vec![snapshot("a", "/w/a", PetState::Working, 0.0)],
            vec![usage("a", Some(25.0), None)],
        );
        assert_eq!(model.gauge_value(GaugeMetric::ContextRemaining), Some(75.0));
        assert_eq!(model.gauge_value(GaugeMetric::FiveHourRemaining), None);
        assert_eq!(model.gauge_value(GaugeMetric::Off), None);
    }
}
