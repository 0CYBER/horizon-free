//! A tooltip of the mod's own for an item link in the chat, a book or a dialog: the mod takes
//! the hover over and draws the item's id and count. Text and entity hovers stay as the game
//! draws them.

// ANCHOR: hover_tooltip
use mod_api::text::TextHover;
use mod_api::{ClientHost, ComponentHoverEvent, Flow, Hooks, Text};

/// The mod.
#[derive(Default)]
pub struct HoverTooltip;

#[mod_api::horizon_mod]
impl Hooks for HoverTooltip {
    fn component_hover(&self, _: &dyn ClientHost, event: &mut ComponentHoverEvent<'_>) -> Flow {
        let TextHover::Item { id, count, .. } = event.hover else {
            return Flow::Continue;
        };
        let lines = [Text::literal(format!("{id} x{count}"))];
        let at = [event.x as f32, event.y as f32];
        // If the lines don't go through, the game draws the hover as usual.
        match event.draw.gui_tooltip(&lines, at) {
            Ok(()) => Flow::Cancel,
            Err(_) => Flow::Continue,
        }
    }
}
// ANCHOR_END: hover_tooltip
