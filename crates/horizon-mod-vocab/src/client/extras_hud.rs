//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// The gap between an overlay and the screen edge or the next overlay.
const MARGIN: f32 = 2.0;
/// Where the next overlay goes in each corner.
// exhaustive: its fields are private, so mods cannot build or destructure it
#[derive(horizon_mod_api_macros::Wire)]
#[wire(internal_field_layouts)]
pub struct Corners {
    pub(crate) width: f32,
    pub(crate) height: f32,
    /// Distance from the corner's top or bottom edge, per `Corner::ALL`.
    pub(crate) used: [f32; 4],
}
impl Corners {
    /// `top_right` reserves the status effect icons' rows.
    pub fn new(width: f32, height: f32, top_right: f32) -> Self {
        Self {
            width,
            height,
            used: [0.0, top_right, 0.0, 0.0],
        }
    }
    /// The top-left position of a `w` x `h` box in `corner`, below (or
    /// above) what the corner already holds.
    pub fn place(&mut self, corner: Corner, w: f32, h: f32) -> (f32, f32) {
        let i = Corner::ALL.iter().position(|c| *c == corner).unwrap_or(0);
        let edge = self.used[i] + MARGIN;
        self.used[i] = edge + h;
        let x = if corner.left() {
            MARGIN
        } else {
            self.width - MARGIN - w
        };
        let y = if corner.top() {
            edge
        } else {
            self.height - edge - h
        };
        (x, y)
    }
}
