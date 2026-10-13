//! Horizon's graphics settings screens, Simple and Advanced: which settings
//! they show, in what groups, with what description and cost, and their
//! input. How they look comes from `graphics_style`.
use crate::{
    graphics::{Overlay, Pacing, Preset, PresentMode},
    graphics_style::{Cost, STYLE},
    hud::{HudAtlas, HudBatch},
    options_ui::Opt,
    settings::Settings,
    text::Text,
    ui,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Level {
    Simple,
    Advanced,
}

/// Horizon's own settings (`graphics::Graphics`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum H {
    Preset,
    CaveCulling,
    ChunkUpdates,
    UploadBudget,
    Sky,
    Fog,
    Weather,
    AnimatedTextures,
    PresentMode,
    Pacing,
    Workers,
    Overlay,
    ResolutionScale,
    Antialiasing,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Item {
    Vanilla(Opt),
    Horizon(H),
}

struct Setting {
    item: Item,
    cost: Cost,
    text: &'static str,
}
enum Row {
    Header(&'static str),
    Setting(Setting),
}
fn v(opt: Opt, cost: Cost, text: &'static str) -> Row {
    Row::Setting(Setting {
        item: Item::Vanilla(opt),
        cost,
        text,
    })
}
fn h(opt: H, cost: Cost, text: &'static str) -> Row {
    Row::Setting(Setting {
        item: Item::Horizon(opt),
        cost,
        text,
    })
}

fn rows(level: Level) -> Vec<Row> {
    use Cost::*;
    let render = v(
        Opt::RenderDistance,
        High,
        "How far terrain is drawn, in chunks. The biggest lever on frame time.",
    );
    let fov = v(Opt::Fov, Low, "Field of view. Wider shows more terrain at once.");
    let fps = v(
        Opt::MaxFps,
        None,
        "Caps frames per second; a cap saves power and heat.",
    );
    let vsync = v(
        Opt::Vsync,
        None,
        "Waits for the display's refresh: no tearing, a little latency.",
    );
    let gui = v(Opt::GuiScale, None, "Size of menus and the HUD.");
    let preset = h(
        H::Preset,
        None,
        "Sets the options below together. Changing one makes it Custom.",
    );
    match level {
        Level::Simple => vec![preset, render, fov, fps, vsync, gui],
        Level::Advanced => vec![
            Row::Header("Rendering"),
            render,
            v(
                Opt::SimulationDistance,
                Medium,
                "How far mobs, crops and machines keep working. CPU time.",
            ),
            v(
                Opt::EntityDistance,
                Medium,
                "How far away mobs and items are drawn, as a share of normal.",
            ),
            h(
                H::CaveCulling,
                None,
                "Skips terrain hidden behind closed sections. Off costs a lot.",
            ),
            h(
                H::ChunkUpdates,
                Low,
                "Meshed sections uploaded per frame. Lower is smoother while loading.",
            ),
            Row::Header("Quality"),
            v(
                Opt::SmoothLighting,
                Low,
                "Soft shadows in corners. Costs chunk building time only.",
            ),
            v(
                Opt::Mipmaps,
                Low,
                "Smaller copies of block textures for distant terrain: less shimmer.",
            ),
            v(
                Opt::BiomeBlend,
                Low,
                "Blends grass and water colours across biomes when building chunks.",
            ),
            v(
                Opt::Clouds,
                Medium,
                "Cloud layer: off, flat (fast) or 3D (fancy).",
            ),
            v(Opt::CloudRange, Low, "How far clouds reach, in chunks."),
            h(H::Sky, Low, "Draws the sky, sun, moon and stars."),
            h(
                H::Fog,
                None,
                "Distance fog. Off shows the edge of the loaded world.",
            ),
            h(H::Weather, Low, "Draws rain and snow."),
            v(Opt::WeatherRadius, Low, "How far rain and snow fall around you."),
            v(
                Opt::Particles,
                Medium,
                "How many particles effects spawn.",
            ),
            v(Opt::EntityShadows, Low, "Round shadows under mobs and items."),
            v(
                Opt::CutoutLeaves,
                Medium,
                "Leaves with see-through gaps. Off draws them solid.",
            ),
            h(
                H::AnimatedTextures,
                Low,
                "Animates water, lava, fire and other moving textures.",
            ),
            Row::Header("Display"),
            fov,
            fps,
            vsync,
            h(
                H::ResolutionScale,
                High,
                "World pixels per screen pixel. Below 100% is faster on the GPU.",
            ),
            h(
                H::Antialiasing,
                Low,
                "FXAA smooths jagged edges of the world; slightly softer image.",
            ),
            h(
                H::PresentMode,
                None,
                "How frames reach the screen. Auto follows VSync.",
            ),
            h(
                H::Pacing,
                None,
                "Efficient sleeps until the next frame; Precise spins, like vanilla.",
            ),
            v(Opt::Fullscreen, None, "Fills the screen (borderless)."),
            gui,
            Row::Header("Performance"),
            h(
                H::Workers,
                None,
                "Threads that generate, light and mesh chunks. Next start.",
            ),
            h(
                H::UploadBudget,
                Low,
                "Mesh data sent to the GPU per frame. Lower is smoother while loading.",
            ),
            Row::Header("Overlay"),
            h(
                H::Overlay,
                None,
                "Frame rate and frame times over the game, with a graph.",
            ),
        ],
    }
}

const UPLOAD_STEPS: [u32; 8] = [0, 256, 512, 1024, 2048, 4096, 8192, 16384];

fn on_off(on: bool) -> &'static str {
    if on { "On" } else { "Off" }
}
fn labelled(caption: &str, value: impl std::fmt::Display) -> Text {
    Text::literal(format!("{caption}: {value}"))
}

impl H {
    fn slider(self, s: &Settings) -> Option<f32> {
        let g = &s.horizon;
        match self {
            H::ChunkUpdates => Some(g.chunk_updates as f32 / 64.0),
            H::UploadBudget => {
                let i = UPLOAD_STEPS
                    .iter()
                    .position(|&b| b >= g.upload_budget_kib)
                    .unwrap_or(UPLOAD_STEPS.len() - 1);
                Some(i as f32 / (UPLOAD_STEPS.len() - 1) as f32)
            }
            H::Workers => Some(g.workers as f32 / max_workers() as f32),
            H::ResolutionScale => Some((g.resolution_scale as f32 - 50.0) / 150.0),
            _ => None,
        }
    }
    fn label(self, s: &Settings) -> Text {
        let g = &s.horizon;
        match self {
            H::Preset => labelled("Preset", g.preset.label()),
            H::CaveCulling => labelled("Cave Culling", on_off(g.cave_culling)),
            H::ChunkUpdates => match g.chunk_updates {
                0 => labelled("Chunk Updates", "Unlimited"),
                n => labelled("Chunk Updates", format!("{n} per frame")),
            },
            H::UploadBudget => match g.upload_budget_kib {
                0 => labelled("Upload Budget", "Unlimited"),
                k if k >= 1024 => labelled("Upload Budget", format!("{} MiB/frame", k / 1024)),
                k => labelled("Upload Budget", format!("{k} KiB/frame")),
            },
            H::Sky => labelled("Sky", on_off(g.sky)),
            H::Fog => labelled("Fog", on_off(g.fog)),
            H::Weather => labelled("Weather", on_off(g.weather)),
            H::AnimatedTextures => labelled("Animated Textures", on_off(g.animated_textures)),
            H::PresentMode => labelled("Present Mode", g.present_mode.label()),
            H::Pacing => labelled(
                "Frame Pacing",
                match g.pacing {
                    Pacing::Efficient => "Efficient",
                    Pacing::Precise => "Precise",
                },
            ),
            H::Workers => match g.workers {
                0 => labelled("Worker Threads", format!("Auto ({})", g.worker_threads())),
                n => labelled("Worker Threads", n),
            },
            H::ResolutionScale => labelled("Resolution Scale", format!("{}%", g.resolution_scale)),
            H::Antialiasing => labelled("Anti-aliasing", if g.fxaa { "FXAA" } else { "Off" }),
            H::Overlay => labelled(
                "Performance Overlay",
                match g.overlay {
                    Overlay::Off => "Off",
                    Overlay::Fps => "FPS",
                    Overlay::Graph => "FPS and Graph",
                },
            ),
        }
    }
    fn set_slider(self, s: &mut Settings, v: f32) {
        let g = &mut s.horizon;
        match self {
            H::ChunkUpdates => g.chunk_updates = (v * 64.0).round() as u32,
            H::UploadBudget => {
                g.upload_budget_kib =
                    UPLOAD_STEPS[(v * (UPLOAD_STEPS.len() - 1) as f32).round() as usize]
            }
            H::Workers => g.workers = (v * max_workers() as f32).round() as u32,
            H::ResolutionScale => g.resolution_scale = (50.0 + (v * 30.0).round() * 5.0) as u32,
            _ => {}
        }
    }
    fn cycle(self, s: &mut Settings, back: bool) {
        fn step<T: Copy + PartialEq>(all: &[T], now: T, back: bool) -> T {
            let i = all.iter().position(|x| *x == now).unwrap_or(0);
            let n = all.len();
            all[if back { (i + n - 1) % n } else { (i + 1) % n }]
        }
        if self == H::Preset {
            let preset = step(&Preset::ALL, s.horizon.preset, back);
            crate::graphics::apply_preset(preset, s);
            s.horizon.preset = preset;
            return;
        }
        let g = &mut s.horizon;
        match self {
            H::CaveCulling => g.cave_culling = !g.cave_culling,
            H::Sky => g.sky = !g.sky,
            H::Fog => g.fog = !g.fog,
            H::Weather => g.weather = !g.weather,
            H::AnimatedTextures => g.animated_textures = !g.animated_textures,
            H::PresentMode => g.present_mode = step(&PresentMode::ALL, g.present_mode, back),
            H::Antialiasing => g.fxaa = !g.fxaa,
            H::Pacing => {
                g.pacing = match g.pacing {
                    Pacing::Efficient => Pacing::Precise,
                    Pacing::Precise => Pacing::Efficient,
                }
            }
            H::Overlay => {
                g.overlay = step(&[Overlay::Off, Overlay::Fps, Overlay::Graph], g.overlay, back)
            }
            _ => {}
        }
    }
}
fn max_workers() -> u32 {
    std::thread::available_parallelism().map_or(8, |n| n.get() as u32)
}

impl Item {
    fn slider(self, s: &Settings) -> Option<f32> {
        match self {
            Item::Vanilla(o) => o.slider_position(s),
            Item::Horizon(h) => h.slider(s),
        }
    }
    fn label(self, s: &Settings) -> Text {
        match self {
            // The vanilla button shows the scale alone, under its own caption.
            Item::Vanilla(Opt::GuiScale) => Text::translate(
                "options.generic_value",
                &[Text::translate("options.guiScale", &[]), Opt::GuiScale.label(s)],
            ),
            Item::Vanilla(o) => o.label(s),
            Item::Horizon(h) => h.label(s),
        }
    }
    fn set_slider(self, s: &mut Settings, v: f32) {
        match self {
            Item::Vanilla(o) => o.set_slider(s, v),
            Item::Horizon(h) => h.set_slider(s, v),
        }
        self.touched(s);
    }
    fn cycle(self, s: &mut Settings, back: bool) {
        match self {
            Item::Vanilla(o) => o.cycle(s, back),
            Item::Horizon(h) => h.cycle(s, back),
        }
        self.touched(s);
    }
    /// A preset no longer describes the settings once one of them changes.
    fn touched(self, s: &mut Settings) {
        let presetless = matches!(
            self,
            Item::Horizon(H::Preset | H::Overlay | H::Pacing | H::PresentMode | H::Workers)
                | Item::Vanilla(Opt::GuiScale | Opt::Fullscreen | Opt::Vsync | Opt::MaxFps | Opt::Fov)
        );
        if !presetless {
            s.horizon.preset = Preset::Custom;
        }
    }
}

/// What the options screens must do after an interaction here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    Changed,
    OpenAdvanced,
    OpenSimple,
    Done,
}

#[derive(Clone, Copy)]
enum Target {
    Item(Item),
    Button(Action),
}
struct Widget {
    x: f32,
    y: f32,
    w: f32,
    target: Target,
    /// In the scrolling list (clipped to it).
    list: bool,
}

#[derive(Default)]
pub struct GraphicsUi {
    scroll: f32,
    dragging: Option<Item>,
}

impl GraphicsUi {
    pub fn reset(&mut self) {
        self.scroll = 0.0;
        self.dragging = None;
    }
    fn bounds(h: f32) -> (f32, f32) {
        (STYLE.band, h - STYLE.band)
    }
    /// Widgets and the list's content height.
    fn layout(&self, level: Level, w: f32, h: f32) -> (Vec<Widget>, Vec<(f32, &'static str)>, f32) {
        let (top, _) = Self::bounds(h);
        let x = STYLE.row_x(w);
        let mut y = top + 4.0 - self.scroll;
        let start = y;
        let mut widgets = Vec::new();
        let mut headers = Vec::new();
        for row in rows(level) {
            match row {
                Row::Header(name) => {
                    headers.push((y, name));
                    y += STYLE.header_height;
                }
                Row::Setting(s) => {
                    widgets.push(Widget {
                        x,
                        y,
                        w: STYLE.control_width,
                        target: Target::Item(s.item),
                        list: true,
                    });
                    y += STYLE.row_height;
                }
            }
        }
        let content = y - start + 4.0;
        let cx = (w / 2.0).floor();
        let by = h - 27.0;
        let other = match level {
            Level::Simple => Action::OpenAdvanced,
            Level::Advanced => Action::OpenSimple,
        };
        widgets.push(Widget {
            x: cx - 155.0,
            y: by,
            w: 150.0,
            target: Target::Button(other),
            list: false,
        });
        widgets.push(Widget {
            x: cx + 5.0,
            y: by,
            w: 150.0,
            target: Target::Button(Action::Done),
            list: false,
        });
        (widgets, headers, content)
    }
    fn clamp_scroll(&mut self, h: f32, content: f32) {
        let (top, bottom) = Self::bounds(h);
        self.scroll = self.scroll.clamp(0.0, (content - (bottom - top)).max(0.0));
    }
    pub fn draw(
        &mut self,
        level: Level,
        s: &Settings,
        batch: &mut HudBatch,
        atlas: &HudAtlas,
        cursor: (f32, f32),
        in_world: bool,
    ) {
        let (w, h) = (batch.width, batch.height);
        let (_, _, content) = self.layout(level, w, h);
        self.clamp_scroll(h, content);
        let (widgets, headers, _) = self.layout(level, w, h);
        let (top, bottom) = Self::bounds(h);
        STYLE.background(batch, atlas, top, bottom, in_world);
        let shown = |y: f32, height: f32| y >= top && y + height <= bottom;
        let x = STYLE.row_x(w);
        for (y, name) in headers {
            if shown(y, STYLE.header_height) {
                STYLE.header(batch, atlas, &Text::literal(name), x, y);
            }
        }
        let descriptions: Vec<(Cost, &'static str)> = rows(level)
            .into_iter()
            .filter_map(|r| match r {
                Row::Setting(s) => Some((s.cost, s.text)),
                Row::Header(_) => None,
            })
            .collect();
        let mut index = 0;
        for wd in &widgets {
            let hover = ui::hovered(cursor, wd.x, wd.y, wd.w, 20.0)
                && (!wd.list || (cursor.1 >= top && cursor.1 < bottom));
            match wd.target {
                Target::Item(item) => {
                    let (cost, text) = descriptions[index];
                    index += 1;
                    if !shown(wd.y, 20.0) {
                        continue;
                    }
                    let label = item.label(s);
                    match item.slider(s) {
                        Some(value) => STYLE.slider(
                            batch,
                            atlas,
                            (wd.x, wd.y, wd.w),
                            &label,
                            value,
                            hover || self.dragging == Some(item),
                        ),
                        None => STYLE.button(batch, atlas, (wd.x, wd.y, wd.w), &label, hover),
                    }
                    STYLE.description(
                        batch,
                        atlas,
                        wd.x + wd.w + STYLE.gap,
                        wd.y + 1.0,
                        cost,
                        text,
                    );
                }
                Target::Button(action) => {
                    let label = match action {
                        Action::OpenAdvanced => "Advanced...",
                        Action::OpenSimple => "Simple...",
                        _ => "Done",
                    };
                    STYLE.button(batch, atlas, (wd.x, wd.y, wd.w), &Text::literal(label), hover);
                }
            }
        }
        STYLE.frame(batch, atlas, top, bottom, in_world);
        ui::scrollbar(
            batch,
            atlas,
            x + STYLE.control_width + STYLE.gap + STYLE.text_width(w) + 6.0,
            top,
            bottom,
            content,
            self.scroll,
        );
        let title = match level {
            Level::Simple => "Graphics",
            Level::Advanced => "Graphics: Advanced",
        };
        STYLE.title(batch, atlas, &Text::literal(title));
    }
    /// A press (or a drag with `drag`) at `(x, y)`.
    pub fn click(
        &mut self,
        level: Level,
        s: &mut Settings,
        (x, y): (f32, f32),
        (w, h): (f32, f32),
        drag: bool,
        right: bool,
    ) -> Vec<Action> {
        let (widgets, _, _) = self.layout(level, w, h);
        let (top, bottom) = Self::bounds(h);
        if drag {
            if let Some(item) = self.dragging
                && let Some(wd) = widgets
                    .iter()
                    .find(|wd| matches!(wd.target, Target::Item(i) if i == item))
            {
                item.set_slider(s, ui::slider_value(wd.x, wd.w, x));
                return vec![Action::Changed];
            }
            return Vec::new();
        }
        for wd in widgets {
            if !ui::hovered((x, y), wd.x, wd.y, wd.w, 20.0) {
                continue;
            }
            if wd.list && (y < top || y >= bottom || wd.y < top || wd.y + 20.0 > bottom) {
                continue;
            }
            return match wd.target {
                Target::Button(action) => vec![action],
                Target::Item(item) => {
                    if item.slider(s).is_some() {
                        self.dragging = Some(item);
                        item.set_slider(s, ui::slider_value(wd.x, wd.w, x));
                    } else {
                        item.cycle(s, right);
                    }
                    vec![Action::Changed]
                }
            };
        }
        Vec::new()
    }
    pub fn release(&mut self) {
        self.dragging = None;
    }
    pub fn scroll(&mut self, amount: f32) {
        self.scroll -= amount * 18.0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_advanced_setting_has_a_description() {
        for level in [Level::Simple, Level::Advanced] {
            for row in rows(level) {
                if let Row::Setting(s) = row {
                    assert!(!s.text.is_empty(), "{:?}", s.item);
                    assert!(s.text.len() < 80, "one line: {:?}", s.item);
                }
            }
        }
    }

    #[test]
    fn presets_set_vanilla_and_horizon_options() {
        let mut s = Settings::default();
        Item::Horizon(H::Preset).cycle(&mut s, false);
        assert_eq!(s.horizon.preset, Preset::Performance);
        assert_eq!(s.render_distance, 8);
        assert_eq!(s.clouds, "false");
        // Any quality change leaves the preset.
        Item::Vanilla(Opt::Clouds).cycle(&mut s, false);
        assert_eq!(s.horizon.preset, Preset::Custom);
        // Display preferences keep it.
        Item::Horizon(H::Preset).cycle(&mut s, true);
        Item::Vanilla(Opt::Vsync).cycle(&mut s, false);
        assert_eq!(s.horizon.preset, Preset::Ultra);
    }

    #[test]
    fn upload_budget_slider_steps() {
        let mut s = Settings::default();
        H::UploadBudget.set_slider(&mut s, 1.0);
        assert_eq!(s.horizon.upload_budget_kib, 16384);
        assert_eq!(H::UploadBudget.slider(&s), Some(1.0));
        H::UploadBudget.set_slider(&mut s, 0.0);
        assert_eq!(s.horizon.upload_budget_kib, 0);
    }
}
