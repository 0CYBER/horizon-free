//! Horizon's official far view: the world past the render distance drawn as
//! distant terrain (`render.set-far-terrain`; the game builds the tiles on
//! its own workers). Its settings page picks how far it reaches and the far
//! tiles' detail, kept in the mod's storage (`distance`, `detail`). The
//! player's own LOD Distance setting, when on, wins over the mod.
use horizon_mod_sdk::{
    Mod, Registration,
    render::{self, FarTerrain},
    storage,
    ui::{self, ButtonWidget, Widget},
};

/// Distances in chunks, as LOD Distance offers them.
const DISTANCES: [u32; 5] = [64, 128, 256, 512, 1024];
/// Settings page buttons.
const DISTANCE_BUTTON: u32 = 1;
const DETAIL_BUTTON: u32 = 2;

/// An index into [`DISTANCES`] and Far Detail's Reduced.
struct FarView {
    distance: usize,
    reduced: bool,
}

impl FarView {
    fn show(&self) {
        render::set_far_terrain(Some(FarTerrain {
            distance: DISTANCES[self.distance],
            reduced: self.reduced,
        }));
    }
    fn page(&self) {
        let button = |id, label: String| Widget::Button(ButtonWidget { id, label });
        ui::set_settings(&[
            button(
                DISTANCE_BUTTON,
                format!("Distance: {} chunks", DISTANCES[self.distance]),
            ),
            button(
                DETAIL_BUTTON,
                format!(
                    "Far detail: {}",
                    if self.reduced { "Reduced" } else { "Full" }
                ),
            ),
        ]);
    }
    fn save(&self) {
        let _ = storage::set("distance", DISTANCES[self.distance].to_string().as_bytes());
        let _ = storage::set("detail", if self.reduced { b"reduced" } else { b"full" });
    }
}

fn stored(key: &str) -> Option<String> {
    storage::get(key)
        .and_then(|v| String::from_utf8(v).ok())
        .map(|v| v.trim().to_owned())
}

impl Mod for FarView {
    fn init() -> Result<(Self, Registration), String> {
        // 256 chunks and full detail, as the docs suggest; an unknown value
        // keeps the default.
        let far = FarView {
            distance: stored("distance")
                .and_then(|v| DISTANCES.iter().position(|d| d.to_string() == v))
                .unwrap_or(2),
            reduced: stored("detail").as_deref() == Some("reduced"),
        };
        far.show();
        far.page();
        Ok((far, Registration::new()))
    }
    fn ui_select(&mut self, id: u32, _row: u32) {
        match id {
            DISTANCE_BUTTON => self.distance = (self.distance + 1) % DISTANCES.len(),
            DETAIL_BUTTON => self.reduced = !self.reduced,
            _ => return,
        }
        self.save();
        self.show();
        self.page();
    }
    fn ui_settings_opened(&mut self) {
        self.page();
    }
}

horizon_mod_sdk::export_mod!(FarView);
