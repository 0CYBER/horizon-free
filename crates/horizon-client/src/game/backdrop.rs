//! The live title backdrop (Menu style: Horizon): a frozen landscape that
//! Horizon generates and renders itself behind the title screens, seen
//! through a slow orbiting camera, falling snow, an overcast sky and the cold
//! blur and grade of `gfx::PostGrade`. Nothing is bundled: the chunks come
//! from the vanilla generator for a fixed seed and are cached in
//! `saves/.horizon-title/backdrop` (no level.dat, so no world list entry).
//! The ice-spikes spot is found once with `/locate biome`'s search and kept
//! in `spot.txt` there. The generator is built and the spot found on a
//! worker, so the title keeps drawing its panorama until both are ready.
use super::Game;
use horizon_client::{
    Result,
    save::{Persistent, SaveDir},
    streaming::Streaming,
    vanillagen::VanillaTerrain,
    world::{Weather, World},
};
use horizon_core::biome::{BiomeZoom, obfuscate_seed};
use std::{
    sync::{Arc, Mutex},
    time::Instant,
};

/// The world the backdrop is cut from (HORIZON_BACKDROP_SEED overrides it).
const SEED: i64 = 2_026_031_337;
/// The biome the camera looks for.
const BIOME: &str = "minecraft:ice_spikes";
/// Late morning: the snow reads clearly; the grade (`PostGrade::FROST`) makes it cold.
const DAY_TIME: i64 = 4_000;

/// The worker's result: the generator for [`SEED`], or why it failed.
type Built = std::result::Result<Arc<VanillaTerrain>, String>;

#[derive(Default)]
pub(super) struct Backdrop {
    pub active: bool,
    /// The generator, once the worker has built it; taken when it is installed.
    terrain: Arc<Mutex<Option<Built>>>,
    /// The generator streams the backdrop's chunks.
    installed: bool,
    /// The search for the spot, then the spot (block x, z).
    spot: Arc<Mutex<Option<Option<(i32, i32)>>>>,
    centred: bool,
    /// Camera position once the ground under it has loaded.
    pub camera: Option<glam::DVec3>,
    pub ready: Option<Instant>,
    pub started: Option<Instant>,
}
fn seed() -> i64 {
    std::env::var("HORIZON_BACKDROP_SEED")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(SEED)
}
impl Game<'_> {
    /// How visible the live backdrop is (0 until its first chunks are drawn).
    pub(super) fn backdrop_fade(&self) -> f32 {
        let Some(at) = self.backdrop.ready else {
            return 0.0;
        };
        let x = (at.elapsed().as_secs_f32() / 1.6).clamp(0.0, 1.0);
        x * x * (3.0 - 2.0 * x)
    }
    /// Whether the frame should draw the backdrop world instead of the panorama.
    pub(super) fn backdrop_live(&self) -> bool {
        self.title.is_some() && self.backdrop.active && self.backdrop.camera.is_some()
    }
    /// The orbiting camera: position, yaw and pitch.
    pub(super) fn backdrop_view(&self) -> Option<(glam::DVec3, f32, f32)> {
        let c = self.backdrop.camera?;
        let t = self
            .backdrop
            .started
            .map_or(0.0, |s| s.elapsed().as_secs_f32());
        // A full turn in about six minutes.
        let yaw = (200.0 + t * 1.0) % 360.0;
        let pitch = 6.0 + (t * 0.05).sin() * 2.0;
        Some((c, yaw, pitch))
    }
    /// Called every frame while the title is up.
    pub(super) fn update_backdrop(&mut self) -> Result<()> {
        let wanted = self.pending_entry.is_none()
            && self.title.is_some()
            && horizon_client::ui_theme::horizon()
            && self.world_renderer_ready
            && self.renderer.is_some()
            && self.options.frames.is_none();
        if !wanted {
            if self.backdrop.active && self.title.is_some() {
                self.stop_backdrop();
            }
            if self.title.is_none() && self.backdrop.active {
                // open_world / connect_server replaced the world and streaming.
                self.backdrop = Backdrop::default();
            }
            return Ok(());
        }
        if !self.backdrop.active {
            self.start_backdrop()?;
        }
        if !self.backdrop.installed {
            let built = self
                .backdrop
                .terrain
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .take();
            match built {
                // Still building: the panorama stays up.
                None => return Ok(()),
                Some(Ok(terrain)) => self.install_backdrop(terrain)?,
                Some(Err(e)) => {
                    // The panorama stays up for good.
                    log::warn!("Horizon: title backdrop: {e}");
                    self.backdrop.installed = true;
                    return Ok(());
                }
            }
        }
        let found = *self.backdrop.spot.lock().unwrap_or_else(|e| e.into_inner());
        let Some(Some((x, z))) = found else {
            return Ok(());
        };
        if !self.backdrop.centred {
            self.backdrop.centred = true;
            self.streaming.center = (x >> 4, z >> 4);
            self.streaming.paused = false;
        }
        self.update_world();
        if self.backdrop.camera.is_none()
            && (-2..=2)
                .all(|dz| (-2..=2).all(|dx| self.streaming.is_lit((x >> 4) + dx, (z >> 4) + dz)))
        {
            // Above the tallest block nearby, so no spike pokes through the lens.
            let mut top = 64;
            for dz in (-12..=12).step_by(3) {
                for dx in (-12..=12).step_by(3) {
                    if let Some(y) = self.surface_y(x + dx, z + dz) {
                        top = top.max(y);
                    }
                }
            }
            let ground = self.surface_y(x, z).unwrap_or(top);
            let y = (ground + 22).max(top + 4);
            self.backdrop.camera = Some(glam::DVec3::new(
                f64::from(x) + 0.5,
                f64::from(y),
                f64::from(z) + 0.5,
            ));
        }
        if self.backdrop.camera.is_some()
            && self.backdrop.ready.is_none()
            && self
                .renderer
                .as_ref()
                .is_some_and(|r| r.section_count() > 40)
        {
            self.backdrop.ready = Some(Instant::now());
            log::info!(
                "Horizon: title backdrop ready; world rest ready: {}",
                self.res.rest.get().is_some()
            );
        }
        Ok(())
    }
    /// The world tick of the backdrop: time passes for clouds and snow only.
    pub(super) fn tick_backdrop(&mut self) {
        if self.backdrop.active {
            self.world.game_time += 1;
            // The snow's fall follows the client tick counter.
            self.ticks += 1;
        }
    }
    fn surface_y(&self, x: i32, z: i32) -> Option<i32> {
        let air = self.res.facts().air;
        (-60..320).rev().find(|&y| self.world.block(x, y, z) != air)
    }
    /// Start building the generator and finding the spot on a worker
    /// (`VanillaTerrain::new` takes seconds on a cold cache).
    fn start_backdrop(&mut self) -> Result<()> {
        log::info!(
            "Horizon: starting title backdrop after {} ms",
            self.started.elapsed().as_millis()
        );
        let seed = seed();
        let dir = self.backdrop_root()?.join(format!("{seed}"));
        std::fs::create_dir_all(&dir)?;
        // The streaming is replaced once the generator is built.
        self.streaming.paused = true;
        let mut world = World::new(self.res.facts().air, seed);
        world.day_time = DAY_TIME;
        world.game_time = 6000;
        // Light snow: the weather renders as snow in frozen biomes.
        world.weather = Weather {
            clear_time: 0,
            rain_time: i32::MAX / 2,
            thunder_time: i32::MAX / 2,
            raining: true,
            thundering: false,
            rain_level: (0.35, 0.35),
            thunder_level: (0.0, 0.0),
        };
        self.world = world;
        self.dim = super::Dim::Overworld;
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.backdrop = Backdrop {
            active: true,
            started: Some(Instant::now()),
            ..Default::default()
        };
        let (terrain, spot) = (self.backdrop.terrain.clone(), self.backdrop.spot.clone());
        let worldgen = self.options.resources.join("worldgen");
        let registry = self.pack.assets.registry;
        let biomes = self.res.biomes().clone();
        let cache = dir.join("spot.txt");
        std::thread::Builder::new()
            .name("horizon-backdrop".into())
            .spawn(move || {
                let built = Instant::now();
                let inner = match VanillaTerrain::new(&worldgen, registry, &biomes, seed) {
                    Ok(t) => Arc::new(t),
                    Err(e) => {
                        *terrain.lock().unwrap_or_else(|e| e.into_inner()) =
                            Some(Err(e.to_string()));
                        return;
                    }
                };
                log::info!(
                    "Horizon: title backdrop generator built in {} ms",
                    built.elapsed().as_millis()
                );
                *terrain.lock().unwrap_or_else(|e| e.into_inner()) = Some(Ok(inner.clone()));
                // Find (or recall) the spot.
                let known = std::fs::read_to_string(&cache).ok().and_then(|t| {
                    let mut it = t.split_whitespace().map(|v| v.parse::<i32>().ok());
                    Some((it.next()??, it.next()??))
                });
                let found = known.or_else(|| {
                    let allowed = |name: &str| name == BIOME;
                    let search = horizon_world::worldgen::generator::BiomeSearch {
                        allowed: &allowed,
                        origin: [0, 64, 0],
                        radius: 6400,
                        horizontal: 32,
                        vertical: 64,
                        min_y: -64,
                        max_y: 320,
                    };
                    let found = inner.find_closest_biome(&search);
                    log::info!("Horizon: title backdrop: {BIOME} at {found:?}");
                    let (_, p) = found?;
                    let _ = std::fs::write(&cache, format!("{} {}\n", p[0], p[2]));
                    Some((p[0], p[2]))
                });
                *spot.lock().unwrap_or_else(|e| e.into_inner()) =
                    Some(Some(found.unwrap_or((0, 0))));
            })?;
        Ok(())
    }
    fn backdrop_root(&self) -> Result<std::path::PathBuf> {
        Ok(std::env::current_dir()?
            .join("saves")
            .join(".horizon-title")
            .join("backdrop"))
    }
    /// The built generator streams the backdrop's chunks.
    fn install_backdrop(&mut self, inner: Arc<VanillaTerrain>) -> Result<()> {
        let seed = seed();
        let save = Arc::new(SaveDir::new(self.backdrop_root()?.join(format!("{seed}"))));
        let (res, pack) = (&*self.res, &self.pack);
        let mut streaming = Streaming::new(
            Arc::new(Persistent {
                inner,
                save: save.clone(),
                registry: pack.assets.clone(),
                biomes: res.biomes().clone(),
            }),
            pack.context.clone(),
            res.threads().min(6),
        );
        streaming.radius = self.settings.render_distance.clamp(4, 10);
        streaming.biome_blend = self.settings.biome_blend;
        streaming.smooth_lighting = self.settings.smooth_lighting;
        streaming.cutout_leaves = self.settings.cutout_leaves;
        streaming.cull_leaves = self.settings.horizon.cull_leaves;
        streaming.biome_zoom = BiomeZoom::new(obfuscate_seed(seed));
        streaming.paused = true;
        self.streaming = streaming;
        self.save = save;
        self.backdrop.installed = true;
        Ok(())
    }
    fn stop_backdrop(&mut self) {
        self.streaming.paused = true;
        self.world = World::new(self.res.facts().air, 0);
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.backdrop = Backdrop::default();
    }
}
