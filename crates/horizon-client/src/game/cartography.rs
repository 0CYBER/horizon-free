//! The cartography table (`CartographyTableMenu`, `CartographyTableScreen`):
//! zooming a map out with paper, cloning it with an empty map, locking it
//! with a glass pane, the post-processing on take (`MapItem.scaleMap`,
//! `lockMap`) and the preview with its error and lock sprites.
use super::{
    Game,
    maps::{MapData, map_id},
    stations::Station,
};
use horizon_client::{hud::HudBatch, items::ItemStack};

impl Game<'_> {
    /// `setupResultSlot`.
    pub(super) fn cartography_result(&self, s: &Station) -> Option<ItemStack> {
        let (map, extra) = (s.inputs[0].as_ref()?, s.inputs[1].as_ref()?);
        let data = self.maps.data.get(&map_id(map)?)?;
        match &*extra.item {
            "minecraft:paper" if !data.locked && data.scale < 4 => Some(ItemStack {
                count: 1,
                ..map.clone()
            }),
            "minecraft:glass_pane" if !data.locked => Some(ItemStack {
                count: 1,
                ..map.clone()
            }),
            "minecraft:map" => Some(ItemStack {
                count: 2,
                ..map.clone()
            }),
            _ => None,
        }
    }
    /// `onCraftedPostProcess` for the taken result: a scaled or locked copy
    /// under a new map id. Also spends the inputs' extra item kind check.
    pub(super) fn cartography_post(&mut self, mut result: ItemStack) -> ItemStack {
        let extra = self
            .station
            .as_ref()
            .and_then(|s| s.inputs[1].as_ref())
            .map(|e| e.item.clone());
        let Some(id) = map_id(&result) else {
            return result;
        };
        let new = match extra.as_deref() {
            Some("minecraft:paper") => self.map(id).map(|m| m.scaled()),
            Some("minecraft:glass_pane") => self.map(id).map(|m| m.locked_copy()),
            _ => None,
        };
        if let Some(data) = new {
            let new_id = self.new_map_id(data);
            result.data_mut().map_id = Some(new_id);
        }
        result
    }
    /// The take sound (once per tick).
    pub(super) fn cartography_taken(&mut self, pos: [i32; 3]) {
        let at = [
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]) + 0.5,
            f64::from(pos[2]) + 0.5,
        ];
        self.audio.play(
            "minecraft:ui.cartography_table.take_result",
            Some(at),
            1.0,
            1.0,
        );
    }
    /// `CartographyTableScreen.extractBackground` after the background.
    pub(super) fn draw_cartography(&mut self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let Some((map, extra)) = self
            .station
            .as_ref()
            .map(|s| (s.inputs[0].clone(), s.inputs[1].clone()))
        else {
            return;
        };
        let extra = extra.map(|e| e.item.to_string()).unwrap_or_default();
        let (duplicate, scaling, locking) = (
            extra == "minecraft:map",
            extra == "minecraft:paper",
            extra == "minecraft:glass_pane",
        );
        let id = map.as_ref().and_then(map_id);
        let state = id.and_then(|id| self.map(id).map(|m| (m.locked, m.scale)));
        let mut locked = false;
        let error = "gui/sprites/container/cartography_table/error";
        if let Some((is_locked, scale)) = state {
            if is_locked {
                locked = true;
                if scaling || locking {
                    batch.sprite(&self.gui.atlas, error, ox + 35.0, oy + 31.0);
                }
            }
            if scaling && scale >= 4 {
                locked = true;
                batch.sprite(&self.gui.atlas, error, ox + 35.0, oy + 31.0);
            }
        }
        let sprite = |batch: &mut HudBatch, game: &Self, name: &str, x: f32, y: f32| {
            batch.sprite(
                &game.gui.atlas,
                &format!("gui/sprites/container/cartography_table/{name}"),
                ox + x,
                oy + y,
            );
        };
        let preview = |batch: &mut HudBatch, game: &mut Self, x: f32, y: f32, scale: f32| {
            if let Some(rect) = id
                .filter(|_| state.is_some())
                .and_then(|id| game.map_texture(id))
            {
                let size = 128.0 * scale;
                batch.rect_uv(ox + x, oy + y, size, size, rect, u32::MAX);
            }
        };
        if scaling && !locked {
            sprite(batch, self, "scaled_map", 67.0, 13.0);
            preview(batch, self, 85.0, 31.0, 0.226);
        } else if duplicate {
            sprite(batch, self, "duplicated_map", 83.0, 13.0);
            preview(batch, self, 86.0, 16.0, 0.34);
            sprite(batch, self, "duplicated_map", 67.0, 29.0);
            preview(batch, self, 70.0, 32.0, 0.34);
        } else if locking {
            sprite(batch, self, "map", 67.0, 13.0);
            preview(batch, self, 71.0, 17.0, 0.45);
            sprite(batch, self, "locked", 118.0, 60.0);
        } else {
            sprite(batch, self, "map", 67.0, 13.0);
            preview(batch, self, 71.0, 17.0, 0.45);
        }
    }
}
impl MapData {
    /// `MapItemSavedData.scaled`: a fresh map one scale up around the same centre.
    pub(super) fn scaled(&self) -> MapData {
        let mut m = MapData::fresh(
            f64::from(self.center[0]),
            f64::from(self.center[1]),
            (self.scale + 1).min(4),
            &self.dimension,
        );
        m.tracking = self.tracking;
        m.unlimited = self.unlimited;
        m
    }
    /// `MapItemSavedData.locked`: the same pixels, locked.
    pub(super) fn locked_copy(&self) -> MapData {
        let mut m = MapData::fresh(
            f64::from(self.center[0]),
            f64::from(self.center[1]),
            self.scale,
            &self.dimension,
        );
        m.center = self.center;
        m.colors = self.colors.clone();
        m.tracking = self.tracking;
        m.unlimited = self.unlimited;
        m.locked = true;
        m
    }
}
