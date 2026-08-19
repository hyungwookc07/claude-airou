//! Pure timing curves for the overlay's motion — approximations of the SwiftUI animations
//! `PetView.swift` uses (spring settle, ease in/out, phase animators, repeating effects).
//! Everything takes an elapsed time in seconds and returns a value, so the painter can
//! evaluate any animation for the current instant without keeping per-frame state.

use super::row_layout::RowLayout;

/// SwiftUI `.spring(duration: 0.34, bounce: 0.18)`: cards settling into their slot.
pub const CARD_SETTLE_DURATION_SECS: f32 = 0.34;
pub const CARD_SETTLE_BOUNCE: f32 = 0.18;
/// SwiftUI `.easeIn(duration: 0.22)`: side cards folding back into the primary before the row collapses.
pub const CARD_FOLD_DURATION_SECS: f32 = 0.22;
/// Where a card that was not on screen before comes from: inside the primary pet, small and transparent.
pub const CARD_ENTRANCE_SCALE: f32 = 0.45;
/// Done / hello: the primary sprite hops (`phaseAnimator([0, -14, 0, -7, 0])`, 0.16 s spring per phase).
pub const DONE_BOUNCE_PHASES: [f32; 5] = [0.0, -14.0, 0.0, -7.0, 0.0];
pub const DONE_BOUNCE_PHASE_SECS: f32 = 0.16;
/// Error: the primary sprite shakes sideways (`phaseAnimator([0, -5, 5, -4, 4, 0])`, 0.06 s linear per phase).
pub const ERROR_SHAKE_PHASES: [f32; 6] = [0.0, -5.0, 5.0, -4.0, 4.0, 0.0];
pub const ERROR_SHAKE_PHASE_SECS: f32 = 0.06;
/// The floating heart after a pet click: rises 28 pt and fades over 1.1 s (ease-out).
pub const HEART_DURATION_SECS: f32 = 1.1;
pub const HEART_RISE_POINTS: f32 = 28.0;
/// Speech bubble entrance: fade + scale from 0.92 over 0.16 s (ease-out).
pub const BUBBLE_APPEAR_DURATION_SECS: f32 = 0.16;
pub const BUBBLE_APPEAR_SCALE: f32 = 0.92;
/// Status badge entrance/change: `.spring(duration: 0.25)` scale + opacity.
pub const BADGE_APPEAR_DURATION_SECS: f32 = 0.25;
/// `symbolEffect(.pulse)` cycle length for the waiting-approval / needs-input badges.
pub const BADGE_PULSE_PERIOD_SECS: f32 = 1.0;
/// One full turn of the working gear (`.linear(duration: 2.4).repeatForever`).
pub const GEAR_TURN_SECS: f32 = 2.4;

/// Quadratic ease-out (close to SwiftUI's `.easeOut` bezier), clamped to `[0, 1]`.
pub fn ease_out(progress: f32) -> f32 {
    let clamped = progress.clamp(0.0, 1.0);
    1.0 - (1.0 - clamped) * (1.0 - clamped)
}

/// Quadratic ease-in (close to SwiftUI's `.easeIn` bezier), clamped to `[0, 1]`.
pub fn ease_in(progress: f32) -> f32 {
    let clamped = progress.clamp(0.0, 1.0);
    clamped * clamped
}

/// SwiftUI `Spring(duration:bounce:)` response, normalised so it goes 0 → 1 (with a small
/// overshoot for `bounce > 0`). `duration` is the perceptual duration (the spring's response
/// period), `bounce` maps to a damping ratio of `1 - bounce`. Returns exactly 1 once the
/// motion has settled (`is_spring_settled`).
pub fn spring_progress(elapsed_secs: f32, duration_secs: f32, bounce: f32) -> f32 {
    if elapsed_secs <= 0.0 {
        return 0.0;
    }
    if is_spring_settled(elapsed_secs, duration_secs) {
        return 1.0;
    }
    let natural_frequency = std::f32::consts::TAU / duration_secs.max(0.01);
    let damping_ratio = (1.0 - bounce).clamp(0.05, 1.0);
    let time = elapsed_secs;
    if damping_ratio >= 0.999 {
        // Critically damped.
        let decay = (-natural_frequency * time).exp();
        return 1.0 - decay * (1.0 + natural_frequency * time);
    }
    let damped_frequency = natural_frequency * (1.0 - damping_ratio * damping_ratio).sqrt();
    let decay = (-damping_ratio * natural_frequency * time).exp();
    let oscillation = (damped_frequency * time).cos()
        + (damping_ratio * natural_frequency / damped_frequency) * (damped_frequency * time).sin();
    1.0 - decay * oscillation
}

/// A spring is treated as settled after twice its perceptual duration.
pub fn is_spring_settled(elapsed_secs: f32, duration_secs: f32) -> bool {
    elapsed_secs >= duration_secs * 2.0
}

/// SwiftUI `phaseAnimator(phases, trigger:)`: walks from the first phase to the last, one
/// transition per `phase_duration_secs`, easing each transition with `ease` (a 0→1 curve).
/// Returns `None` once every transition is over (the view sits at the last phase).
pub fn phase_animator_value(
    phases: &[f32],
    phase_duration_secs: f32,
    elapsed_secs: f32,
    ease: impl Fn(f32) -> f32,
) -> Option<f32> {
    if phases.len() < 2 || phase_duration_secs <= 0.0 || elapsed_secs < 0.0 {
        return None;
    }
    let transition_count = phases.len() - 1;
    let total = phase_duration_secs * transition_count as f32;
    if elapsed_secs >= total {
        return None;
    }
    let transition_index = ((elapsed_secs / phase_duration_secs).floor() as usize).min(transition_count - 1);
    let local_progress = (elapsed_secs - transition_index as f32 * phase_duration_secs) / phase_duration_secs;
    let from = phases[transition_index];
    let to = phases[transition_index + 1];
    Some(from + (to - from) * ease(local_progress))
}

/// Vertical offset (points) of the primary sprite during the done/hello hop; `None` when idle.
pub fn done_bounce_offset_y(elapsed_secs: f32) -> Option<f32> {
    phase_animator_value(&DONE_BOUNCE_PHASES, DONE_BOUNCE_PHASE_SECS, elapsed_secs, ease_out)
}

/// Horizontal offset (points) of the primary sprite during the error shake; `None` when idle.
pub fn error_shake_offset_x(elapsed_secs: f32) -> Option<f32> {
    phase_animator_value(&ERROR_SHAKE_PHASES, ERROR_SHAKE_PHASE_SECS, elapsed_secs, |progress| progress)
}

/// The floating heart's (rise in points, opacity); `None` once it has faded out.
pub fn floating_heart(elapsed_secs: f32) -> Option<(f32, f32)> {
    if !(0.0..HEART_DURATION_SECS).contains(&elapsed_secs) {
        return None;
    }
    let progress = ease_out(elapsed_secs / HEART_DURATION_SECS);
    Some((HEART_RISE_POINTS * progress, 1.0 - progress * 0.9))
}

/// Speech-bubble entrance: (scale about the bottom edge, opacity). Settles at (1, 1).
pub fn bubble_appearance(elapsed_secs: f32) -> (f32, f32) {
    let progress = ease_out(elapsed_secs / BUBBLE_APPEAR_DURATION_SECS);
    (BUBBLE_APPEAR_SCALE + (1.0 - BUBBLE_APPEAR_SCALE) * progress, progress)
}

/// Status badge pop-in: (scale, opacity) with a light spring; settles at (1, 1).
pub fn badge_appearance(elapsed_secs: f32) -> (f32, f32) {
    let progress = spring_progress(elapsed_secs, BADGE_APPEAR_DURATION_SECS, 0.0);
    (progress.max(0.0), progress.clamp(0.0, 1.0))
}

/// Opacity of a pulsing badge symbol (SF `pulse`): sinusoidal between 0.35 and 1.
pub fn badge_pulse_opacity(elapsed_secs: f32) -> f32 {
    let phase = (elapsed_secs / BADGE_PULSE_PERIOD_SECS) * std::f32::consts::TAU;
    0.675 + 0.325 * phase.cos()
}

/// Rotation of the working gear in radians (one turn per `GEAR_TURN_SECS`).
pub fn gear_rotation_radians(elapsed_secs: f32) -> f32 {
    (elapsed_secs / GEAR_TURN_SECS).fract() * std::f32::consts::TAU
}

/// Where a card starts its FLIP motion (offset x in content points, scale, opacity) relative
/// to its new slot — port of `SessionCardView.animateFromPreviousLayout`. `panel_shift_x`
/// is how far the panel itself moved on screen for this layout change, so all motion is
/// computed in screen space and the primary pet never appears to move.
/// Returns `None` when the card is already exactly where it was (nothing to animate).
pub fn card_motion_start(
    previous: &RowLayout,
    new_layout: &RowLayout,
    card_id: &str,
    panel_shift_x: f32,
) -> Option<(f32, f32, f32)> {
    let card = new_layout.cards.iter().find(|card| card.id() == card_id)?;
    let new_screen_center_x = card.center_x() + panel_shift_x;
    let (start_offset_x, start_scale, start_opacity) =
        match previous.cards.iter().find(|old| old.id() == card_id) {
            Some(old) => (old.center_x() - new_screen_center_x, old.pixel_scale / card.pixel_scale, 1.0),
            None => (previous.primary_center_x() - new_screen_center_x, CARD_ENTRANCE_SCALE, 0.0),
        };
    if start_offset_x.abs() < 0.5 && (start_scale - 1.0).abs() < 0.01 && start_opacity == 1.0 {
        return None;
    }
    Some((start_offset_x, start_scale, start_opacity))
}

/// Interpolates a card's FLIP motion at `elapsed_secs`: (offset x, scale, opacity).
pub fn card_motion_at(start: (f32, f32, f32), elapsed_secs: f32) -> (f32, f32, f32) {
    let progress = spring_progress(elapsed_secs, CARD_SETTLE_DURATION_SECS, CARD_SETTLE_BOUNCE);
    let (offset_x, scale, opacity) = start;
    (
        offset_x * (1.0 - progress),
        scale + (1.0 - scale) * progress,
        (opacity + (1.0 - opacity) * progress).clamp(0.0, 1.0),
    )
}

/// A side card folding back into the primary before the row collapses: (offset x, scale,
/// opacity) at `elapsed_secs` (ease-in over `CARD_FOLD_DURATION_SECS`).
pub fn card_fold_at(card_center_x: f32, primary_center_x: f32, elapsed_secs: f32) -> (f32, f32, f32) {
    let progress = ease_in(elapsed_secs / CARD_FOLD_DURATION_SECS);
    (
        (primary_center_x - card_center_x) * progress,
        1.0 + (CARD_ENTRANCE_SCALE - 1.0) * progress,
        1.0 - progress,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::row_layout::GridSize;

    #[test]
    fn easing_curves_are_monotonic_and_clamped() {
        assert_eq!(ease_out(-1.0), 0.0);
        assert_eq!(ease_out(0.0), 0.0);
        assert_eq!(ease_out(1.0), 1.0);
        assert_eq!(ease_out(2.0), 1.0);
        assert!(ease_out(0.5) > 0.5, "ease-out is fast first");
        assert!(ease_in(0.5) < 0.5, "ease-in is slow first");
        assert_eq!(ease_in(1.0), 1.0);
    }

    #[test]
    fn spring_starts_at_zero_overshoots_and_settles_at_one() {
        assert_eq!(spring_progress(0.0, 0.34, 0.18), 0.0);
        let mut max_value: f32 = 0.0;
        let mut previous = 0.0;
        let mut step = 0.0;
        while step < 0.2 {
            let value = spring_progress(step, 0.34, 0.18);
            assert!(value >= previous - 1e-4, "rises during the first stretch");
            previous = value;
            max_value = max_value.max(value);
            step += 0.005;
        }
        let mut step = 0.0;
        while step < 0.68 {
            max_value = max_value.max(spring_progress(step, 0.34, 0.18));
            step += 0.005;
        }
        assert!(max_value > 1.0 && max_value < 1.15, "small overshoot, got {max_value}");
        assert!(is_spring_settled(0.68, 0.34));
        assert_eq!(spring_progress(0.68, 0.34, 0.18), 1.0);
        assert_eq!(spring_progress(5.0, 0.34, 0.18), 1.0);
        // Critically damped (bounce 0) never overshoots.
        let mut step = 0.0;
        while step < 0.5 {
            assert!(spring_progress(step, 0.25, 0.0) <= 1.0 + 1e-5);
            step += 0.005;
        }
    }

    #[test]
    fn phase_animator_walks_the_phases_then_ends() {
        let linear = |progress: f32| progress;
        assert_eq!(phase_animator_value(&[0.0, 10.0, 0.0], 1.0, 0.0, linear), Some(0.0));
        assert_eq!(phase_animator_value(&[0.0, 10.0, 0.0], 1.0, 0.5, linear), Some(5.0));
        assert_eq!(phase_animator_value(&[0.0, 10.0, 0.0], 1.0, 1.0, linear), Some(10.0));
        assert_eq!(phase_animator_value(&[0.0, 10.0, 0.0], 1.0, 1.5, linear), Some(5.0));
        assert_eq!(phase_animator_value(&[0.0, 10.0, 0.0], 1.0, 2.0, linear), None);
        assert_eq!(phase_animator_value(&[0.0], 1.0, 0.5, linear), None);
        assert_eq!(phase_animator_value(&[0.0, 1.0], 0.0, 0.5, linear), None);
        assert_eq!(phase_animator_value(&[0.0, 1.0], 1.0, -0.5, linear), None);
    }

    #[test]
    fn done_bounce_and_error_shake_have_the_swift_durations() {
        assert!(done_bounce_offset_y(0.0).is_some());
        assert!(done_bounce_offset_y(0.16 * 4.0 - 0.01).is_some());
        assert_eq!(done_bounce_offset_y(0.16 * 4.0), None);
        // First transition heads upwards (negative y).
        assert!(done_bounce_offset_y(0.08).unwrap() < -5.0);
        assert!(error_shake_offset_x(0.0).is_some());
        assert_eq!(error_shake_offset_x(0.3), None);
        assert!((error_shake_offset_x(0.03).unwrap() - -2.5).abs() < 1e-4, "linear halfway to -5");
    }

    #[test]
    fn heart_rises_and_fades_then_disappears() {
        let (rise_start, opacity_start) = floating_heart(0.0).unwrap();
        assert_eq!(rise_start, 0.0);
        assert_eq!(opacity_start, 1.0);
        let (rise_mid, opacity_mid) = floating_heart(0.55).unwrap();
        assert!(rise_mid > 14.0 && rise_mid < 28.0);
        assert!(opacity_mid < 1.0 && opacity_mid > 0.1);
        assert_eq!(floating_heart(1.1), None);
        assert_eq!(floating_heart(-0.1), None);
    }

    #[test]
    fn bubble_and_badge_entrances_settle_at_identity() {
        assert_eq!(bubble_appearance(0.0), (0.92, 0.0));
        assert_eq!(bubble_appearance(1.0), (1.0, 1.0));
        let (scale, opacity) = badge_appearance(0.0);
        assert_eq!((scale, opacity), (0.0, 0.0));
        assert_eq!(badge_appearance(1.0), (1.0, 1.0));
    }

    #[test]
    fn repeating_effects_cycle() {
        assert!((badge_pulse_opacity(0.0) - 1.0).abs() < 1e-5);
        assert!((badge_pulse_opacity(0.5) - 0.35).abs() < 1e-5);
        assert!((badge_pulse_opacity(1.0) - 1.0).abs() < 1e-5);
        assert_eq!(gear_rotation_radians(0.0), 0.0);
        assert!((gear_rotation_radians(1.2) - std::f32::consts::PI).abs() < 1e-4);
        assert!(gear_rotation_radians(2.4).abs() < 1e-4);
    }

    fn layout(labels: &[&str], primary_index: usize) -> RowLayout {
        let labels: Vec<String> = labels.iter().map(|label| label.to_string()).collect();
        let ids: Vec<Option<String>> = labels.iter().map(|label| Some(label.clone())).collect();
        RowLayout::make(GridSize { width: 28, height: 28 }, 5.0, &labels, &ids, primary_index, 0.0, false)
    }

    #[test]
    fn card_motion_start_places_new_cards_inside_the_previous_primary() {
        let collapsed = layout(&["mid"], 0);
        let expanded = layout(&["left", "mid", "right"], 1);
        // The panel moved left so that "mid" stays put on screen.
        let panel_shift_x = collapsed.primary_center_x() - expanded.primary_center_x();
        // The primary did not move on screen -> nothing to animate.
        assert_eq!(card_motion_start(&collapsed, &expanded, "mid", panel_shift_x), None);
        // A new side card starts at the previous primary's screen position, small and transparent.
        let (offset_x, scale, opacity) = card_motion_start(&collapsed, &expanded, "left", panel_shift_x).unwrap();
        let left = &expanded.cards[0];
        assert!((offset_x - (collapsed.primary_center_x() - (left.center_x() + panel_shift_x))).abs() < 1e-3);
        assert!(offset_x > 0.0, "left card slides out towards the left, so it starts to the right");
        assert_eq!(scale, CARD_ENTRANCE_SCALE);
        assert_eq!(opacity, 0.0);
        // Unknown card ids yield None.
        assert_eq!(card_motion_start(&collapsed, &expanded, "ghost", 0.0), None);
    }

    #[test]
    fn card_motion_start_for_a_swapped_primary_uses_old_position_and_scale() {
        let before = layout(&["a", "b", "c"], 1);
        let after = layout(&["b", "a", "c"], 1); // "a" pinned: now primary in the middle
        let (offset_x, scale, opacity) = card_motion_start(&before, &after, "a", 0.0).unwrap();
        assert!(offset_x < 0.0, "a comes from the left slot");
        assert!((scale - 3.0 / 5.0).abs() < 1e-5, "was a side card at scale 3, now 5");
        assert_eq!(opacity, 1.0);
        // Interpolation ends at identity.
        assert_eq!(card_motion_at((offset_x, scale, opacity), 10.0), (0.0, 1.0, 1.0));
        let (mid_offset, mid_scale, _) = card_motion_at((offset_x, scale, opacity), 0.05);
        assert!(mid_offset < 0.0 && mid_offset > offset_x);
        assert!(mid_scale > scale && mid_scale < 1.0);
    }

    #[test]
    fn card_fold_moves_into_the_primary_and_vanishes() {
        assert_eq!(card_fold_at(50.0, 150.0, 0.0), (0.0, 1.0, 1.0));
        let (offset_x, scale, opacity) = card_fold_at(50.0, 150.0, CARD_FOLD_DURATION_SECS);
        assert_eq!(offset_x, 100.0);
        assert!((scale - CARD_ENTRANCE_SCALE).abs() < 1e-6);
        assert_eq!(opacity, 0.0);
    }
}
