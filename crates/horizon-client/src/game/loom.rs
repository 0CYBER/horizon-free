//! The loom (`LoomMenu`, `LoomScreen`): stacking banner patterns onto a
//! banner with a dye (and a pattern item for the special patterns), the
//! six-layer limit, the scrolling pattern list and the banner preview.
use super::{
    Game,
    stations::{INPUT, Station},
};
use horizon_client::{
    banner::{self, BannerLayer},
    hud::{HudBatch, Rect, rgba},
    items::ItemStack,
};
use std::sync::Arc;

/// Per-menu loom state: `selectablePatterns`, the selected index and the
/// screen's scroller.
#[derive(Default)]
pub(super) struct LoomState {
    pub(super) patterns: Vec<Arc<str>>,
    pub(super) selected: Option<usize>,
    /// `displayPatterns`: the pattern buttons are shown.
    pub(super) display: bool,
    pub(super) has_max: bool,
    pub(super) scroll: f32,
    pub(super) start_row: usize,
    pub(super) scrolling: bool,
}
impl LoomState {
    fn total_rows(&self) -> usize {
        self.patterns.len().div_ceil(4)
    }
    fn offscreen_rows(&self) -> i32 {
        self.total_rows() as i32 - 4
    }
    fn scroll_active(&self) -> bool {
        self.display && self.patterns.len() > 16
    }
    /// `startRow` for a scroller position.
    fn row_for(&self, offs: f32) -> usize {
        ((offs * self.offscreen_rows() as f32 + 0.5) as i32).max(0) as usize
    }
}

const BUTTON: f32 = 14.0;

fn inside(p: (f32, f32), x: f32, y: f32, w: f32, h: f32) -> bool {
    p.0 >= x && p.1 >= y && p.0 < x + w && p.1 < y + h
}
/// A sub-rectangle of a sprite given in texels.
fn sub_rect(r: Rect, [x, y, w, h]: [f32; 4]) -> Rect {
    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
    Rect {
        u0: r.u0 + x * du,
        v0: r.v0 + y * dv,
        u1: r.u0 + (x + w) * du,
        v1: r.v0 + (y + h) * dv,
        w: w as u32,
        h: h as u32,
    }
}
/// A dye's tint as a HUD vertex colour.
pub(super) fn dye_color(name: &str) -> u32 {
    let c = banner::dye_rgb(name);
    rgba((c >> 16) as u8, (c >> 8) as u8, c as u8, 255)
}

impl Game<'_> {
    /// `LoomMenu.slotsChanged` and the screen's `containerChanged`.
    pub(super) fn loom_changed(&self, s: &mut Station) {
        let banners = &self.res.stations.banners;
        let items = &self.res.items;
        let (banner_in, dye_in) = (s.inputs[0].as_ref(), s.inputs[1].as_ref());
        let l = &mut s.loom;
        let mut result = None;
        if let (Some(banner_stack), Some(dye_stack)) = (banner_in, dye_in) {
            let valid = l.selected.is_some_and(|i| i < l.patterns.len());
            let previous = std::mem::take(&mut l.patterns);
            l.patterns = banners.selectable(s.inputs[2].as_ref());
            let shown = if l.patterns.len() == 1 {
                l.selected = Some(0);
                Some(l.patterns[0].clone())
            } else if !valid {
                l.selected = None;
                None
            } else {
                let value = previous[l.selected.unwrap_or(0)].clone();
                match l.patterns.iter().position(|p| *p == value) {
                    Some(i) => {
                        l.selected = Some(i);
                        Some(value)
                    }
                    None => {
                        l.selected = None;
                        None
                    }
                }
            };
            l.has_max = banner::has_max_layers(banner_stack);
            if let Some(pattern) = shown {
                if l.has_max {
                    l.selected = None;
                } else if let Some(dye) = banners.loom_dye(items, dye_stack) {
                    result = Some(banner::loom_result(banner_stack, &dye, &pattern));
                }
            }
            l.display = !l.has_max && !l.patterns.is_empty();
        } else {
            l.patterns.clear();
            l.selected = None;
            l.has_max = banner_in.is_some_and(banner::has_max_layers);
            l.display = false;
        }
        if l.start_row >= l.total_rows() {
            l.start_row = 0;
            l.scroll = 0.0;
        }
        s.result = result;
    }
    /// The loom slots' `mayPlace`.
    pub(super) fn loom_slot_rules(&self, input: usize, stack: &ItemStack) -> (bool, u32) {
        let banners = &self.res.stations.banners;
        let ok = match input {
            0 => stack.item.ends_with("_banner"),
            1 => banners.loom_dye(&self.res.items, stack).is_some(),
            _ => banners.is_pattern_item(&self.res.items, stack),
        };
        (ok, 64)
    }
    /// The result slot's `onTake`: one banner and one dye are used up.
    pub(super) fn loom_taken(&mut self, s: &mut Station) {
        for slot in &mut s.inputs[..2] {
            if let Some(x) = slot.as_mut() {
                x.count -= 1;
                if x.count == 0 {
                    *slot = None;
                }
            }
        }
        if s.inputs[0].is_none() || s.inputs[1].is_none() {
            s.loom.selected = None;
        }
        if s.last_sound != self.ticks {
            s.last_sound = self.ticks;
            let pos = s.pos;
            let at = [
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ];
            self.audio
                .play("minecraft:ui.loom.take_result", Some(at), 1.0, 1.0);
        }
    }
    /// `LoomMenu.quickMoveStack` from an inventory slot.
    pub(super) fn loom_quick_move(&mut self, slot: usize, stack: &mut ItemStack) -> bool {
        let banners = &self.res.stations.banners;
        let items = &self.res.items;
        let target = if stack.item.ends_with("_banner") {
            Some(INPUT)
        } else if banners.loom_dye(items, stack).is_some() {
            Some(INPUT + 1)
        } else if banners.is_pattern_item(items, stack) {
            Some(INPUT + 2)
        } else {
            None
        };
        match target {
            Some(t) => self.move_to(stack, &[t], false),
            None if (9..36).contains(&slot) => {
                let hotbar: Vec<usize> = (0..9).collect();
                self.move_to(stack, &hotbar, false)
            }
            None if slot < 9 => {
                let main: Vec<usize> = (9..36).collect();
                self.move_to(stack, &main, false)
            }
            None => false,
        }
    }
    /// A left click inside the loom's pattern grid or scroller.
    pub(super) fn loom_click(&mut self, local: (f32, f32)) -> bool {
        let Some(s) = self.station.as_ref() else {
            return false;
        };
        let l = &s.loom;
        if !l.display {
            return false;
        }
        for row in 0..4 {
            for column in 0..4 {
                let index = (row + l.start_row) * 4 + column;
                let (x, y) = (60.0 + column as f32 * BUTTON, 13.0 + row as f32 * BUTTON);
                if index < l.patterns.len() && inside(local, x, y, BUTTON, BUTTON) {
                    if let Some(s) = self.station.as_mut() {
                        s.loom.selected = Some(index);
                    }
                    self.audio
                        .play("minecraft:ui.loom.select_pattern", None, 1.0, 1.0);
                    self.station_changed();
                    return true;
                }
            }
        }
        if inside(local, 119.0, 9.0, 12.0, 56.0)
            && let Some(s) = self.station.as_mut()
        {
            s.loom.scrolling = true;
            return true;
        }
        false
    }
    /// `LoomScreen.mouseDragged` on the scroller.
    pub(super) fn loom_drag(&mut self) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let (_, oy) = self.inventory_origin(&HudBatch::new(size));
        let cursor_y = self.cursor.1;
        let Some(l) = self.station.as_mut().map(|s| &mut s.loom) else {
            return;
        };
        if l.scrolling && l.display && l.offscreen_rows() > 0 {
            let top = oy + 13.0;
            l.scroll = ((cursor_y - top - 7.5) / (56.0 - 15.0)).clamp(0.0, 1.0);
            l.start_row = l.row_for(l.scroll);
        }
    }
    pub(super) fn loom_release(&mut self) {
        if let Some(s) = self.station.as_mut() {
            s.loom.scrolling = false;
        }
    }
    /// `LoomScreen.mouseScrolled`.
    pub(super) fn loom_scroll(&mut self, d: f32) {
        let Some(l) = self.station.as_mut().map(|s| &mut s.loom) else {
            return;
        };
        let offscreen = l.offscreen_rows();
        if l.display && offscreen > 0 {
            l.scroll = (l.scroll - d / offscreen as f32).clamp(0.0, 1.0);
            l.start_row = l.row_for(l.scroll);
        }
    }
    /// A banner flag's front face (`BannerFlagModel`, 20x40 texels at (1, 1))
    /// drawn flat: the cloth, the base colour and every layer.
    pub(super) fn draw_flag(
        &self,
        batch: &mut HudBatch,
        rect: [f32; 4],
        base: &str,
        layers: &[BannerLayer],
    ) {
        let banners = &self.res.stations.banners;
        let mut paint = |name: &str, color: u32| {
            if let Some(r) = self.res.atlas.sprites.get(name).copied() {
                let uv = sub_rect(r, [1.0, 1.0, 20.0, 40.0]);
                batch.rect_uv(rect[0], rect[1], rect[2], rect[3], uv, color);
            }
        };
        paint("entity/banner/banner_base", u32::MAX);
        paint("entity/banner/base", dye_color(base));
        for layer in layers.iter().take(16) {
            paint(
                &banners.banner_texture(&layer.pattern),
                dye_color(&layer.color),
            );
        }
    }
    /// `LoomScreen.extractBackground` after the background; returns the
    /// pattern name tooltip of the hovered button.
    pub(super) fn draw_loom(
        &mut self,
        batch: &mut HudBatch,
        ox: f32,
        oy: f32,
        local: (f32, f32),
    ) -> Option<Vec<(String, u32)>> {
        let s = self.station.as_ref()?;
        let l = &s.loom;
        let atlas = &self.res.atlas;
        for (i, icon) in ["banner", "dye", "banner_pattern"].iter().enumerate() {
            if s.inputs[i].is_none() {
                let pos = [(13.0, 26.0), (33.0, 26.0), (23.0, 45.0)][i];
                batch.sprite(
                    atlas,
                    &format!("gui/sprites/container/slot/{icon}"),
                    ox + pos.0,
                    oy + pos.1,
                );
            }
        }
        let scroller = if l.scroll_active() {
            "scroller"
        } else {
            "scroller_disabled"
        };
        batch.sprite(
            atlas,
            &format!("gui/sprites/container/loom/{scroller}"),
            ox + 119.0,
            oy + 13.0 + (41.0 * l.scroll).floor(),
        );
        let result = s.result.clone();
        let (display, has_max, start_row, selected) =
            (l.display, l.has_max, l.start_row, l.selected);
        let patterns = l.patterns.clone();
        let dye = s.inputs[1]
            .as_ref()
            .and_then(|d| self.res.stations.banners.loom_dye(&self.res.items, d))
            .unwrap_or_else(|| Arc::from("white"));
        if let Some(result) = result.filter(|_| !has_max)
            && let Some(base) = banner::banner_color(&result.item)
        {
            self.draw_flag(
                batch,
                [ox + 141.0, oy + 8.0, 20.0, 40.0],
                base,
                banner::layers(&result),
            );
        } else if has_max {
            batch.sprite(
                &self.res.atlas,
                "gui/sprites/container/loom/error",
                ox + 138.0,
                oy + 52.0,
            );
        }
        let mut tooltip = None;
        if display {
            let atlas = &self.res.atlas;
            for row in 0..4 {
                for column in 0..4 {
                    let index = (row + start_row) * 4 + column;
                    let Some(pattern) = patterns.get(index) else {
                        return tooltip;
                    };
                    let (x, y) = (60.0 + column as f32 * BUTTON, 13.0 + row as f32 * BUTTON);
                    let hover = inside(local, x, y, BUTTON, BUTTON);
                    let sprite = if selected == Some(index) {
                        "pattern_selected"
                    } else if hover {
                        let key = self
                            .res
                            .stations
                            .banners
                            .layer_key(&BannerLayer::new(pattern, &dye));
                        tooltip = Some(vec![(self.translate(&key, &key), u32::MAX)]);
                        "pattern_highlighted"
                    } else {
                        "pattern"
                    };
                    batch.sprite(
                        atlas,
                        &format!("gui/sprites/container/loom/{sprite}"),
                        ox + x,
                        oy + y,
                    );
                    // extractBannerOnButton: a 5x10 grey cloth with the pattern.
                    batch.fill(
                        atlas,
                        ox + x + 4.0,
                        oy + y + 2.0,
                        5.0,
                        10.0,
                        dye_color("gray"),
                    );
                    let texture = self.res.stations.banners.banner_texture(pattern);
                    if let Some(r) = atlas.sprites.get(&texture).copied() {
                        let uv = sub_rect(r, [0.0, 1.0, 21.0, 40.0]);
                        batch.rect_uv(ox + x + 4.0, oy + y + 2.0, 5.0, 10.0, uv, u32::MAX);
                    }
                }
            }
        }
        tooltip
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroller_rows_follow_the_offset() {
        let l = LoomState {
            patterns: (0..40)
                .map(|i| Arc::from(format!("p{i}").as_str()))
                .collect(),
            display: true,
            ..Default::default()
        };
        // 40 patterns = 10 rows, 6 off screen.
        assert_eq!(l.offscreen_rows(), 6);
        assert!(l.scroll_active());
        assert_eq!(l.row_for(0.0), 0);
        assert_eq!(l.row_for(1.0), 6);
        assert_eq!(l.row_for(0.5), 3);
    }
    #[test]
    fn few_patterns_do_not_scroll() {
        let l = LoomState {
            patterns: (0..16)
                .map(|i| Arc::from(format!("p{i}").as_str()))
                .collect(),
            display: true,
            ..Default::default()
        };
        assert!(!l.scroll_active());
        assert_eq!(l.offscreen_rows(), 0);
    }
}
