//! Pure overlay state logic: which session each pet represents, what it says, which
//! animation frame is showing, how the row of session cards is laid out and what a click
//! does. Port of the Swift original's `UI/PetViewModel.swift` (minus SwiftUI plumbing);
//! nothing here touches a window, so it is unit-tested without one.

use super::row_layout::{GridSize, RowCard, RowLayout};
use crate::model::{GaugeMetric, PetState, SessionSnapshot, SessionUsageSnapshot};
use std::collections::HashMap;

/// Seconds a pet-click reaction phrase stays on screen (Swift: `petReactionDurationSeconds`).
pub const PET_REACTION_DURATION_SECS: f64 = 2.5;
/// How long the side cards take to fold back into the primary before the row collapses
/// (Swift: `collapseAnimationSeconds`).
pub const COLLAPSE_ANIMATION_SECS: f64 = 0.22;

/// What the layout needs besides the sessions: pet grid, size and gauge setting.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LayoutInputs {
    pub grid: GridSize,
    pub pixel_scale: f32,
    pub shows_gauge: bool,
}

/// What a click on the content did (Swift logs the same words to overlay.log).
#[derive(Debug, Clone, PartialEq)]
pub enum ClickAction {
    /// The pet was petted (reaction phrase + heart).
    Pet,
    /// The row fanned out.
    Expand,
    /// The side cards started folding back (collapse in `COLLAPSE_ANIMATION_SECS`).
    Collapse,
    /// A side session was pinned as the primary.
    Pin(String),
    /// Nothing happened (a collapse animation is running).
    Ignored,
}

/// Result of a relayout: whether the panel must be resized/moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutChange {
    Unchanged,
    /// Same panel geometry (size and primary position); only the content changed.
    ContentOnly,
    /// The panel must be resized so the primary pet stays where it is on screen.
    PanelGeometry,
}

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

    /// Fan-out: show every session side by side. Toggled by clicking the pet; forced by the menu.
    pub is_fanned_out: bool,
    pub is_always_fanned_out: bool,
    /// User-chosen focus (click a side pet / pick from the menu). Overrides the automatic rule.
    pub pinned_session_id: Option<String>,
    /// True while side cards fold back into the primary before the row actually collapses.
    pub is_collapsing: bool,
    collapse_finishes_at_secs: Option<f64>,

    /// Current geometry; the panel resizes from this.
    pub layout: RowLayout,
    /// The layout before the last change (cards animate from their old screen position).
    pub previous_layout: Option<RowLayout>,
    /// Bumped on every layout change so cards can run their entrance / move animation.
    pub layout_generation: u64,
    /// When `layout_generation` last changed (epoch seconds), for the card motion.
    pub layout_changed_at_secs: f64,
    /// How far the panel moved (screen x, points) to keep the primary pet still during the
    /// last layout change; set by the window right after resizing, read by the painter.
    pub panel_shift_x: f32,

    /// Animation triggers: epoch seconds when the effect last started (`None` = never).
    pub done_bounce_started_at_secs: Option<f64>,
    pub error_shake_started_at_secs: Option<f64>,
    pub pet_reaction_started_at_secs: Option<f64>,
    /// When the speech bubble last became visible (for its fade/scale entrance).
    pub bubble_shown_at_secs: Option<f64>,
    was_bubble_visible: bool,
}

impl OverlayModel {
    pub fn new(inputs: LayoutInputs) -> OverlayModel {
        let layout = RowLayout::make(
            inputs.grid,
            inputs.pixel_scale,
            &["no session".to_string()],
            &[None],
            0,
            0.0,
            inputs.shows_gauge,
        );
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
            is_fanned_out: false,
            is_always_fanned_out: false,
            pinned_session_id: None,
            is_collapsing: false,
            collapse_finishes_at_secs: None,
            layout,
            previous_layout: None,
            layout_generation: 0,
            layout_changed_at_secs: 0.0,
            panel_shift_x: 0.0,
            done_bounce_started_at_secs: None,
            error_shake_started_at_secs: None,
            pet_reaction_started_at_secs: None,
            bubble_shown_at_secs: None,
            was_bubble_visible: false,
        }
    }

    // MARK: - Session selection

    /// Ingests a fresh `StateStore::load_all()` result (already newest-first) and the
    /// usage snapshots. Mirrors `PetViewModel.reloadSessions()`: pinned session wins, then
    /// attention-needed > busy > most recently updated; the message shows only while the
    /// effective state is not idle; a state or focus change restarts the animation and
    /// fires the done-bounce / error-shake triggers. Returns true when anything the
    /// painter shows changed.
    pub fn reload(&mut self, sessions: Vec<SessionSnapshot>, usage: Vec<SessionUsageSnapshot>, now_secs: f64) -> bool {
        let mut changed = false;
        // Sessions that were only opened (resumed by clicking through the desktop app's
        // list, or an MCP server that never spoke) show their hello wave and then step out
        // of the row until they do something.
        let sessions: Vec<SessionSnapshot> = sessions
            .into_iter()
            .filter(|session| !session.is_opened_without_activity())
            .collect();
        let usage_by_session: HashMap<String, SessionUsageSnapshot> = usage
            .into_iter()
            .map(|snapshot| (snapshot.session_id.clone(), snapshot))
            .collect();
        if usage_by_session != self.usage_by_session {
            self.usage_by_session = usage_by_session;
            changed = true;
        }

        if let Some(pinned) = &self.pinned_session_id {
            if !sessions.iter().any(|session| &session.session_id == pinned) {
                self.pinned_session_id = None; // the pinned session ended
                changed = true;
            }
        }
        if sessions.len() < 2 && self.is_fanned_out {
            self.is_fanned_out = false; // nothing left to fan out
            self.is_collapsing = false;
            self.collapse_finishes_at_secs = None;
            changed = true;
        }

        let focused = sessions
            .iter()
            .find(|session| Some(&session.session_id) == self.pinned_session_id.as_ref())
            .or_else(|| sessions.iter().find(|session| session.effective_state().is_attention_needed()))
            .or_else(|| sessions.iter().find(|session| session.effective_state().is_busy()))
            .or_else(|| sessions.first())
            .cloned();
        if sessions != self.sessions {
            self.sessions = sessions;
            changed = true;
        }

        let new_state = focused
            .as_ref()
            .map(|session| session.effective_state())
            .unwrap_or(PetState::Idle);
        let new_message = match &focused {
            Some(session)
                if matches!(new_state, PetState::Done | PetState::Error)
                    && session.age_secs() > PetState::RESULT_BUBBLE_LINGER_SECS =>
            {
                String::new() // the result badge stays; the bubble steps aside after a while
            }
            Some(session) if new_state != PetState::Idle => session.message.clone(),
            _ => String::new(),
        };

        let focused_id = focused.as_ref().map(|session| session.session_id.clone());
        let focus_changed = focused_id != self.previous_focused_session_id;
        let state_changed = new_state != self.previous_display_state;
        if state_changed || focus_changed {
            self.frame_index = 0;
            self.frame_accumulator_secs = 0.0;
            if matches!(new_state, PetState::Done | PetState::Hello) {
                self.done_bounce_started_at_secs = Some(now_secs);
            }
            if new_state == PetState::Error {
                self.error_shake_started_at_secs = Some(now_secs);
            }
            changed = true;
        }
        self.previous_display_state = new_state;
        self.previous_focused_session_id = focused_id;
        if focused != self.focused {
            self.focused = focused;
            changed = true;
        }
        if new_state != self.display_state {
            self.display_state = new_state;
            changed = true;
        }
        if new_message != self.display_message {
            self.display_message = new_message;
            changed = true;
        }
        changed
    }

    /// Advances the sprite animation by `dt` seconds at `fps` frames per second; returns
    /// true when the frame index moved. May advance several frames per call so high-fps
    /// pets keep pace whatever the tick length.
    pub fn advance_frames(&mut self, dt_secs: f64, fps: f64) -> bool {
        if !(fps > 0.0) || !dt_secs.is_finite() || dt_secs <= 0.0 {
            return false;
        }
        self.frame_accumulator_secs += dt_secs;
        let frame_duration = 1.0 / fps;
        let mut advanced = false;
        while self.frame_accumulator_secs >= frame_duration {
            self.frame_accumulator_secs -= frame_duration;
            self.frame_index = self.frame_index.wrapping_add(1);
            advanced = true;
        }
        advanced
    }

    // MARK: - Derived

    /// Swift `isExpanded`: fanned out (by click or always) and at least two sessions.
    pub fn is_expanded(&self) -> bool {
        (self.is_fanned_out || self.is_always_fanned_out) && self.sessions.len() >= 2
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

    /// True when a session other than the focused one is waiting on the user (shown as a
    /// red dot on the collapsed badge).
    pub fn has_hidden_attention(&self) -> bool {
        let focused_id = self.focused.as_ref().map(|session| session.session_id.as_str());
        self.sessions
            .iter()
            .any(|session| Some(session.session_id.as_str()) != focused_id && session.effective_state().is_attention_needed())
    }

    pub fn session_with_id(&self, id: Option<&str>) -> Option<&SessionSnapshot> {
        let id = id?;
        self.sessions.iter().find(|session| session.session_id == id)
    }

    /// The state to draw for a given card (transient decay applied).
    pub fn state_for_card(&self, card: &RowCard) -> PetState {
        if card.is_primary {
            return self.display_state;
        }
        self.session_with_id(card.session_id.as_deref())
            .map(|session| session.effective_state())
            .unwrap_or(PetState::Idle)
    }

    /// Remaining percentage for a card's gauge, or None when unknown.
    pub fn gauge_value_for_card(&self, card: &RowCard, metric: GaugeMetric) -> Option<f64> {
        let usage = card.session_id.as_ref().and_then(|id| self.usage_by_session.get(id));
        metric.value(usage)
    }

    /// Swift menu header: "<pet> · <project>: <state label>" or the waiting line.
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

    // MARK: - Layout

    /// Row order when expanded: primary in the middle, the others alternating right/left by recency.
    pub fn expanded_row(&self) -> Vec<&SessionSnapshot> {
        let Some(focused) = &self.focused else { return Vec::new() };
        let mut left: Vec<&SessionSnapshot> = Vec::new();
        let mut right: Vec<&SessionSnapshot> = Vec::new();
        for (index, session) in self
            .sessions
            .iter()
            .filter(|session| session.session_id != focused.session_id)
            .enumerate()
        {
            if index % 2 == 0 {
                right.push(session);
            } else {
                left.push(session);
            }
        }
        let mut row: Vec<&SessionSnapshot> = left.into_iter().rev().collect();
        // The focused snapshot lives in `sessions` too; point at that copy.
        if let Some(focused_in_list) = self.sessions.iter().find(|session| session.session_id == focused.session_id) {
            row.push(focused_in_list);
        }
        row.extend(right);
        row
    }

    /// Geometry of the collapsed (single-card) row for the current pet and scale — used to
    /// store a stable window position regardless of the current fan-out state.
    pub fn collapsed_layout(&self, inputs: LayoutInputs, speech_bubble_width: f32) -> RowLayout {
        RowLayout::make(
            inputs.grid,
            inputs.pixel_scale,
            &[self.collapsed_label()],
            &[self.focused.as_ref().map(|session| session.session_id.clone())],
            0,
            speech_bubble_width,
            inputs.shows_gauge,
        )
    }

    /// Recomputes the layout (Swift `relayoutIfNeeded`). `speech_bubble_width` is the
    /// measured width of the visible bubble (0 when hidden).
    pub fn relayout(&mut self, inputs: LayoutInputs, speech_bubble_width: f32, now_secs: f64) -> LayoutChange {
        let new_layout = if self.is_expanded() {
            let row = self.expanded_row();
            let focused_id = self.focused.as_ref().map(|session| session.session_id.as_str());
            let primary_index = row
                .iter()
                .position(|session| Some(session.session_id.as_str()) == focused_id)
                .unwrap_or(0);
            let labels: Vec<String> = row.iter().map(|session| session.project_name()).collect();
            let session_ids: Vec<Option<String>> = row.iter().map(|session| Some(session.session_id.clone())).collect();
            RowLayout::make(
                inputs.grid,
                inputs.pixel_scale,
                &labels,
                &session_ids,
                primary_index,
                speech_bubble_width,
                inputs.shows_gauge,
            )
        } else {
            self.collapsed_layout(inputs, speech_bubble_width)
        };
        if new_layout == self.layout {
            return LayoutChange::Unchanged;
        }
        let needs_panel_update = new_layout.content_width != self.layout.content_width
            || new_layout.content_height != self.layout.content_height
            || new_layout.primary_center_x() != self.layout.primary_center_x();
        self.previous_layout = Some(std::mem::replace(&mut self.layout, new_layout));
        self.panel_shift_x = 0.0;
        self.layout_generation = self.layout_generation.wrapping_add(1);
        self.layout_changed_at_secs = now_secs;
        if needs_panel_update {
            LayoutChange::PanelGeometry
        } else {
            LayoutChange::ContentOnly
        }
    }

    /// Tracks bubble visibility transitions so the painter can play the entrance.
    pub fn note_bubble_visibility(&mut self, is_visible: bool, now_secs: f64) {
        if is_visible && !self.was_bubble_visible {
            self.bubble_shown_at_secs = Some(now_secs);
        }
        self.was_bubble_visible = is_visible;
    }

    // MARK: - User interaction

    /// A click at `content_x` (points from the panel's left edge). Port of
    /// `PetViewModel.handleClick(atContentX:)`; the phrase seed keeps this pure.
    pub fn handle_click(&mut self, content_x: f32, phrases: &[String], seed: u64, now_secs: f64) -> ClickAction {
        if self.sessions.len() < 2 {
            self.pet_clicked(phrases, seed, now_secs);
            return ClickAction::Pet;
        }
        if self.is_collapsing {
            return ClickAction::Ignored;
        }
        if !self.is_expanded() {
            self.is_fanned_out = true;
            return ClickAction::Expand;
        }
        let Some(card) = self.layout.card_at_content_x(content_x).cloned() else {
            self.collapse(now_secs);
            return ClickAction::Collapse;
        };
        if card.is_primary {
            if self.is_always_fanned_out {
                self.pet_clicked(phrases, seed, now_secs); // can't collapse; treat as petting
                ClickAction::Pet
            } else {
                self.collapse(now_secs);
                ClickAction::Collapse
            }
        } else if let Some(session_id) = card.session_id {
            self.pin(Some(session_id.clone()));
            ClickAction::Pin(session_id)
        } else {
            ClickAction::Ignored
        }
    }

    /// One-line description of the cards for the click log (Swift logs the same shape).
    pub fn cards_description(&self) -> String {
        self.layout
            .cards
            .iter()
            .map(|card| {
                format!(
                    "{}[{}-{}]{}",
                    card.label,
                    card.x as i64,
                    (card.x + card.width) as i64,
                    if card.is_primary { "*" } else { "" }
                )
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// Folds the side cards back into the primary, then collapses the row
    /// (`finish_collapse_if_due` completes it after `COLLAPSE_ANIMATION_SECS`).
    pub fn collapse(&mut self, now_secs: f64) {
        if !self.is_fanned_out || self.is_collapsing {
            return;
        }
        self.is_collapsing = true;
        self.collapse_finishes_at_secs = Some(now_secs + COLLAPSE_ANIMATION_SECS);
    }

    /// Returns true when the collapse just completed (the caller must relayout).
    pub fn finish_collapse_if_due(&mut self, now_secs: f64) -> bool {
        match self.collapse_finishes_at_secs {
            Some(finishes_at) if now_secs >= finishes_at => {
                self.collapse_finishes_at_secs = None;
                self.is_collapsing = false;
                self.is_fanned_out = false;
                true
            }
            _ => false,
        }
    }

    /// Elapsed seconds of the running collapse animation, if any.
    pub fn collapse_elapsed_secs(&self, now_secs: f64) -> Option<f64> {
        let finishes_at = self.collapse_finishes_at_secs?;
        Some((now_secs - (finishes_at - COLLAPSE_ANIMATION_SECS)).max(0.0))
    }

    /// Make a session the primary one, overriding the automatic focus rule. `None`
    /// restores automatic (Swift `pin(sessionId:)` — reloads the current sessions).
    pub fn pin(&mut self, session_id: Option<String>) {
        self.pinned_session_id = session_id;
        let sessions = self.sessions.clone();
        let usage: Vec<SessionUsageSnapshot> = self.usage_by_session.values().cloned().collect();
        self.reload(sessions, usage, crate::model::now_epoch_secs());
    }

    /// The user clicked the pet itself: react with a phrase (does not touch session state).
    pub fn pet_clicked(&mut self, phrases: &[String], seed: u64, now_secs: f64) {
        if !phrases.is_empty() {
            let index = (seed % phrases.len() as u64) as usize;
            self.pet_reaction_message = Some(phrases[index].clone());
            self.pet_reaction_expires_at_secs = Some(now_secs + PET_REACTION_DURATION_SECS);
        }
        self.pet_reaction_started_at_secs = Some(now_secs);
    }

    /// Returns true when a reaction just expired (the caller must relayout).
    pub fn expire_pet_reaction_if_due(&mut self, now_secs: f64) -> bool {
        if let Some(expires_at) = self.pet_reaction_expires_at_secs {
            if now_secs >= expires_at {
                self.pet_reaction_expires_at_secs = None;
                self.pet_reaction_message = None;
                return true;
            }
        }
        false
    }

    /// True while any timed effect may still be moving (drives the 60 Hz redraw).
    pub fn is_animating(&self, now_secs: f64) -> bool {
        use super::animation::*;
        let running = |started_at: Option<f64>, duration: f32| {
            started_at.is_some_and(|start| (now_secs - start) < duration as f64)
        };
        self.is_collapsing
            || running(self.done_bounce_started_at_secs, DONE_BOUNCE_PHASE_SECS * (DONE_BOUNCE_PHASES.len() - 1) as f32)
            || running(self.error_shake_started_at_secs, ERROR_SHAKE_PHASE_SECS * (ERROR_SHAKE_PHASES.len() - 1) as f32)
            || running(self.pet_reaction_started_at_secs, HEART_DURATION_SECS)
            || running(self.bubble_shown_at_secs, BUBBLE_APPEAR_DURATION_SECS)
            || (self.previous_layout.is_some() && running(Some(self.layout_changed_at_secs), CARD_SETTLE_DURATION_SECS * 2.0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{now_epoch_secs, UsageSource};

    const INPUTS: LayoutInputs = LayoutInputs {
        grid: GridSize { width: 28, height: 28 },
        pixel_scale: 5.0,
        shows_gauge: false,
    };

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

    fn three_sessions() -> Vec<SessionSnapshot> {
        // Newest-first ordering like StateStore::load_all.
        vec![
            snapshot("recent", "/w/recent", PetState::Idle, 1.0),
            snapshot("busy", "/w/busy", PetState::Working, 5.0),
            snapshot("waiting", "/w/waiting", PetState::WaitingApproval, 10.0),
        ]
    }

    fn model() -> OverlayModel {
        OverlayModel::new(INPUTS)
    }

    #[test]
    fn done_bubble_steps_aside_after_the_linger_but_the_state_stays() {
        let mut model = model();
        model.reload(vec![snapshot("s", "/w/s", PetState::Done, 5.0)], vec![], 0.0);
        assert_eq!(model.display_state, PetState::Done);
        assert_eq!(model.speech_text(), "msg-s");
        model.reload(
            vec![snapshot("s", "/w/s", PetState::Done, PetState::RESULT_BUBBLE_LINGER_SECS + 1.0)],
            vec![],
            0.0,
        );
        assert_eq!(model.display_state, PetState::Done, "badge/label icon keep showing the result");
        assert!(!model.is_speech_bubble_visible(false));
    }

    #[test]
    fn reload_hides_sessions_that_were_only_opened() {
        let mut model = model();
        let mut resumed_long_ago = snapshot("resumed", "/w/resumed", PetState::Hello, 60.0);
        resumed_long_ago.last_event_name = "SessionStart".into();
        let mut resumed_just_now = snapshot("fresh", "/w/fresh", PetState::Hello, 0.0);
        resumed_just_now.last_event_name = "SessionStart".into();
        let working = snapshot("busy", "/w/busy", PetState::Working, 5.0);
        model.reload(vec![resumed_just_now, working, resumed_long_ago], vec![], 0.0);
        let ids: Vec<&str> = model.sessions.iter().map(|session| session.session_id.as_str()).collect();
        assert_eq!(ids, vec!["fresh", "busy"], "the decayed hello-only session is filtered out");
        assert_eq!(model.collapsed_label(), "busy +1");
    }

    #[test]
    fn focus_prefers_attention_then_busy_then_recent() {
        let mut model = model();
        model.reload(three_sessions(), vec![], 0.0);
        assert_eq!(model.focused.as_ref().unwrap().session_id, "waiting");
        assert_eq!(model.display_state, PetState::WaitingApproval);
        assert_eq!(model.display_message, "msg-waiting");

        model.reload(
            vec![
                snapshot("recent", "/w/recent", PetState::Idle, 1.0),
                snapshot("busy", "/w/busy", PetState::Working, 5.0),
            ],
            vec![],
            0.0,
        );
        assert_eq!(model.focused.as_ref().unwrap().session_id, "busy");

        model.reload(vec![snapshot("recent", "/w/recent", PetState::Idle, 1.0)], vec![], 0.0);
        assert_eq!(model.focused.as_ref().unwrap().session_id, "recent");
        // Idle focused session shows no message.
        assert_eq!(model.display_message, "");
    }

    #[test]
    fn pinned_session_wins_until_it_ends() {
        let mut model = model();
        model.reload(three_sessions(), vec![], 0.0);
        model.pin(Some("recent".to_string()));
        assert_eq!(model.focused.as_ref().unwrap().session_id, "recent");
        assert_eq!(model.display_state, PetState::Idle);
        // Reload keeps the pin.
        model.reload(three_sessions(), vec![], 1.0);
        assert_eq!(model.focused.as_ref().unwrap().session_id, "recent");
        // The pinned session ends -> back to automatic.
        model.reload(
            vec![
                snapshot("busy", "/w/busy", PetState::Working, 5.0),
                snapshot("waiting", "/w/waiting", PetState::WaitingApproval, 10.0),
            ],
            vec![],
            2.0,
        );
        assert_eq!(model.pinned_session_id, None);
        assert_eq!(model.focused.as_ref().unwrap().session_id, "waiting");
        // Explicit un-pin.
        model.pin(Some("busy".to_string()));
        assert_eq!(model.focused.as_ref().unwrap().session_id, "busy");
        model.pin(None);
        assert_eq!(model.focused.as_ref().unwrap().session_id, "waiting");
    }

    #[test]
    fn transient_decay_applies_before_focus_choice() {
        let mut model = model();
        model.reload(
            vec![
                snapshot("stale-done", "/w/done", PetState::Done, 60.0),
                snapshot("busy", "/w/busy", PetState::Working, 5.0),
            ],
            vec![],
            0.0,
        );
        assert_eq!(model.focused.as_ref().unwrap().session_id, "busy");
        assert_eq!(model.display_state, PetState::Working);
    }

    #[test]
    fn no_sessions_means_idle_no_message() {
        let mut model = model();
        assert!(!model.reload(vec![], vec![], 0.0), "nothing changed from the initial state");
        assert!(model.focused.is_none());
        assert_eq!(model.display_state, PetState::Idle);
        assert_eq!(model.display_message, "");
        assert_eq!(model.collapsed_label(), "no session");
        assert_eq!(model.menu_header("Airou"), "Airou · waiting for a Claude Code session");
        assert_eq!(model.layout.cards[0].label, "no session");
        assert_eq!(model.layout.cards[0].session_id, None);
    }

    #[test]
    fn state_or_focus_change_resets_frame_index_and_fires_triggers() {
        let mut model = model();
        model.reload(vec![snapshot("a", "/w/a", PetState::Working, 0.0)], vec![], 0.0);
        assert!(model.advance_frames(1.0, 3.0));
        assert_eq!(model.frame_index, 3);
        // Same state, same focus: frame index survives a reload.
        model.reload(vec![snapshot("a", "/w/a", PetState::Working, 0.0)], vec![], 0.0);
        assert_eq!(model.frame_index, 3);
        // State change resets and fires the done bounce.
        model.reload(vec![snapshot("a", "/w/a", PetState::Done, 0.0)], vec![], 10.0);
        assert_eq!(model.frame_index, 0);
        assert_eq!(model.done_bounce_started_at_secs, Some(10.0));
        assert_eq!(model.error_shake_started_at_secs, None);
        // Focus change resets too; error fires the shake.
        model.advance_frames(1.0, 3.0);
        model.reload(vec![snapshot("b", "/w/b", PetState::Error, 0.0)], vec![], 20.0);
        assert_eq!(model.frame_index, 0);
        assert_eq!(model.error_shake_started_at_secs, Some(20.0));
        // Hello bounces like done.
        model.reload(vec![snapshot("b", "/w/b", PetState::Hello, 0.0)], vec![], 30.0);
        assert_eq!(model.done_bounce_started_at_secs, Some(30.0));
        assert!(model.is_animating(30.1));
        assert!(!model.is_animating(31.0));
    }

    #[test]
    fn advance_frames_accumulates_fractional_ticks() {
        let mut model = model();
        assert!(!model.advance_frames(0.3, 3.0));
        assert_eq!(model.frame_index, 0);
        assert!(model.advance_frames(0.3, 3.0));
        assert_eq!(model.frame_index, 1);
        let mut fast = OverlayModel::new(INPUTS);
        fast.advance_frames(0.3, 12.0);
        assert_eq!(fast.frame_index, 3);
        assert!(!fast.advance_frames(f64::NAN, 12.0));
        assert!(!fast.advance_frames(0.3, 0.0));
        assert_eq!(fast.frame_index, 3);
    }

    #[test]
    fn collapsed_label_counts_extras_and_hidden_attention() {
        let mut model = model();
        model.reload(
            vec![
                snapshot("a", "/work/alpha", PetState::Working, 0.0),
                snapshot("b", "/work/beta", PetState::Idle, 1.0),
                snapshot("c", "/work/gamma", PetState::Idle, 2.0),
            ],
            vec![],
            0.0,
        );
        assert_eq!(model.collapsed_label(), "alpha +2");
        assert!(!model.has_hidden_attention());
        // Pin "b"; make "c" wait for approval: attention hidden behind the pinned primary.
        model.pin(Some("b".to_string()));
        model.reload(
            vec![
                snapshot("a", "/work/alpha", PetState::Working, 0.0),
                snapshot("b", "/work/beta", PetState::Idle, 1.0),
                snapshot("c", "/work/gamma", PetState::WaitingApproval, 2.0),
            ],
            vec![],
            0.0,
        );
        assert_eq!(model.focused.as_ref().unwrap().session_id, "b");
        assert!(model.has_hidden_attention());
        model.reload(vec![snapshot("a", "/work/alpha", PetState::Working, 0.0)], vec![], 0.0);
        assert_eq!(model.collapsed_label(), "alpha");
        assert_eq!(model.menu_header("Airou"), "Airou · alpha: Working");
    }

    #[test]
    fn pet_reaction_overrides_message_and_expires() {
        let mut model = model();
        model.reload(vec![snapshot("a", "/w/a", PetState::Working, 0.0)], vec![], 0.0);
        assert_eq!(model.speech_text(), "msg-a");
        assert!(model.is_speech_bubble_visible(false));
        assert!(!model.is_speech_bubble_visible(true));

        let phrases = vec!["purr".to_string(), "meow".to_string()];
        model.pet_clicked(&phrases, 3, 100.0); // 3 % 2 == 1 -> "meow"
        assert_eq!(model.speech_text(), "meow");
        assert!(model.is_speech_bubble_visible(true));
        assert_eq!(model.pet_reaction_started_at_secs, Some(100.0));

        assert!(!model.expire_pet_reaction_if_due(100.0 + PET_REACTION_DURATION_SECS - 0.1));
        assert_eq!(model.speech_text(), "meow");
        assert!(model.expire_pet_reaction_if_due(100.0 + PET_REACTION_DURATION_SECS + 0.1));
        assert_eq!(model.speech_text(), "msg-a");
        assert!(model.pet_reaction_message.is_none());
        // A pet with no phrases still gets the heart.
        model.pet_clicked(&[], 0, 200.0);
        assert_eq!(model.pet_reaction_message, None);
        assert_eq!(model.pet_reaction_started_at_secs, Some(200.0));
    }

    #[test]
    fn usage_summary_matches_swift_format() {
        let mut model = model();
        let mut full = usage("a", Some(38.4), Some(0.416));
        full.five_hour_used_percentage = Some(29.0);
        full.seven_day_used_percentage = Some(70.2);
        model.reload(
            vec![snapshot("a", "/w/a", PetState::Working, 0.0)],
            vec![full, usage("b", None, None)],
            0.0,
        );
        assert_eq!(
            model.usage_summary(Some("a")).unwrap(),
            "ctx 62% left · 5h 71% left · 7d 30% left · $0.42"
        );
        assert_eq!(model.usage_summary(Some("b")), None);
        assert_eq!(model.usage_summary(None), None);
        assert_eq!(model.usage_summary(Some("missing")), None);
    }

    #[test]
    fn gauge_value_per_card_and_state_per_card() {
        let mut model = model();
        model.reload(three_sessions(), vec![usage("busy", Some(25.0), None)], 0.0);
        model.is_fanned_out = true;
        assert_eq!(model.relayout(INPUTS, 0.0, 0.0), LayoutChange::PanelGeometry);
        let busy_card = model.layout.cards.iter().find(|card| card.id() == "busy").unwrap().clone();
        let waiting_card = model.layout.cards.iter().find(|card| card.id() == "waiting").unwrap().clone();
        assert!(waiting_card.is_primary);
        assert_eq!(model.gauge_value_for_card(&busy_card, GaugeMetric::ContextRemaining), Some(75.0));
        assert_eq!(model.gauge_value_for_card(&waiting_card, GaugeMetric::ContextRemaining), None);
        assert_eq!(model.gauge_value_for_card(&busy_card, GaugeMetric::Off), None);
        assert_eq!(model.state_for_card(&busy_card), PetState::Working);
        assert_eq!(model.state_for_card(&waiting_card), PetState::WaitingApproval);
    }

    #[test]
    fn expanded_row_puts_primary_in_the_middle_alternating_right_then_left() {
        let mut model = model();
        model.reload(
            vec![
                snapshot("s1", "/w/s1", PetState::Idle, 1.0),
                snapshot("s2", "/w/s2", PetState::Idle, 2.0),
                snapshot("s3", "/w/s3", PetState::Idle, 3.0),
                snapshot("s4", "/w/s4", PetState::Idle, 4.0),
                snapshot("focus", "/w/focus", PetState::WaitingApproval, 5.0),
            ],
            vec![],
            0.0,
        );
        let ids: Vec<&str> = model.expanded_row().iter().map(|session| session.session_id.as_str()).collect();
        // others by recency: s1 -> right, s2 -> left, s3 -> right, s4 -> left; left reversed.
        assert_eq!(ids, vec!["s4", "s2", "focus", "s1", "s3"]);
        // Not expanded until fanned out.
        assert!(!model.is_expanded());
        model.is_always_fanned_out = true;
        assert!(model.is_expanded());
        assert_eq!(model.relayout(INPUTS, 0.0, 0.0), LayoutChange::PanelGeometry);
        assert_eq!(model.layout.cards.len(), 5);
        assert!(model.layout.cards[2].is_primary);
        assert_eq!(model.layout.cards[2].label, "focus");
        assert_eq!(model.layout_generation, 1);
        assert!(model.previous_layout.is_some());
        // Same inputs again -> unchanged.
        assert_eq!(model.relayout(INPUTS, 0.0, 0.0), LayoutChange::Unchanged);
        assert_eq!(model.layout_generation, 1);
        // A bubble that fits inside the existing width only changes the content.
        assert_eq!(model.relayout(INPUTS, 100.0, 0.0), LayoutChange::ContentOnly);
    }

    #[test]
    fn expanded_row_with_a_single_session_and_no_focus() {
        let mut model = model();
        assert!(model.expanded_row().is_empty());
        model.reload(vec![snapshot("only", "/w/only", PetState::Idle, 0.0)], vec![], 0.0);
        let ids: Vec<&str> = model.expanded_row().iter().map(|session| session.session_id.as_str()).collect();
        assert_eq!(ids, vec!["only"]);
    }

    #[test]
    fn click_behaviour_matches_swift() {
        let phrases = vec!["hi".to_string()];
        let mut model = model();
        // One session: any click pets.
        model.reload(vec![snapshot("only", "/w/only", PetState::Idle, 0.0)], vec![], 0.0);
        assert_eq!(model.handle_click(10.0, &phrases, 0, 0.0), ClickAction::Pet);
        assert_eq!(model.pet_reaction_message.as_deref(), Some("hi"));

        // Three sessions collapsed: click expands.
        model.reload(three_sessions(), vec![], 0.0);
        model.pet_reaction_message = None;
        assert_eq!(model.handle_click(110.0, &phrases, 0, 0.0), ClickAction::Expand);
        assert!(model.is_fanned_out);
        model.relayout(INPUTS, 0.0, 0.0);
        assert_eq!(model.layout.cards.len(), 3);

        // Click on a side card pins it.
        let side = model.layout.cards.iter().find(|card| !card.is_primary).unwrap().clone();
        assert_eq!(model.handle_click(side.center_x(), &phrases, 0, 1.0), ClickAction::Pin(side.id().to_string()));
        assert_eq!(model.pinned_session_id.as_deref(), Some(side.id()));
        assert_eq!(model.focused.as_ref().unwrap().session_id, side.id());
        model.relayout(INPUTS, 0.0, 1.0);
        assert!(model.layout.cards[1].is_primary);
        assert_eq!(model.layout.cards[1].id(), side.id());

        // Click in a gap collapses (with the fold animation first).
        let gap_x = model.layout.cards[0].x + model.layout.cards[0].width + 2.0;
        assert_eq!(model.handle_click(gap_x, &phrases, 0, 2.0), ClickAction::Collapse);
        assert!(model.is_collapsing);
        assert!(model.is_fanned_out, "still fanned out while folding");
        assert!((model.collapse_elapsed_secs(2.1).unwrap() - 0.1).abs() < 1e-9);
        assert_eq!(model.handle_click(gap_x, &phrases, 0, 2.1), ClickAction::Ignored);
        assert!(!model.finish_collapse_if_due(2.0 + COLLAPSE_ANIMATION_SECS - 0.05));
        assert!(model.finish_collapse_if_due(2.0 + COLLAPSE_ANIMATION_SECS + 0.01));
        assert!(!model.is_fanned_out && !model.is_collapsing);
        model.relayout(INPUTS, 0.0, 3.0);
        assert_eq!(model.layout.cards.len(), 1);

        // Expanded: clicking the primary collapses…
        assert_eq!(model.handle_click(110.0, &phrases, 0, 4.0), ClickAction::Expand);
        model.relayout(INPUTS, 0.0, 4.0);
        assert_eq!(model.handle_click(model.layout.primary_center_x(), &phrases, 0, 5.0), ClickAction::Collapse);
        model.finish_collapse_if_due(10.0);
        model.relayout(INPUTS, 0.0, 10.0);
        // …unless the row is always fanned out, then it pets.
        model.is_always_fanned_out = true;
        model.relayout(INPUTS, 0.0, 11.0);
        assert_eq!(model.handle_click(model.layout.primary_center_x(), &phrases, 0, 12.0), ClickAction::Pet);
        assert!(model.pet_reaction_message.is_some());
    }

    #[test]
    fn dropping_below_two_sessions_leaves_fan_out() {
        let mut model = model();
        model.reload(three_sessions(), vec![], 0.0);
        model.is_fanned_out = true;
        model.collapse(0.0);
        assert!(model.is_collapsing);
        model.reload(vec![snapshot("only", "/w/only", PetState::Idle, 0.0)], vec![], 1.0);
        assert!(!model.is_fanned_out);
        assert!(!model.is_collapsing);
        assert_eq!(model.collapse_elapsed_secs(1.0), None);
        // The label changed ("waiting +2" -> "only") but the geometry did not.
        assert_eq!(model.relayout(INPUTS, 0.0, 1.0), LayoutChange::ContentOnly);
        assert_eq!(model.layout.cards.len(), 1);
    }

    #[test]
    fn collapsed_layout_ignores_fan_out_state() {
        let mut model = model();
        model.reload(three_sessions(), vec![], 0.0);
        model.is_fanned_out = true;
        model.relayout(INPUTS, 0.0, 0.0);
        assert_eq!(model.layout.cards.len(), 3);
        let collapsed = model.collapsed_layout(INPUTS, 0.0);
        assert_eq!(collapsed.cards.len(), 1);
        assert_eq!(collapsed.cards[0].label, "waiting +2");
        assert_eq!(collapsed.cards[0].session_id.as_deref(), Some("waiting"));
        assert_eq!(collapsed.primary_center_x(), 110.0);
        // Others by recency: "recent" goes right, "busy" left.
        assert_eq!(model.cards_description(), "busy[16-100] waiting[110-242]* recent[252-336]");
    }

    #[test]
    fn bubble_visibility_transition_is_timestamped() {
        let mut model = model();
        model.note_bubble_visibility(false, 1.0);
        assert_eq!(model.bubble_shown_at_secs, None);
        model.note_bubble_visibility(true, 2.0);
        assert_eq!(model.bubble_shown_at_secs, Some(2.0));
        model.note_bubble_visibility(true, 3.0);
        assert_eq!(model.bubble_shown_at_secs, Some(2.0), "stays while visible");
        model.note_bubble_visibility(false, 4.0);
        model.note_bubble_visibility(true, 5.0);
        assert_eq!(model.bubble_shown_at_secs, Some(5.0));
        assert!(model.is_animating(5.1));
    }
}
