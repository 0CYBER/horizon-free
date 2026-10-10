//! The overlay case kinds of the HUD parity fixture.
use horizon_client::{hud::HudAtlas, hud_layout::GuiDraw};
use serde_json::Value;

pub fn run(kind: &str, _c: &Value, _atlas: &HudAtlas) -> Vec<GuiDraw> {
    match kind {
        _ => Vec::new(),
    }
}
