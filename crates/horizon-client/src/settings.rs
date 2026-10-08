//! Player settings persisted in `options.txt` with vanilla's keys, and the
//! options screen (sliders and toggles in OptionsScreen's two-column layout).
use crate::hud::{HudAtlas, HudBatch, rgba};
use std::{fs, path::Path};

#[derive(Clone, Debug)]
pub struct Settings {
    /// Degrees, 30..=110 (vanilla stores the slider value).
    pub fov: f32,
    /// Chunks, 2..=32.
    pub render_distance: i32,
    /// 0..1, vanilla default 0.5.
    pub sensitivity: f32,
    /// 0..1.
    pub master_volume: f32,
    /// 0 = auto.
    pub gui_scale: u32,
    pub view_bobbing: bool,
}
impl Default for Settings {
    fn default() -> Self {
        Self { fov: 70.0, render_distance: 12, sensitivity: 0.5, master_volume: 1.0, gui_scale: 0, view_bobbing: true }
    }
}
impl Settings {
    pub fn load(path: &Path) -> Self {
        let mut s = Self::default();
        let Ok(text) = fs::read_to_string(path) else { return s };
        for line in text.lines() {
            let Some((k, v)) = line.split_once(':') else { continue };
            match k {
                // Options.fov is stored as (fov - 70) / 40.
                "fov" => s.fov = v.parse::<f32>().map_or(70.0, |f| (70.0 + f * 40.0).clamp(30.0, 110.0)),
                "renderDistance" => s.render_distance = v.parse().unwrap_or(12).clamp(2, 32),
                "mouseSensitivity" => s.sensitivity = v.parse().unwrap_or(0.5f32).clamp(0.0, 1.0),
                "soundCategory_master" => s.master_volume = v.parse().unwrap_or(1.0f32).clamp(0.0, 1.0),
                "guiScale" => s.gui_scale = v.parse().unwrap_or(0),
                "bobView" => s.view_bobbing = v == "true",
                _ => {}
            }
        }
        s
    }
    pub fn save(&self, path: &Path) {
        let text = format!(
            "version:4793\nfov:{}\nrenderDistance:{}\nmouseSensitivity:{}\nsoundCategory_master:{}\nguiScale:{}\nbobView:{}\n",
            (self.fov - 70.0) / 40.0,
            self.render_distance,
            self.sensitivity,
            self.master_volume,
            self.gui_scale,
            self.view_bobbing
        );
        let _ = fs::write(path, text);
    }
    fn rows(&self) -> Vec<(String, Option<f32>)> {
        vec![
            (format!("FOV: {}", if self.fov.round() == 70.0 { "Normal".into() } else if self.fov >= 110.0 { "Quake Pro".into() } else { format!("{:.0}", self.fov) }), Some((self.fov - 30.0) / 80.0)),
            (format!("Render Distance: {} chunks", self.render_distance), Some((self.render_distance - 2) as f32 / 30.0)),
            (format!("Sensitivity: {}", if self.sensitivity == 0.0 { "*yawn*".into() } else if self.sensitivity >= 1.0 { "HYPERSPEED!!!".into() } else { format!("{:.0}%", self.sensitivity * 200.0) }), Some(self.sensitivity)),
            (format!("Master Volume: {}", if self.master_volume == 0.0 { "OFF".into() } else { format!("{:.0}%", self.master_volume * 100.0) }), Some(self.master_volume)),
            (format!("GUI Scale: {}", if self.gui_scale == 0 { "Auto".into() } else { self.gui_scale.to_string() }), None),
            (format!("View Bobbing: {}", if self.view_bobbing { "ON" } else { "OFF" }), None),
        ]
    }
    fn layout(w: f32, i: usize) -> (f32, f32) {
        (w / 2.0 - 155.0 + (i % 2) as f32 * 160.0, 48.0 + (i / 2) as f32 * 24.0)
    }
    pub fn draw(&self, atlas: &HudAtlas, batch: &mut HudBatch, cursor: (f32, f32)) {
        let (w, h) = (batch.width, batch.height);
        batch.fill(atlas, 0.0, 0.0, w, h, rgba(0, 0, 0, 150));
        let title = "Options";
        let tw = HudBatch::text_width(atlas, title);
        batch.text(atlas, title, (w - tw) / 2.0, 15.0, u32::MAX, true);
        let mut rows = self.rows();
        rows.push(("Done".into(), None));
        for (i, (label, slider)) in rows.iter().enumerate() {
            let (x, y) = if i == rows.len() - 1 { (w / 2.0 - 100.0, h / 6.0 + 168.0) } else { Self::layout(w, i) };
            let width = if i == rows.len() - 1 { 200.0 } else { 150.0 };
            let hover = cursor.0 >= x && cursor.0 < x + width && cursor.1 >= y && cursor.1 < y + 20.0;
            let sprite = match (slider, hover) {
                (Some(_), _) => "gui/sprites/widget/slider",
                (None, true) => "gui/sprites/widget/button_highlighted",
                (None, false) => "gui/sprites/widget/button",
            };
            if let Some(r) = atlas.sprites.get(sprite).copied() {
                let half = width / 2.0;
                let mut left = r;
                left.u1 = r.u0 + (r.u1 - r.u0) * (half / r.w as f32);
                batch.rect_uv(x, y, half, 20.0, left, u32::MAX);
                let mut right = r;
                right.u0 = r.u1 - (r.u1 - r.u0) * (half / r.w as f32);
                batch.rect_uv(x + half, y, half, 20.0, right, u32::MAX);
            }
            if let Some(v) = slider {
                let handle = if hover { "gui/sprites/widget/slider_handle_highlighted" } else { "gui/sprites/widget/slider_handle" };
                batch.sprite_scaled(atlas, handle, x + v.clamp(0.0, 1.0) * (width - 8.0), y, 8.0, 20.0, u32::MAX);
            }
            let tw = HudBatch::text_width(atlas, label);
            batch.text(atlas, label, x + (width - tw) / 2.0, y + 6.0, u32::MAX, true);
        }
    }
    /// Handle a click or drag; returns true when Done was pressed.
    pub fn click(&mut self, x: f32, y: f32, size: (f32, f32), drag: bool) -> bool {
        let (w, h) = size;
        let rows = self.rows().len();
        if !drag && x >= w / 2.0 - 100.0 && x < w / 2.0 + 100.0 && y >= h / 6.0 + 168.0 && y < h / 6.0 + 188.0 {
            return true;
        }
        for i in 0..rows {
            let (bx, by) = Self::layout(w, i);
            if !(x >= bx && x < bx + 150.0 && y >= by && y < by + 20.0) {
                continue;
            }
            let v = ((x - bx - 4.0) / 142.0).clamp(0.0, 1.0);
            match i {
                0 => self.fov = (30.0 + v * 80.0).round(),
                1 => self.render_distance = 2 + (v * 30.0).round() as i32,
                2 => self.sensitivity = (v * 100.0).round() / 100.0,
                3 => self.master_volume = (v * 100.0).round() / 100.0,
                4 if !drag => self.gui_scale = (self.gui_scale + 1) % 5,
                5 if !drag => self.view_bobbing = !self.view_bobbing,
                _ => {}
            }
        }
        false
    }
}
