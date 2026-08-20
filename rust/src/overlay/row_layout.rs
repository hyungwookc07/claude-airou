//! Deterministic horizontal layout of one or more session "cards" (sprite + gauge + label).
//! Port of the Swift original's `UI/RowLayout.swift`: both the painter and the panel
//! placement derive geometry from this, so the panel can grow/shrink while keeping the
//! primary pet exactly where it was on screen. Everything is in points (logical pixels).

/// One card in the row.
#[derive(Debug, Clone, PartialEq)]
pub struct RowCard {
    /// `None` when there is no session at all (the "no session" placeholder card).
    pub session_id: Option<String>,
    pub is_primary: bool,
    /// Integer-valued so pet pixels stay crisp.
    pub pixel_scale: f32,
    pub width: f32,
    /// Leading edge within the content.
    pub x: f32,
    pub label: String,
}

impl RowCard {
    /// Stable identity across layouts (Swift `Card.id`).
    pub fn id(&self) -> &str {
        self.session_id.as_deref().unwrap_or("none")
    }

    pub fn center_x(&self) -> f32 {
        self.x + self.width / 2.0
    }
}

pub const MINIMUM_CONTENT_WIDTH: f32 = 220.0;
pub const HORIZONTAL_PADDING: f32 = 16.0;
/// Room reserved on every side for the effort aura. Without it the halo is clipped by the
/// card canvas, which flattens the size ramp to nothing: `max` ends up only brighter than
/// `low`, never wider.
///
/// It has to scale with the pet. The halo's radius is a multiple of the sprite's
/// half-height, so a constant margin fits at Small and clips hard at Large — the size
/// where the aura is most visible. Capped at both ends because the panel takes mouse
/// clicks unless click-through is on: every reserved point is a point of desktop the user
/// can no longer click.
///
/// Reserved unconditionally (not only when a session reports effort) so the panel geometry
/// — and with it the user's saved window origin — never jumps mid-session.
pub fn aura_reserved_margin(primary_sprite_height: f32) -> f32 {
    (primary_sprite_height * 0.5 * 0.62).clamp(20.0, 64.0).round()
}
pub const CARD_SPACING: f32 = 10.0;
pub const MINIMUM_CARD_WIDTH: f32 = 72.0;
pub const MAXIMUM_CARD_WIDTH: f32 = 132.0;
pub const SIDE_SPRITE_SCALE_FACTOR: f32 = 0.7;
pub const SPEECH_BUBBLE_RESERVED_HEIGHT: f32 = 66.0;
/// Gap between the bubble's tail and the top of the sprite.
pub const SPEECH_BUBBLE_BOTTOM_INSET: f32 = 12.0;
pub const SESSION_BADGE_RESERVED_HEIGHT: f32 = 22.0;
/// Battery gauge row between the sprite and the label (0 when the gauge is off).
pub const GAUGE_RESERVED_HEIGHT: f32 = 16.0;
pub const VERTICAL_PADDING: f32 = 12.0;
/// Rough width of one label character at the badge font, for card sizing without text measurement.
pub const APPROXIMATE_LABEL_CHARACTER_WIDTH: f32 = 6.2;
pub const LABEL_HORIZONTAL_INSET: f32 = 18.0;
/// Room for the status icon that sits at the right end of the label capsule.
pub const LABEL_STATUS_ICON_ALLOWANCE: f32 = 18.0;
/// The speech bubble is centred over the primary card; the row reserves room for the current
/// bubble width around that centre so the bubble never has to be pushed off the pet.
pub const SPEECH_BUBBLE_MAX_WIDTH: f32 = 300.0;
pub const SPEECH_BUBBLE_EDGE_MARGIN: f32 = 4.0;
/// Vertical spacing inside a card (SwiftUI `VStack(spacing: 4)`).
pub const CARD_VERTICAL_SPACING: f32 = 4.0;

#[derive(Debug, Clone, PartialEq)]
pub struct RowLayout {
    pub cards: Vec<RowCard>,
    pub content_width: f32,
    /// Room kept free around the pet for the effort aura (see `aura_reserved_margin`).
    pub aura_margin: f32,
    pub content_height: f32,
    pub primary_sprite_width: f32,
    pub primary_sprite_height: f32,
    /// Width the speech bubble will take for the current text (0 when there is no bubble).
    pub speech_bubble_width: f32,
    /// Whether a gauge row is laid out under the sprites.
    pub shows_gauge: bool,
}

/// Inputs shared by every `RowLayout::make` call for one pet/config.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridSize {
    pub width: u32,
    pub height: u32,
}

impl RowLayout {
    pub fn gauge_height(&self) -> f32 {
        if self.shows_gauge {
            GAUGE_RESERVED_HEIGHT
        } else {
            0.0
        }
    }

    /// Height of one card: sprite + gauge + label (+ spacing).
    pub fn card_height(&self) -> f32 {
        self.primary_sprite_height + self.gauge_height() + SESSION_BADGE_RESERVED_HEIGHT + CARD_VERTICAL_SPACING
    }

    pub fn primary_card(&self) -> &RowCard {
        self.cards.iter().find(|card| card.is_primary).unwrap_or(&self.cards[0])
    }

    pub fn primary_center_x(&self) -> f32 {
        self.primary_card().center_x()
    }

    /// Side cards render the pet smaller: 70 % of the primary scale, floored, never below 2.
    pub fn side_scale(pixel_scale: f32) -> f32 {
        ((pixel_scale * SIDE_SPRITE_SCALE_FACTOR).floor()).max(2.0)
    }

    /// `labels`: one per card, left to right; `primary_index` marks the full-size card.
    /// Panics (like Swift's precondition) when the inputs are inconsistent.
    pub fn make(
        grid: GridSize,
        pixel_scale: f32,
        labels: &[String],
        session_ids: &[Option<String>],
        primary_index: usize,
        speech_bubble_width: f32,
        shows_gauge: bool,
    ) -> RowLayout {
        assert!(
            labels.len() == session_ids.len() && !labels.is_empty() && primary_index < labels.len(),
            "RowLayout::make: labels/sessionIds mismatch"
        );
        let side_scale = Self::side_scale(pixel_scale);
        let primary_sprite_width = grid.width as f32 * pixel_scale;
        let primary_sprite_height = grid.height as f32 * pixel_scale;

        let mut card_widths: Vec<f32> = Vec::with_capacity(labels.len());
        for (index, label) in labels.iter().enumerate() {
            let scale = if index == primary_index { pixel_scale } else { side_scale };
            let sprite_width = grid.width as f32 * scale;
            let label_width = label.chars().count() as f32 * APPROXIMATE_LABEL_CHARACTER_WIDTH
                + LABEL_HORIZONTAL_INSET
                + LABEL_STATUS_ICON_ALLOWANCE;
            let width = MAXIMUM_CARD_WIDTH.min(MINIMUM_CARD_WIDTH.max(sprite_width).max(label_width));
            card_widths.push((width / 2.0).ceil() * 2.0); // even, so card centres are whole points
        }

        let row_width: f32 =
            card_widths.iter().sum::<f32>() + CARD_SPACING * (card_widths.len().saturating_sub(1)) as f32;
        let aura_margin = aura_reserved_margin(primary_sprite_height);
        let mut content_width =
            MINIMUM_CONTENT_WIDTH.max(row_width + (HORIZONTAL_PADDING + aura_margin) * 2.0);
        let mut row_leading = ((content_width - row_width) / 2.0).round();

        // Make room for the bubble around the primary card's centre (it may sit off-centre in the row).
        let primary_leading = row_leading
            + card_widths[..primary_index].iter().sum::<f32>()
            + CARD_SPACING * primary_index as f32;
        let primary_center = primary_leading + card_widths[primary_index] / 2.0;
        let bubble_width = SPEECH_BUBBLE_MAX_WIDTH.min(speech_bubble_width.max(0.0));
        let bubble_half = bubble_width / 2.0 + SPEECH_BUBBLE_EDGE_MARGIN;
        let left_shortfall = (bubble_half - primary_center).max(0.0);
        row_leading += left_shortfall;
        content_width += left_shortfall;
        let right_shortfall = ((primary_center + left_shortfall + bubble_half) - content_width).max(0.0);
        content_width += right_shortfall;

        let gauge_height = if shows_gauge { GAUGE_RESERVED_HEIGHT } else { 0.0 };
        // The halo also reaches below the label, so the panel keeps that much room under
        // the cards; without it the canvas is cut off flush with the label capsule.
        let content_height = SPEECH_BUBBLE_RESERVED_HEIGHT
            + primary_sprite_height
            + gauge_height
            + SESSION_BADGE_RESERVED_HEIGHT
            + VERTICAL_PADDING
            + aura_margin;

        let mut cards: Vec<RowCard> = Vec::with_capacity(labels.len());
        let mut x = row_leading;
        for index in 0..labels.len() {
            let is_primary = index == primary_index;
            cards.push(RowCard {
                session_id: session_ids[index].clone(),
                is_primary,
                pixel_scale: if is_primary { pixel_scale } else { side_scale },
                width: card_widths[index],
                x,
                label: labels[index].clone(),
            });
            x += card_widths[index] + CARD_SPACING;
        }

        RowLayout {
            cards,
            content_width,
            content_height,
            primary_sprite_width,
            primary_sprite_height,
            speech_bubble_width: bubble_width,
            shows_gauge,
            aura_margin,
        }
    }

    /// The card under a content-space x coordinate, if any.
    pub fn card_at_content_x(&self, x: f32) -> Option<&RowCard> {
        self.cards.iter().find(|card| x >= card.x && x <= card.x + card.width)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GRID: GridSize = GridSize { width: 28, height: 28 };

    fn labels(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| item.to_string()).collect()
    }

    fn ids(items: &[Option<&str>]) -> Vec<Option<String>> {
        items.iter().map(|item| item.map(str::to_string)).collect()
    }

    #[test]
    fn side_scale_is_seventy_percent_floored_min_two() {
        assert_eq!(RowLayout::side_scale(5.0), 3.0);
        assert_eq!(RowLayout::side_scale(3.0), 2.0);
        assert_eq!(RowLayout::side_scale(7.0), 4.0);
        assert_eq!(RowLayout::side_scale(1.0), 2.0);
    }

    #[test]
    fn single_card_is_centred_in_minimum_content_width() {
        let layout = RowLayout::make(GRID, 5.0, &labels(&["no session"]), &ids(&[None]), 0, 0.0, false);
        assert_eq!(layout.cards.len(), 1);
        let card = &layout.cards[0];
        // sprite 140 > max card 132 -> clamped to 132, already even.
        assert_eq!(card.width, 132.0);
        // 132 + 2 * (16 padding + 43 aura margin at this sprite size); the minimum no
        // longer binds, and the panel keeps one margin of room under the cards too.
        assert_eq!(layout.content_width, 250.0);
        assert_eq!(card.x, 59.0);
        assert_eq!(layout.primary_center_x(), 125.0);
        assert_eq!(layout.content_height, 66.0 + 140.0 + 22.0 + 12.0 + 43.0);
        assert_eq!(layout.card_height(), 140.0 + 22.0 + 4.0);
        assert!(card.is_primary);
        assert_eq!(card.id(), "none");
    }

    #[test]
    fn gauge_adds_sixteen_points_to_height_and_card() {
        let with_gauge = RowLayout::make(GRID, 3.0, &labels(&["a"]), &ids(&[Some("a")]), 0, 0.0, true);
        let without = RowLayout::make(GRID, 3.0, &labels(&["a"]), &ids(&[Some("a")]), 0, 0.0, false);
        assert_eq!(with_gauge.content_height - without.content_height, 16.0);
        assert_eq!(with_gauge.card_height() - without.card_height(), 16.0);
        assert_eq!(with_gauge.gauge_height(), 16.0);
        assert_eq!(without.gauge_height(), 0.0);
    }

    #[test]
    fn card_width_uses_label_approximation_and_is_even() {
        // 3 px scale: sprite 84 wide; label "pass_finder" = 11 chars * 6.2 + 36 = 104.2 -> 106 (even).
        let layout = RowLayout::make(GRID, 3.0, &labels(&["pass_finder"]), &ids(&[Some("a")]), 0, 0.0, false);
        assert_eq!(layout.cards[0].width, 106.0);
        // Very long labels are clamped at the maximum card width.
        let long = RowLayout::make(GRID, 3.0, &labels(&["a-very-long-project-name-indeed"]), &ids(&[Some("a")]), 0, 0.0, false);
        assert_eq!(long.cards[0].width, 132.0);
        // Tiny sprite + short label -> minimum width.
        let tiny = RowLayout::make(GridSize { width: 8, height: 8 }, 2.0, &labels(&["x"]), &ids(&[Some("a")]), 0, 0.0, false);
        assert_eq!(tiny.cards[0].width, 72.0);
    }

    #[test]
    fn three_cards_primary_in_the_middle_with_side_scale() {
        let layout = RowLayout::make(
            GRID,
            5.0,
            &labels(&["left", "mid", "right"]),
            &ids(&[Some("l"), Some("m"), Some("r")]),
            1,
            0.0,
            true,
        );
        // side sprite 28*3 = 84 -> card 84 (label "left" 4*6.2+36=60.8 < 84).
        let widths: Vec<f32> = layout.cards.iter().map(|card| card.width).collect();
        assert_eq!(widths, vec![84.0, 132.0, 84.0]);
        let scales: Vec<f32> = layout.cards.iter().map(|card| card.pixel_scale).collect();
        assert_eq!(scales, vec![3.0, 5.0, 3.0]);
        let row_width = 84.0 + 132.0 + 84.0 + 2.0 * CARD_SPACING;
        assert_eq!(
            layout.content_width,
            row_width + 2.0 * (HORIZONTAL_PADDING + aura_reserved_margin(140.0))
        );
        let leading = HORIZONTAL_PADDING + aura_reserved_margin(140.0);
        assert_eq!(layout.cards[0].x, leading);
        assert_eq!(layout.cards[1].x, leading + 84.0 + 10.0);
        assert_eq!(layout.cards[2].x, leading + 84.0 + 10.0 + 132.0 + 10.0);
        assert!(layout.cards[1].is_primary);
        assert_eq!(layout.primary_center_x(), leading + 94.0 + 66.0);
        // Primary sprite size is unaffected by the side cards.
        assert_eq!(layout.primary_sprite_width, 140.0);
        // Hit testing. Cards now sit at l[59-143] m[153-285] r[295-379]; the reserved aura
        // margin is empty space, so a click out there hits no card.
        assert_eq!(layout.card_at_content_x(70.0).unwrap().id(), "l");
        assert_eq!(layout.card_at_content_x(200.0).unwrap().id(), "m");
        assert_eq!(layout.card_at_content_x(300.0).unwrap().id(), "r");
        assert!(layout.card_at_content_x(148.0).is_none(), "gap between cards");
        assert!(layout.card_at_content_x(20.0).is_none(), "inside the reserved aura margin");
        assert!(layout.card_at_content_x(-1.0).is_none());
        assert!(layout.card_at_content_x(1000.0).is_none());
    }

    #[test]
    fn bubble_room_is_reserved_around_the_primary_centre() {
        // Single card, 300 pt bubble: needs 154 on each side of the centre (110) ->
        // 44 shortfall left and right -> content 308, centre 154.
        let layout = RowLayout::make(GRID, 5.0, &labels(&["p"]), &ids(&[Some("p")]), 0, 300.0, false);
        assert_eq!(layout.content_width, 308.0);
        assert_eq!(layout.primary_center_x(), 154.0);
        assert_eq!(layout.speech_bubble_width, 300.0);
        // Widths above the maximum are clamped, negative ones ignored.
        let clamped = RowLayout::make(GRID, 5.0, &labels(&["p"]), &ids(&[Some("p")]), 0, 900.0, false);
        assert_eq!(clamped.speech_bubble_width, 300.0);
        assert_eq!(clamped.content_width, 308.0);
        let negative = RowLayout::make(GRID, 5.0, &labels(&["p"]), &ids(&[Some("p")]), 0, -5.0, false);
        assert_eq!(negative.speech_bubble_width, 0.0);
        assert_eq!(negative.content_width, 250.0);
        // A small bubble fits without changing the width.
        let small = RowLayout::make(GRID, 5.0, &labels(&["p"]), &ids(&[Some("p")]), 0, 120.0, false);
        assert_eq!(small.content_width, 250.0);
        assert_eq!(small.primary_center_x(), 125.0);
    }

    #[test]
    fn primary_at_the_edge_of_the_row_shifts_row_for_the_bubble() {
        // Primary is the leftmost card: bubble reserve pushes the whole row right.
        let layout = RowLayout::make(
            GRID,
            5.0,
            &labels(&["p", "b"]),
            &ids(&[Some("p"), Some("b")]),
            0,
            200.0,
            false,
        );
        // Row: 132 + 10 + 84 = 226 -> content 344, leading 59, primary centre 125.
        // bubbleHalf 104 < 125, so the bubble already fits and the row does not shift.
        assert_eq!(layout.cards[0].x, 59.0);
        assert_eq!(layout.primary_center_x(), 125.0);
        assert_eq!(layout.content_width, 344.0);
    }

    #[test]
    #[should_panic]
    fn mismatched_inputs_panic_like_swift_precondition() {
        let _ = RowLayout::make(GRID, 5.0, &labels(&["a", "b"]), &ids(&[Some("a")]), 0, 0.0, false);
    }
}
