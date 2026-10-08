//! The playable game loop: streaming world, player simulation at 20 ticks/s,
//! block interaction, inventory and HUD.
mod chat;
mod commands;
mod control;
mod options;
mod packs;
mod screens;
use horizon_client::{
    Result,
    assets::Assets,
    audio::Audio,
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    chunk::{HEIGHT, MIN_Y},
    clock::FixedStep,
    entities::{Arrow, ArrowHit, Experience, ExperienceOrb, ItemEntity},
    mobs::{Mob, MobEvent, MobType, MobTypes, MobWorld},
    particles::Particle,
    containers::{BlockEntity, Providers},
    creative::{Creative, Names},
    crafting::Recipes,
    collisions,
    environment::Environment,
    gfx::{Frame, Renderer},
    hud::{HudAtlas, HudBatch, rgba},
    items::{ItemStack, Items},
    light::LightTables,
    mesher::{MeshTables, Vertex},
    models,
    motion::{MotionEvent, MotionResources, MotionWorld, PlayerMotion},
    sprint::Keys,
    streaming::{Streaming, WorkerContext},
    save::{Level, Persistent, SaveDir},
    settings::Settings,
    title::{Action, Title},
    testgen::TestTerrain,
    vanillagen::VanillaTerrain,
    world::World,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_core::{geometry::Direction, position::BlockPos};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    level::{
        Behaviours, Dimension as LevelDimension, LevelAccess, LevelEvent, Landing, PlaceContext, PlaceOutcome, PlayerContext,
        PlayerEffect, Ticks, flags as level_flags,
    },
    registry::BlockStateId,
};
use std::{
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::Arc,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

struct Options {
    resources: PathBuf,
    distance: i32,
    distance_override: Option<i32>,
    seed: i64,
    creative: bool,
    test_terrain: bool,
    frames: Option<u64>,
    screenshot_after: Option<u64>,
    timeout: Duration,
    report: Option<PathBuf>,
    world: Option<String>,
    look: Option<(f32, f32)>,
    time: Option<i64>,
    weather: Option<String>,
    dimension: Option<String>,
    /// Options.pauseOnLostFocus off (F3+P), for scripted testing.
    no_pause: bool,
    /// Control socket path (`--control`), for `horizon-ctl`.
    control: Option<PathBuf>,
}
impl Options {
    fn read() -> Result<Option<Self>> {
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or("missing executable directory")?
            .join("resources");
        let mut o = Self {
            resources: if sibling.join("horizon-resources.json").exists() {
                sibling
            } else {
                PathBuf::from(".local/horizon-client/26.3")
            },
            distance: 12,
            distance_override: None,
            seed: 0,
            creative: false,
            test_terrain: false,
            frames: None,
            screenshot_after: None,
            timeout: Duration::from_secs(120),
            report: None,
            world: None,
            look: None,
            time: None,
            weather: None,
            dimension: None,
            no_pause: false,
            control: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "Horizon — Minecraft Java Edition 26.3 in Rust\n\n  --resources PATH        Prepared resources (tools/prepare_native_client.py)\n  --render-distance N     Chunks, 2..64 (default 12)\n  --world NAME            World folder under saves/ (default \"New World\")\n  --seed N                World seed for new worlds (default 0)\n  --creative              Creative mode (default survival)\n  --test-terrain          Simple test terrain instead of vanilla generation\n  --frames N              Exit after N frames (testing)\n  --smoke-report PATH     Write a JSON report on exit\n  --control               Accept horizon-ctl commands on a local socket\n  --control-socket PATH   Control socket path (default $XDG_RUNTIME_DIR/horizon-control.sock)\n\nControls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,\nleft click break, right click place/use, 1-9/scroll hotbar, E inventory,\nQ drop, F3 debug, Esc pause."
                    );
                    return Ok(None);
                }
                "--creative" => o.creative = true,
                "--test-terrain" => o.test_terrain = true,
                "--no-pause" => o.no_pause = true,
                "--control" => o.control = Some(horizon_client::control::default_path()),
                _ => {
                    let value = args.next().ok_or_else(|| format!("missing value after {arg}"))?;
                    match arg.as_str() {
                        "--resources" => o.resources = value.into(),
                        "--render-distance" | "--radius" => {
                            o.distance = value.parse()?;
                            o.distance_override = Some(o.distance);
                            if !(2..=64).contains(&o.distance) {
                                return Err("render distance must be 2..64".into());
                            }
                        }
                        "--seed" => {
                            o.seed = value.parse().unwrap_or_else(|_| {
                                // Text seeds hash like String.hashCode.
                                value
                                    .encode_utf16()
                                    .fold(0i32, |h, c| h.wrapping_mul(31).wrapping_add(i32::from(c)))
                                    .into()
                            })
                        }
                        "--frames" => o.frames = Some(value.parse()?),
                        "--timeout-seconds" => o.timeout = Duration::from_secs(value.parse()?),
                        "--smoke-report" => o.report = Some(value.into()),
                        "--screenshot-after" => o.screenshot_after = Some(value.parse()?),
                        "--world" => o.world = Some(value),
                        "--control-socket" => o.control = Some(value.into()),
                        "--time" => o.time = Some(value.parse()?),
                        "--weather" => o.weather = Some(value),
                        "--dimension" => o.dimension = Some(if value.contains(':') { value } else { format!("minecraft:{value}") }),
                        "--look" => {
                            let (y, p) = value.split_once(',').ok_or("--look takes YAW,PITCH")?;
                            o.look = Some((y.parse()?, p.parse()?));
                        }
                        _ => return Err(format!("unknown option {arg}").into()),
                    }
                }
            }
        }
        Ok(Some(o))
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Playing,
    Paused,
    Options,
    Chat,
    Creative,
    Inventory,
    Dead,
}
#[derive(Clone)]
enum Icon {
    Flat(String),
    Cube { top: String, left: String, right: String, tint: u32 },
}
struct Mining {
    pos: [i32; 3],
    progress: f32,
    ticks: u32,
}
struct Hit {
    pos: [i32; 3],
    face: [i32; 3],
    point: [f64; 3],
}

/// The client world as seen by the vanilla block-behaviour library.
struct GameLevel<'w> {
    world: &'w mut World,
    facts: &'w BlockFacts,
    events: &'w mut Vec<LevelEvent>,
    dimension: LevelDimension,
    darken: i32,
    raining: bool,
    thundering: bool,
    player_box: Option<([f64; 3], [f64; 3])>,
    rules: horizon_world::level::GameRules,
}
impl LevelAccess for GameLevel<'_> {
    fn game_rules(&self) -> horizon_world::level::GameRules {
        self.rules
    }
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.world.block(pos.x, pos.y, pos.z)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        if !self.world.loaded(pos.x, pos.z) || pos.y < MIN_Y || pos.y >= MIN_Y + HEIGHT {
            return None;
        }
        let previous = self.world.block(pos.x, pos.y, pos.z);
        self.world.set(pos.x, pos.y, pos.z, state).ok()?;
        Some(previous)
    }
    fn is_loaded(&self, pos: BlockPos) -> bool {
        self.world.loaded(pos.x, pos.z)
    }
    fn min_y(&self) -> i32 {
        if self.dimension == LevelDimension::Nether { 0 } else { MIN_Y }
    }
    fn max_y(&self) -> i32 {
        if self.dimension == LevelDimension::Nether { 255 } else { MIN_Y + HEIGHT - 1 }
    }
    fn random(&mut self) -> &mut horizon_core::random::LegacyRandom {
        &mut self.world.random
    }
    fn game_time(&self) -> i64 {
        self.world.game_time
    }
    fn block_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) & 15)
    }
    fn sky_light(&self, pos: BlockPos) -> i32 {
        i32::from(self.world.light(pos.x, pos.y, pos.z) >> 4)
    }
    fn sky_darken(&self) -> i32 {
        self.darken
    }
    fn is_raining(&self) -> bool {
        self.raining
    }
    fn is_thundering(&self) -> bool {
        self.thundering
    }
    fn dimension(&self) -> LevelDimension {
        self.dimension
    }
    /// Level.isUnobstructed against the player for full-collision blocks.
    fn is_unobstructed(&self, pos: BlockPos, state: BlockStateId) -> bool {
        let Some((lo, hi)) = self.player_box else { return true };
        if !self.facts.get(state).full_collision {
            return true;
        }
        let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
        !(x < hi[0] - 1e-7 && x + 1.0 > lo[0] + 1e-7 && y < hi[1] - 1e-7 && y + 1.0 > lo[1] + 1e-7 && z < hi[2] - 1e-7 && z + 1.0 > lo[2] + 1e-7)
    }
    fn emit(&mut self, event: LevelEvent) {
        self.events.push(event);
    }
}
/// FallingBlockEntity carrying a block state.
#[derive(Clone, Debug)]
struct FallingBlock {
    state: BlockStateId,
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    age: u32,
}
pub struct Resources {
    assets: Arc<Assets>,
    facts: BlockFacts,
    items: Items,
    recipes: Recipes,
    biomes: Arc<Biomes>,
    environment: Environment,
    atlas: HudAtlas,
    icons: HashMap<String, Option<Icon>>,
    layer_names: HashMap<u32, String>,
    mob_types: HashMap<String, Arc<MobType>>,
    providers: Providers,
    chest_model: Option<horizon_client::mobs::Part>,
    nether_environment: Environment,
    behaviours: Behaviours,
    names: Names,
    /// clouds.png: (width, height, filled cells).
    clouds: (u32, u32, Vec<bool>),
    context: Arc<WorkerContext>,
    threads: usize,
    player_arm: Option<horizon_client::mobs::Part>,
    /// firstperson_righthand display transforms per item: (rotation, translation, scale).
    displays: HashMap<String, Option<([f32; 3], [f32; 3], [f32; 3])>>,
}
struct Game<'a> {
    res: &'a mut Resources,
    collision: &'a BlockCollisionRegistry<'a>,
    motion: &'a MotionResources,
    world: World,
    streaming: Streaming,
    options: Options,
    player: Option<PlayerMotion>,
    spawn: Option<Vec3>,
    clock: FixedStep,
    previous_position: Vec3,
    eye: f32,
    previous_eye: f32,
    yaw: f32,
    pitch: f32,
    inventory: Vec<Option<ItemStack>>,
    selected: usize,
    carried: Option<ItemStack>,
    craft_grid: Vec<Option<ItemStack>>,
    craft_size: usize,
    /// Open container block entity, if any.
    container: Option<(i32, i32, i32)>,
    scratch_slot: Option<ItemStack>,
    mining: Option<Mining>,
    destroy_delay: u32,
    use_delay: u32,
    creative: bool,
    keys: HashSet<KeyCode>,
    left: bool,
    right: bool,
    cursor: (f32, f32),
    captured: bool,
    focused: bool,
    debug: bool,
    screen: Screen,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    started: Instant,
    previous: Instant,
    frames: u64,
    fps: f32,
    fps_frames: u64,
    fps_at: Instant,
    ticks: u64,
    error: Option<String>,
    message: Option<(String, Instant)>,
    save: Arc<SaveDir>,
    level: Level,
    audio: Audio,
    dropped: Vec<ItemEntity>,
    next_entity: u32,
    particles: Vec<Particle>,
    spawn_search: u32,
    mobs: Vec<Mob>,
    arrows: Vec<Arrow>,
    attack_ticks: u32,
    last_hurt_by_player: HashSet<u32>,
    chest_open: HashMap<(i32, i32, i32), (f32, f32)>,
    /// Item being used (eaten): hotbar slot and ticks used.
    using: Option<(usize, u32)>,
    title: Option<Title>,
    /// Armor slots: head, chest, legs, feet.
    armor: [Option<ItemStack>; 4],
    applied_modifiers: Vec<(String, String)>,
    experience: Experience,
    orbs: Vec<ExperienceOrb>,
    xp_delay: u32,
    last_level_up: u64,
    /// Arm swing: ticks into the 6-tick swing (None when idle), previous/current progress.
    swing: Option<u32>,
    swing_anim: (f32, f32),
    /// Main-hand equip height (previous, current) and the item it shows.
    hand_height: (f32, f32),
    hand_item: Option<ItemStack>,
    settings: Settings,
    /// Where the options screen returns: true = title, false = pause menu.
    options_from_title: bool,
    dragging: bool,
    walk: (f32, f32),
    bob: (f32, f32),
    flash: u32,
    /// Sleeping in a bed: (bed head position, ticks asleep).
    sleeping: Option<([i32; 3], u32)>,
    dim: Dim,
    world_root: PathBuf,
    /// A player carried through a portal, re-placed once the destination loads.
    travelling: Option<(PlayerMotion, Vec3)>,
    portal_ticks: u32,
    portal_cooldown: u32,
    block_ticks: Ticks,
    tick_chunks: HashSet<(i32, i32)>,
    level_events: Vec<LevelEvent>,
    falling: Vec<FallingBlock>,
    creative_screen: Creative,
    control: Option<horizon_client::control::Server>,
    frame_stats: horizon_client::control::FrameStats,
    /// Control replies waiting for a rendered frame or a benchmark deadline.
    pending_screenshot: Option<std::sync::mpsc::Sender<String>>,
    pending_bench: Option<(Instant, std::sync::mpsc::Sender<String>)>,
    ui: chat::UiState,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dim {
    Overworld,
    Nether,
}
impl Dim {
    fn id(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
        }
    }
    fn from_id(id: &str) -> Self {
        if id == "minecraft:the_nether" { Self::Nether } else { Self::Overworld }
    }
    /// Region folder inside the world (vanilla's DIM-1 for the Nether).
    fn folder(self) -> Option<&'static str> {
        match self {
            Self::Overworld => None,
            Self::Nether => Some("DIM-1"),
        }
    }
}
fn facing_offset(facing: &str) -> [i32; 3] {
    match facing {
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        "east" => [1, 0, 0],
        "up" => [0, 1, 0],
        _ => [0, -1, 0],
    }
}
fn view_vector(yaw: f32, pitch: f32) -> [f64; 3] {
    let (y, p) = (f64::from(yaw).to_radians(), f64::from(pitch).to_radians());
    [-y.sin() * p.cos(), -p.sin(), y.cos() * p.cos()]
}
impl Game<'_> {
    fn eye_position(&self) -> Option<[f64; 3]> {
        let p = self.player.as_ref()?;
        Some([p.position.x, p.position.y + f64::from(p.dimensions.eye_height), p.position.z])
    }
    fn reach(&self) -> f64 {
        if self.creative { 5.0 } else { 4.5 }
    }
    /// Voxel traversal against non-air, non-fluid blocks (outline = full cube).
    fn pick(&self) -> Option<Hit> {
        let origin = self.eye_position()?;
        let dir = view_vector(self.yaw, self.pitch);
        let reach = self.reach();
        let mut pos = origin.map(|v| v.floor() as i32);
        let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
        let delta = dir.map(|d| if d == 0.0 { f64::INFINITY } else { (1.0 / d).abs() });
        let mut t_max = [0.0; 3];
        for i in 0..3 {
            let boundary = if dir[i] > 0.0 { f64::from(pos[i]) + 1.0 } else { f64::from(pos[i]) };
            t_max[i] = if dir[i] == 0.0 { f64::INFINITY } else { (boundary - origin[i]) / dir[i] };
        }
        let mut face = [0; 3];
        let mut t = 0.0;
        while t <= reach {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let f = self.res.facts.get(state);
            if !self.res.facts.is_air(state)
                && !(f.fluid != Fluid::None && !f.waterlogged && &*f.name != "minecraft:kelp" && f.render_shape != horizon_client::blocks::RenderShape::Model)
                && !matches!(&*f.name, "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column")
            {
                return Some(Hit {
                    pos,
                    face,
                    point: [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t],
                });
            }
            let axis = if t_max[0] < t_max[1] {
                if t_max[0] < t_max[2] { 0 } else { 2 }
            } else if t_max[1] < t_max[2] {
                1
            } else {
                2
            };
            t = t_max[axis];
            t_max[axis] += delta[axis];
            pos[axis] += step[axis];
            face = [0; 3];
            face[axis] = -step[axis];
        }
        None
    }
    fn held(&self) -> Option<&ItemStack> {
        self.inventory[self.selected].as_ref()
    }
    /// Inventory.add: returns the stack that did not fit.
    fn give(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
        let max = self.res.items.get(&stack.item).map_or(64, |i| i.max_stack);
        // Prefer the selected slot, then the hotbar, then the main inventory.
        let order: Vec<usize> = std::iter::once(self.selected).chain((0..36).filter(|&i| i != self.selected)).collect();
        for &i in &order {
            if let Some(slot) = &mut self.inventory[i]
                && slot.item == stack.item
                && slot.damage == stack.damage
                && slot.count < max
            {
                let moved = (max - slot.count).min(stack.count);
                slot.count += moved;
                stack.count -= moved;
                if stack.count == 0 {
                    return None;
                }
            }
        }
        for i in 0..36 {
            if self.inventory[i].is_none() {
                let moved = stack.count.min(max);
                self.inventory[i] = Some(ItemStack { count: moved, ..stack.clone() });
                stack.count -= moved;
                if stack.count == 0 {
                    return None;
                }
            }
        }
        Some(stack)
    }
    /// The block's particle texture (approximated by its first model face) and tint.
    /// The `particle` texture of a block's first blockstate model (for models without faces).
    fn model_particle(&self, block: &str) -> Option<String> {
        let root = &self.options.resources;
        let read = |path: String| -> Option<serde_json::Value> { serde_json::from_slice(&std::fs::read(root.join(path)).ok()?).ok() };
        let states = read(format!("assets/minecraft/blockstates/{block}.json"))?;
        let mut model = states["variants"].as_object()?.values().next().and_then(|v| v["model"].as_str().or_else(|| v[0]["model"].as_str()))?.to_owned();
        for _ in 0..8 {
            let m = read(format!("assets/minecraft/models/{}.json", model.trim_start_matches("minecraft:")))?;
            if let Some(p) = m["textures"]["particle"].as_str() {
                let p = if let Some(alias) = p.strip_prefix('#') { m["textures"][alias].as_str()? } else { p };
                return Some(p.trim_start_matches("minecraft:").to_owned());
            }
            model = m["parent"].as_str()?.to_owned();
        }
        None
    }
    fn particle_sprite(&self, state: BlockStateId) -> Option<(String, u32)> {
        let models = self.res.assets.model_catalog.collect(state, horizon_core::position::BlockPos::new(0, 0, 0)).ok()?;
        let face = models.iter().flat_map(|(_, m)| m.faces.iter()).find(|f| f.quad.facing == horizon_assets::model::Direction::Up)
            .or_else(|| models.iter().flat_map(|(_, m)| m.faces.iter()).next())?;
        let name = self.res.layer_names.get(&face.layer)?.clone();
        let tint = if face.quad.tint_index >= 0 {
            let n = &self.res.facts.get(state).name;
            if n.contains("leaves") || n.contains("vine") { 0x48B518 } else { 0x91BD59 }
        } else {
            0xFFFFFF
        };
        Some((name, tint))
    }
    /// ParticleEngine.destroy: a 4x4x4 burst.
    fn destroy_particles(&mut self, pos: [i32; 3], state: BlockStateId) {
        let Some((sprite, tint)) = self.particle_sprite(state) else { return };
        for x in 0..4 {
            for y in 0..4 {
                for z in 0..4 {
                    let d = |i: i32| (f64::from(i) + 0.5) / 4.0;
                    let p = [f64::from(pos[0]) + d(x), f64::from(pos[1]) + d(y), f64::from(pos[2]) + d(z)];
                    let v = [d(x) - 0.5, d(y) - 0.5, d(z) - 0.5];
                    let particle = Particle::terrain(&mut self.world.random, p, v, sprite.clone(), tint);
                    self.particles.push(particle);
                }
            }
        }
    }
    /// ParticleEngine.crack: one crumb on the face being mined.
    fn crack_particle(&mut self, pos: [i32; 3], face: [i32; 3], state: BlockStateId) {
        let Some((sprite, tint)) = self.particle_sprite(state) else { return };
        let r = &mut self.world.random;
        let mut p = [0.0; 3];
        for i in 0..3 {
            let base = f64::from(pos[i]);
            p[i] = match face[i] {
                1 => base + 1.0 + 0.1,
                -1 => base - 0.1,
                _ => base + 0.1 + r.next_double() * 0.8,
            };
        }
        let mut particle = Particle::terrain(r, p, [0.0; 3], sprite, tint);
        for c in &mut particle.velocity {
            *c *= 0.2;
        }
        particle.velocity[1] = (particle.velocity[1] - 0.1) * 0.2 + 0.1;
        particle.size *= 0.6;
        self.particles.push(particle);
    }
    fn spawn_item(&mut self, stack: ItemStack, position: [f64; 3], velocity: [f64; 3], pickup_delay: u32) {
        self.next_entity += 1;
        let p = Vec3::new(position[0], position[1], position[2]);
        let bob = self.world.random.next_float() * std::f32::consts::PI * 2.0;
        self.dropped.push(ItemEntity {
            id: self.next_entity,
            stack,
            position: p,
            previous: p,
            velocity: Vec3::new(velocity[0], velocity[1], velocity[2]),
            on_ground: false,
            age: 0,
            pickup_delay,
            bob_offset: bob,
        });
    }
    /// Block.popResource
    fn pop_resource(&mut self, pos: [i32; 3], stack: ItemStack) {
        let r = &mut self.world.random;
        let mut offset = || f64::from(r.next_float()) * 0.5 - 0.25;
        let p = [
            f64::from(pos[0]) + 0.5 + offset(),
            f64::from(pos[1]) + 0.5 + offset() - 0.125,
            f64::from(pos[2]) + 0.5 + offset(),
        ];
        let v = [r.next_double() * 0.2 - 0.1, 0.2, r.next_double() * 0.2 - 0.1];
        self.spawn_item(stack, p, v, 10);
    }
    /// Player.drop: thrown from the eyes along the view direction.
    fn drop_held(&mut self, all: bool) {
        let Some(eye) = self.eye_position() else { return };
        let Some(stack) = self.inventory[self.selected].as_mut() else { return };
        let count = if all { stack.count } else { 1 };
        let thrown = ItemStack { count, ..stack.clone() };
        stack.count -= count;
        if stack.count == 0 {
            self.inventory[self.selected] = None;
        }
        let (yaw, pitch) = (f64::from(self.yaw).to_radians(), f64::from(self.pitch).to_radians());
        let r = &mut self.world.random;
        let spread = f64::from(r.next_float()) * std::f64::consts::TAU;
        let strength = 0.02 * f64::from(r.next_float());
        let v = [
            -yaw.sin() * pitch.cos() * 0.3 + spread.cos() * strength,
            -pitch.sin() * 0.3 + 0.1 + f64::from(r.next_float() - r.next_float()) * 0.1,
            yaw.cos() * pitch.cos() * 0.3 + spread.sin() * strength,
        ];
        self.spawn_item(thrown, [eye[0], eye[1] - 0.3, eye[2]], v, 40);
    }
    fn tick_items(&mut self) {
        let mut items = std::mem::take(&mut self.dropped);
        items.retain_mut(|item| item.tick(&self.world, &self.res.facts, self.collision));
        let Some(p) = self.player.as_ref() else {
            self.dropped = items;
            return;
        };
        if p.health.dead_or_dying() {
            self.dropped = items;
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [p.position.x - w - 1.0, p.position.y - 0.5, p.position.z - w - 1.0],
            [p.position.x + w + 1.0, p.position.y + f64::from(p.dimensions.height) + 0.5, p.position.z + w + 1.0],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let mut kept = Vec::with_capacity(items.len());
        for mut item in items {
            let b = item.bounds();
            let touching = b.min_position().x < max[0] && b.max_position().x > min[0]
                && b.min_position().y < max[1] && b.max_position().y > min[1]
                && b.min_position().z < max[2] && b.max_position().z > min[2];
            if touching && item.pickup_delay == 0 {
                let before = item.stack.count;
                match self.give(item.stack.clone()) {
                    None => {
                        let pitch = ((self.world.random.next_float() - self.world.random.next_float()) * 0.7 + 1.0) * 2.0;
                        self.audio.play("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
                        continue;
                    }
                    Some(rest) => {
                        if rest.count < before {
                            self.audio.play("minecraft:entity.item.pickup", Some(feet), 0.2, 1.0);
                        }
                        item.stack = rest;
                    }
                }
            }
            kept.push(item);
        }
        self.dropped = kept;
    }
    /// Run vanilla block behaviour against the world through `LevelAccess`.
    fn with_level<R>(&mut self, f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R) -> R {
        let darken = self.sky_darken();
        let (rain, thunder) = self.world.weather.levels(1.0);
        let player_box = self.player.as_ref().map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            ([p.position.x - w, p.position.y, p.position.z - w], [p.position.x + w, p.position.y + f64::from(p.dimensions.height), p.position.z + w])
        });
        let dimension = if self.dim == Dim::Nether { LevelDimension::Nether } else { LevelDimension::Overworld };
        let rules = self.level_rules();
        let mut level = GameLevel {
            world: &mut self.world,
            facts: &self.res.facts,
            events: &mut self.level_events,
            dimension,
            darken,
            raining: rain > 0.2,
            thundering: thunder > 0.9,
            player_box,
            rules,
        };
        f(&self.res.behaviours, &mut level, &mut self.block_ticks)
    }
    /// Level.setBlock(pos, state, UPDATE_ALL) with neighbour/shape updates.
    fn level_set(&mut self, pos: [i32; 3], state: BlockStateId) -> Result<()> {
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]), state, level_flags::UPDATE_ALL);
        });
        self.process_level_events()
    }
    /// Track which chunks are ticking (Ticks needs chunk load/unload).
    fn sync_tick_chunks(&mut self) {
        let loaded: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        for k in loaded.difference(&self.tick_chunks.clone()) {
            let _ = self.block_ticks.add_empty_chunk(horizon_core::position::ChunkPos::new(k.0, k.1));
        }
        for k in self.tick_chunks.difference(&loaded) {
            self.block_ticks.remove_chunk(horizon_core::position::ChunkPos::new(k.0, k.1));
        }
        self.tick_chunks = loaded;
    }
    /// ServerLevel.tick block phases: scheduled ticks, random ticks within the
    /// simulation distance, block events and moving pistons.
    fn tick_blocks(&mut self) -> Result<()> {
        self.sync_tick_chunks();
        let Some(p) = self.player.as_ref() else { return Ok(()) };
        let (pcx, pcz) = ((p.position.x.floor() as i32) >> 4, (p.position.z.floor() as i32) >> 4);
        let simulation = self.settings.render_distance.min(8);
        let mut ticking = Vec::new();
        for (&(x, z), chunk) in &self.world.chunks {
            if (x - pcx).abs() > simulation || (z - pcz).abs() > simulation || !chunk.lit {
                continue;
            }
            let sections: Vec<i32> = chunk
                .sections
                .iter()
                .enumerate()
                .filter(|(_, s)| !s.is_uniform(0))
                .map(|(i, _)| MIN_Y + i as i32 * 16)
                .collect();
            ticking.push((x, z, sections));
        }
        self.with_level(|b, level, ticks| {
            b.tick_scheduled(level, ticks);
            for (x, z, sections) in ticking {
                b.tick_chunk_random(level, ticks, horizon_core::position::ChunkPos::new(x, z), sections);
            }
            b.run_block_events(level, ticks);
            b.tick_block_entities(level, ticks);
        });
        self.process_level_events()
    }
    /// Side effects reported by the behaviour library, in vanilla order.
    fn process_level_events(&mut self) -> Result<()> {
        let events = std::mem::take(&mut self.level_events);
        for event in events {
            match event {
                LevelEvent::LevelEvent { id: 2001, pos, data } => {
                    // LevelRenderer: destroy particles and the block's break sound.
                    let state = BlockStateId(data as u32);
                    let p = [pos.x, pos.y, pos.z];
                    self.destroy_particles(p, state);
                    let sounds = self.res.facts.get(state).sounds.clone();
                    let c = [f64::from(pos.x) + 0.5, f64::from(pos.y) + 0.5, f64::from(pos.z) + 0.5];
                    self.audio.play(&sounds.break_sound, Some(c), (sounds.volume + 1.0) / 2.0, sounds.pitch * 0.8);
                }
                LevelEvent::LevelEvent { id, pos, .. } => {
                    let c = Some([f64::from(pos.x) + 0.5, f64::from(pos.y) + 0.5, f64::from(pos.z) + 0.5]);
                    let sound = match id {
                        1501 => Some(("minecraft:block.lava.extinguish", 0.5)),
                        1009 => Some(("minecraft:block.fire.extinguish", 0.5)),
                        1502 => Some(("minecraft:block.redstone_torch.burnout", 0.5)),
                        1030 => Some(("minecraft:block.anvil.use", 1.0)),
                        1031 => Some(("minecraft:block.anvil.land", 0.3)),
                        _ => None,
                    };
                    if let Some((name, volume)) = sound {
                        self.audio.play(name, c, volume, 1.0);
                    }
                }
                LevelEvent::Sound { pos, sound, volume, pitch } => {
                    let c = [f64::from(pos.x) + 0.5, f64::from(pos.y) + 0.5, f64::from(pos.z) + 0.5];
                    let name = if sound.contains(':') { sound.to_string() } else { format!("minecraft:{sound}") };
                    self.audio.play(&name, Some(c), volume, pitch);
                }
                LevelEvent::DropResources { pos, state } => {
                    let drops = self.res.items.block_drops(&self.res.assets.registry, state, None, &mut self.world.random);
                    for d in drops {
                        if &*d.item != "minecraft:air" {
                            self.pop_resource([pos.x, pos.y, pos.z], d);
                        }
                    }
                    if let Some(entity) = self.world.block_entities.remove(&(pos.x, pos.y, pos.z)) {
                        for stack in entity.contents() {
                            self.pop_resource([pos.x, pos.y, pos.z], stack);
                        }
                    }
                }
                LevelEvent::FallingBlock { pos, state } => {
                    let p = Vec3::new(f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5);
                    self.falling.push(FallingBlock { state, position: p, previous: p, velocity: Vec3::ZERO, age: 0 });
                }
                LevelEvent::Player { pos, effect, .. } => match effect {
                    PlayerEffect::OpenMenu => self.open_menu([pos.x, pos.y, pos.z]),
                    PlayerEffect::Sleep => self.try_sleep([pos.x, pos.y, pos.z]),
                    PlayerEffect::GiveItem(item) => {
                        if let Some(rest) = self.give(ItemStack { item, count: 1, damage: 0 }) {
                            self.pop_resource([pos.x, pos.y, pos.z], rest);
                        }
                    }
                    PlayerEffect::DropItem(item) => self.pop_resource([pos.x, pos.y + 1, pos.z], ItemStack { item, count: 1, damage: 0 }),
                    PlayerEffect::EatCake => {
                        if let Some(p) = self.player.as_mut() {
                            let food = &mut p.health.food;
                            food.food = (food.food + 2).min(20);
                            food.saturation = (food.saturation + 0.4).min(food.food as f32);
                        }
                    }
                    PlayerEffect::InteractLoot(_) | PlayerEffect::RespawnAnchor => {}
                },
                LevelEvent::PrimeTnt { pos } => {
                    // Simplified: TNT explodes after its 80-tick fuse immediately.
                    self.explode([f64::from(pos.x) + 0.5, f64::from(pos.y), f64::from(pos.z) + 0.5], 4.0)?;
                }
                LevelEvent::GrowTree { .. } | LevelEvent::PlaceFeature { .. } | LevelEvent::BlockEvent { .. } | LevelEvent::GameEvent { .. } => {}
            }
        }
        Ok(())
    }
    /// Menus opened by PlayerEffect::OpenMenu.
    fn open_menu(&mut self, pos: [i32; 3]) {
        let name = self.res.facts.get(self.world.block(pos[0], pos[1], pos[2])).name.clone();
        let key = (pos[0], pos[1], pos[2]);
        if &*name == "minecraft:crafting_table" {
            self.craft_size = 3;
        } else if let Some(entity) = BlockEntity::for_block(&name) {
            self.world.block_entities.entry(key).or_insert(entity);
            self.container = Some(key);
            let sound = if name.contains("chest") { Some("minecraft:block.chest.open") } else if &*name == "minecraft:barrel" { Some("minecraft:block.barrel.open") } else { None };
            if let Some(sound) = sound {
                self.audio.play(sound, Some([f64::from(key.0) + 0.5, f64::from(key.1) + 0.5, f64::from(key.2) + 0.5]), 0.5, 1.0);
            }
        } else {
            self.message = Some((format!("{} is not available yet", name.trim_start_matches("minecraft:").replace('_', " ")), Instant::now()));
            return;
        }
        self.screen = Screen::Inventory;
        self.capture(false);
        self.right = false;
    }
    /// FallingBlockEntity.tick: fall with 0.04 gravity and 0.98 drag, then land.
    fn tick_falling(&mut self) -> Result<()> {
        let mut falling = std::mem::take(&mut self.falling);
        let mut landed = Vec::new();
        falling.retain_mut(|f| {
            f.previous = f.position;
            f.age += 1;
            f.velocity.y -= 0.04;
            let next = Vec3::new(f.position.x + f.velocity.x, f.position.y + f.velocity.y, f.position.z + f.velocity.z);
            let below = self.world.block(next.x.floor() as i32, next.y.floor() as i32, next.z.floor() as i32);
            let solid = self.res.facts.get(below).full_collision;
            if solid || f.age > 600 {
                landed.push((f.state, [next.x.floor() as i32, next.y.floor() as i32 + 1, next.z.floor() as i32]));
                return false;
            }
            f.position = next;
            f.velocity = Vec3::new(f.velocity.x * 0.98, f.velocity.y * 0.98, f.velocity.z * 0.98);
            true
        });
        self.falling = falling;
        for (state, pos) in landed {
            let landing = self.with_level(|b, level, ticks| b.falling_block_landed(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]), state, false));
            if matches!(landing, Landing::Broken) {
                let item = self.res.facts.get(state).item.clone();
                if &*item != "minecraft:air" {
                    self.pop_resource(pos, ItemStack { item, count: 1, damage: 0 });
                }
            }
        }
        self.process_level_events()
    }
    fn break_block(&mut self, pos: [i32; 3]) -> Result<()> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let facts = self.res.facts.get(state).clone();
        if !self.creative {
            if let Some(p) = self.player.as_mut() {
                p.health.food.exhaustion = (p.health.food.exhaustion + 0.005).min(40.0);
            }
            let held = self.held().cloned();
            let harvest = !facts.requires_tool
                || self.res.items.correct_for_drops(held.as_ref(), &facts.name);
            if harvest {
                let drops = self.res.items.block_drops(
                    &self.res.assets.registry,
                    state,
                    held.as_ref(),
                    &mut self.world.random,
                );
                let silk = false;
                for d in drops {
                    if &*d.item != "minecraft:air" {
                        self.pop_resource(pos, d);
                    }
                }
                let xp = if silk { 0 } else { self.block_experience(&facts.name) };
                if xp > 0 {
                    self.spawn_orbs([f64::from(pos[0]) + 0.5, f64::from(pos[1]) + 0.5, f64::from(pos[2]) + 0.5], xp);
                }
            }
            // Tools lose durability on blocks with nonzero hardness.
            if facts.hardness > 0.0
                && let Some(stack) = self.inventory[self.selected].as_mut()
                && let Some(info) = self.res.items.get(&stack.item)
                && info.max_damage > 0
                && !info.tool.is_empty()
            {
                stack.damage += 1;
                if stack.damage >= info.max_damage {
                    self.inventory[self.selected] = None;
                }
            }
        }
        let replacement = if facts.waterlogged {
            self.res.assets.registry.default_state("minecraft:water")
        } else {
            None
        }
        .unwrap_or(self.world.air);
        if let Some(entity) = self.world.block_entities.remove(&(pos[0], pos[1], pos[2])) {
            // Containers.dropContents
            for stack in entity.contents() {
                self.pop_resource(pos, stack);
            }
            if self.container == Some((pos[0], pos[1], pos[2])) {
                self.container = None;
                self.screen = Screen::Playing;
            }
        }
        let _ = replacement;
        // Block.playerDestroy through the vanilla behaviour library: removal,
        // neighbour and shape updates, and the destroy event (particles, sound).
        let creative = self.creative;
        self.with_level(|b, level, ticks| {
            b.player_destroy_block(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]), creative);
        });
        self.process_level_events()?;
        Ok(())
    }
    fn use_item(&mut self) -> Result<()> {
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        // MultiPlayerGameMode.useItemOn: blocks react first unless sneaking with an item.
        if !(sneaking && self.held().is_some())
            && let Some(hit) = self.pick()
        {
            let face = match hit.face {
                [0, 1, 0] => Direction::Up,
                [0, -1, 0] => Direction::Down,
                [0, 0, -1] => Direction::North,
                [0, 0, 1] => Direction::South,
                [-1, 0, 0] => Direction::West,
                _ => Direction::East,
            };
            let player = PlayerContext {
                yaw: self.yaw,
                pitch: self.pitch,
                may_build: true,
                can_eat: self.creative || self.player.as_ref().is_some_and(|p| p.health.food.food < 20),
                game_master: false,
            };
            let at = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
            let result = self.with_level(|b, level, ticks| b.use_without_item(level, ticks, BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]), face, at, player));
            self.process_level_events()?;
            if result.consumes_action() {
                self.swing_arm();
                return Ok(());
            }
        }
        let Some(stack) = self.held().cloned() else { return Ok(()) };
        let Some(info) = self.res.items.get(&stack.item).cloned() else { return Ok(()) };
        if &*stack.item == "minecraft:bow" {
            if self.using.is_none() {
                self.using = Some((self.selected, 0));
            }
            return Ok(());
        }
        if info.food.is_some()
            && let Some(p) = self.player.as_ref()
            && (p.health.food.food < 20 || self.creative)
        {
            // Consumable.startConsuming: held right click for consume_seconds.
            if self.using.is_none() {
                self.using = Some((self.selected, 0));
            }
            return Ok(());
        }
        if self.use_tool(&stack)? {
            return Ok(());
        }
        let Some(hit) = self.pick() else { return Ok(()) };
        if info.block.is_none() {
            return Ok(());
        }
        let face = match hit.face {
            [0, 1, 0] => Direction::Up,
            [0, -1, 0] => Direction::Down,
            [0, 0, -1] => Direction::North,
            [0, 0, 1] => Direction::South,
            [-1, 0, 0] => Direction::West,
            _ => Direction::East,
        };
        // BlockItem.place through the vanilla placement rules.
        let ctx = PlaceContext {
            clicked_pos: BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
            face,
            hit: Vec3::new(hit.point[0], hit.point[1], hit.point[2]),
            inside: false,
            yaw: self.yaw,
            pitch: self.pitch,
            sneaking: self.keys.contains(&KeyCode::ShiftLeft),
            item: stack.item.to_string(),
            can_use_game_master_blocks: false,
        };
        let outcome = self.with_level(|b, level, ticks| b.place_block(level, ticks, &ctx));
        let PlaceOutcome::Placed { pos, state } = outcome else {
            self.process_level_events()?;
            return Ok(());
        };
        let target = [pos.x, pos.y, pos.z];
        if let Some(entity) = BlockEntity::for_block(&self.res.facts.get(state).name) {
            self.world.block_entities.insert((target[0], target[1], target[2]), entity);
        }
        self.swing_arm();
        let sounds = self.res.facts.get(state).sounds.clone();
        let center = [f64::from(target[0]) + 0.5, f64::from(target[1]) + 0.5, f64::from(target[2]) + 0.5];
        self.audio.play(&sounds.place, Some(center), (sounds.volume + 1.0) / 2.0, sounds.pitch * 0.8);
        self.process_level_events()?;
        self.consume_held();
        Ok(())
    }
    fn consume_held(&mut self) {
        if self.creative {
            return;
        }
        if let Some(s) = self.inventory[self.selected].as_mut() {
            s.count -= 1;
            if s.count == 0 {
                self.inventory[self.selected] = None;
            }
        }
    }
    fn tick_using(&mut self) {
        let Some((slot, ticks)) = self.using else { return };
        let Some(stack) = self.inventory[slot].clone() else {
            self.using = None;
            return;
        };
        if !self.right || slot != self.selected || self.screen != Screen::Playing {
            if &*stack.item == "minecraft:bow" && slot == self.selected {
                self.release_bow(ticks);
            }
            self.using = None;
            return;
        }
        let Some(info) = self.res.items.get(&stack.item).cloned() else {
            self.using = None;
            return;
        };
        if &*stack.item == "minecraft:bow" {
            self.using = Some((slot, ticks + 1));
            return;
        }
        let duration = 32;
        let ticks = ticks + 1;
        let remaining = duration - ticks.min(duration);
        let eye = self.eye_position().unwrap_or_default();
        // Consumable.shouldEmitParticlesAndSounds: every 4 ticks in the last 25.
        if remaining <= 25 && remaining % 4 == 0 {
            let pitch = (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
            self.audio.play("minecraft:entity.generic.eat", Some(eye), 0.5 + 0.5 * self.world.random.next_int_bound(2).unwrap_or(0) as f32, pitch);
            if let Some(Icon::Flat(sprite)) = self.icon(&stack.item) {
                let (yaw, pitch) = (f64::from(self.yaw).to_radians(), f64::from(self.pitch).to_radians());
                let at = [eye[0] - yaw.sin() * pitch.cos() * 0.3, eye[1] - 0.1, eye[2] + yaw.cos() * pitch.cos() * 0.3];
                for _ in 0..5 {
                    let v = [(f64::from(self.world.random.next_float()) - 0.5) * 0.1, 0.1, (f64::from(self.world.random.next_float()) - 0.5) * 0.1];
                    let p = Particle::terrain(&mut self.world.random, at, v, sprite.clone(), 0xFFFFFF);
                    self.particles.push(p);
                }
            }
        }
        if ticks >= duration {
            self.using = None;
            if let Some((nutrition, saturation)) = info.food
                && let Some(p) = self.player.as_mut()
            {
                // FoodData.eat(nutrition, saturation)
                let food = &mut p.health.food;
                food.food = (food.food + nutrition as i32).clamp(0, 20);
                food.saturation = (food.saturation + saturation).clamp(0.0, food.food as f32);
                self.audio.play("minecraft:entity.player.burp", Some(eye), 0.5, self.world.random.next_float() * 0.1 + 0.9);
                self.consume_held();
            }
        } else {
            self.using = Some((slot, ticks));
        }
    }
    fn tick_mining(&mut self) -> Result<()> {
        if self.destroy_delay > 0 {
            self.destroy_delay -= 1;
        }
        if !self.left || self.screen != Screen::Playing {
            self.mining = None;
            return Ok(());
        }
        let Some(hit) = self.pick() else {
            self.mining = None;
            return Ok(());
        };
        if self.creative {
            if self.destroy_delay == 0 {
                self.break_block(hit.pos)?;
                self.destroy_delay = 5;
            }
            return Ok(());
        }
        if self.destroy_delay > 0 {
            return Ok(());
        }
        if self.mining.as_ref().is_none_or(|m| m.pos != hit.pos) {
            self.mining = Some(Mining { pos: hit.pos, progress: 0.0, ticks: 0 });
        }
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let facts = self.res.facts.get(state);
        if facts.hardness < 0.0 {
            return Ok(());
        }
        // Player.getDestroySpeed / BlockBehaviour.getDestroyProgress.
        let held = self.held();
        let mut speed = self.res.items.mining_speed(held, &facts.name);
        let player = self.player.as_ref().ok_or("no player")?;
        if !player.ground.on_ground {
            speed /= 5.0;
        }
        let eye = self.eye_position().unwrap_or_default();
        if self.res.facts.get(self.world.block(eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32)).fluid == Fluid::Water {
            speed /= 5.0;
        }
        let harvest = !facts.requires_tool || self.res.items.correct_for_drops(held, &facts.name);
        let progress = if facts.hardness == 0.0 {
            1.0
        } else {
            speed / facts.hardness / if harvest { 30.0 } else { 100.0 }
        };
        let hit_sound = facts.sounds.clone();
        self.swing_arm();
        let m = self.mining.as_mut().ok_or("mining")?;
        m.progress += progress;
        m.ticks += 1;
        let crack = (m.pos, hit.face);
        if m.ticks % 4 == 1 {
            let c = [f64::from(m.pos[0]) + 0.5, f64::from(m.pos[1]) + 0.5, f64::from(m.pos[2]) + 0.5];
            self.audio.play(&hit_sound.hit, Some(c), (hit_sound.volume + 1.0) / 8.0, hit_sound.pitch * 0.5);
        }
        self.crack_particle(crack.0, crack.1, state);
        let m = self.mining.as_mut().ok_or("mining")?;
        if m.progress >= 1.0 {
            let pos = m.pos;
            self.mining = None;
            self.break_block(pos)?;
            self.destroy_delay = 5;
        }
        Ok(())
    }
    /// The i-th world spawn search point: a square spiral in 48-block steps.
    fn spawn_candidate(i: u32) -> (i32, i32) {
        if i == 0 {
            return (0, 0);
        }
        let mut k = 1u32;
        let mut ring = 1i32;
        loop {
            let points = 8 * ring as u32;
            if i < k + points {
                let j = (i - k) as i32;
                let side = j / (2 * ring);
                let t = j % (2 * ring) - ring;
                let (x, z) = match side {
                    0 => (t, -ring),
                    1 => (ring, t),
                    2 => (-t, ring),
                    _ => (-ring, -t),
                };
                return (x * 48, z * 48);
            }
            k += points;
            ring += 1;
        }
    }
    /// A column whose surface is a full solid, non-fluid, non-leaf block open to the sky.
    fn land_column(&self, x: i32, z: i32) -> Option<i32> {
        if self.dim == Dim::Nether {
            // Below the bedrock ceiling: a solid floor with two air blocks above.
            for y in (32..120).rev() {
                let floor = self.res.facts.get(self.world.block(x, y - 1, z));
                if floor.full_collision && floor.fluid == Fluid::None
                    && self.res.facts.is_air(self.world.block(x, y, z))
                    && self.res.facts.is_air(self.world.block(x, y + 1, z))
                {
                    return Some(y);
                }
            }
            return None;
        }
        let mut y = MIN_Y + HEIGHT - 1;
        while y > MIN_Y {
            let s = self.world.block(x, y, z);
            if !self.res.facts.is_air(s) {
                let f = self.res.facts.get(s);
                let ok = f.full_collision && f.fluid == Fluid::None && !f.name.contains("leaves") && f.hardness >= 0.0;
                return ok.then_some(y + 1);
            }
            y -= 1;
        }
        None
    }
    fn spawn_player(&mut self) -> Result<()> {
        if self.player.is_some() {
            return Ok(());
        }
        if let Some((_, target)) = &self.travelling {
            let (cx, cz) = ((target.x.floor() as i32) >> 4, (target.z.floor() as i32) >> 4);
            self.streaming.center = (cx, cz);
            if (-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                let (player, target) = self.travelling.take().ok_or("travel")?;
                self.place_after_travel(player, target)?;
            }
            return Ok(());
        }
        let saved = self.level.position.map(|p| Vec3::new(p[0], p[1], p[2]));
        if self.spawn.is_none() {
            self.spawn = self.level.spawn.map(|s| Vec3::new(s[0] + 0.5, s[1], s[2] + 0.5));
        }
        let anchor = match saved.or(self.spawn) {
            Some(p) => (p.x.floor() as i32, p.z.floor() as i32),
            None => Self::spawn_candidate(self.spawn_search),
        };
        let (cx, cz) = (anchor.0 >> 4, anchor.1 >> 4);
        self.streaming.center = (cx, cz);
        if !(-1..=1).all(|dz| (-1..=1).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
            return Ok(());
        }
        let feet = if let Some(p) = saved {
            self.level.position = None;
            p
        } else if let Some(s) = self.spawn {
            // WorldSpawn: the stored column, lifted onto its current surface.
            let y = self.land_column(s.x.floor() as i32, s.z.floor() as i32).map_or(s.y, f64::from);
            Vec3::new(s.x, y.max(s.y), s.z)
        } else {
            let mut best: Option<(i64, i32, i32, i32)> = None;
            for z in (cz - 1) * 16..(cz + 2) * 16 {
                for x in (cx - 1) * 16..(cx + 2) * 16 {
                    if let Some(y) = self.land_column(x, z) {
                        let d = i64::from(x - anchor.0).pow(2) + i64::from(z - anchor.1).pow(2);
                        if best.is_none_or(|b| d < b.0) {
                            best = Some((d, x, y, z));
                        }
                    }
                }
            }
            match best {
                Some((_, x, y, z)) => {
                    let spawn = Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5);
                    self.spawn = Some(spawn);
                    self.level.spawn = Some([f64::from(x), f64::from(y), f64::from(z)]);
                    spawn
                }
                None => {
                    self.spawn_search += 1;
                    return Ok(());
                }
            }
        };
        let mut player = PlayerMotion::new(self.motion, feet)?;
        player.yaw = self.yaw;
        if self.level.health > 0.0 && self.ticks < 2 {
            player.set_health(self.level.health, false)?;
            player.health.food.food = self.level.food;
            player.health.food.saturation = self.level.saturation;
        }
        self.previous_position = feet;
        self.player = Some(player);
        self.apply_equipment();
        self.eye = 1.62;
        self.previous_eye = 1.62;
        self.clock.reset();
        Ok(())
    }
    fn tick(&mut self) -> Result<()> {
        self.ticks += 1;
        self.world.game_time += 1;
        if self.ui.rules.bool("advance_weather") {
            self.world.weather.tick(&mut self.world.random);
        }
        self.tick_weather_effects();
        if self.ticks % 6000 == 0 {
            self.save_game()?;
        }
        self.advance_clock();
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        let Some(player) = self.player.as_mut() else { return Ok(()) };
        // Hold position until the surrounding terrain exists.
        let (cx, cz) = ((player.position.x.floor() as i32) >> 4, (player.position.z.floor() as i32) >> 4);
        if !self.streaming.is_lit(cx, cz) {
            return Ok(());
        }
        let playing = self.screen == Screen::Playing;
        let held = |k| playing && self.keys.contains(&k);
        let keys = Keys {
            forward: held(KeyCode::KeyW),
            backward: held(KeyCode::KeyS),
            left: held(KeyCode::KeyA),
            right: held(KeyCode::KeyD),
            jump: held(KeyCode::Space),
            shift: held(KeyCode::ShiftLeft),
            sprint: held(KeyCode::ControlLeft),
        };
        self.previous_position = player.position;
        player.yaw = self.yaw;
        MotionWorld {
            player,
            scene: &mut self.world,
            assets: &self.res.assets,
            collision_binding: self.collision,
            resources: self.motion,
        }
        .controlled_simulation_tick(keys)?;
        let events = std::mem::take(&mut player.events);
        let feet = [player.position.x, player.position.y, player.position.z];
        self.previous_eye = self.eye;
        self.eye += (player.dimensions.eye_height - self.eye) * 0.5;
        let crouching = player.pose == horizon_client::pose::Pose::Crouching;
        for event in events {
            match event {
                MotionEvent::Emission { position, state, .. } if !crouching => {
                    // Entity.playStepSound: the block's step sound at 0.15 volume.
                    let sounds = self.res.facts.get(state).sounds.clone();
                    self.audio.play(&sounds.step, Some(feet), sounds.volume * 0.15, sounds.pitch);
                    let _ = position;
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Sound { sound, volume, pitch, .. }) => {
                    let name = match sound {
                        horizon_client::landing::SceneSound::BigFall => "minecraft:entity.player.big_fall",
                        horizon_client::landing::SceneSound::SmallFall => "minecraft:entity.player.small_fall",
                        _ => continue,
                    };
                    self.audio.play(name, Some(feet), volume, pitch);
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Damage { .. }) => {
                    self.audio.play("minecraft:entity.player.hurt", Some(feet), 1.0, 1.0);
                }
                _ => {}
            }
        }
        let Some(player) = self.player.as_mut() else { return Ok(()) };
        if player.position.y < f64::from(MIN_Y - 64) {
            // Out of the world.
            let _ = player.set_health(0.0, false);
        }
        if player.health.dead_or_dying() && self.screen != Screen::Dead {
            // Inventory.dropAll: every stack thrown in a random direction.
            let at = [player.position.x, player.position.y + 1.32, player.position.z];
            self.death_message();
            if !self.creative && !self.ui.rules.bool("keep_inventory") {
                let stacks: Vec<ItemStack> = self.inventory.iter_mut().filter_map(Option::take).collect();
                for stack in stacks {
                    let r = &mut self.world.random;
                    let strength = f64::from(r.next_float()) * 0.5;
                    let angle = f64::from(r.next_float()) * std::f64::consts::TAU;
                    let v = [-angle.sin() * strength, 0.2, angle.cos() * strength];
                    self.spawn_item(stack, at, v, 40);
                }
            }
            self.audio.play("minecraft:entity.player.death", Some(at), 1.0, 1.0);
            self.screen = Screen::Dead;
            self.capture(false);
        }
        self.tick_using();
        self.tick_mining()?;
        self.tick_items();
        self.tick_mobs()?;
        self.tick_experience();
        self.tick_hand();
        self.tick_sleep();
        self.tick_portal()?;
        self.tick_blocks()?;
        self.tick_falling()?;
        if let Some(p) = &self.player {
            // Player.aiStep bob and Entity.walkDist (0.6 per block).
            let (dx, dz) = (p.position.x - self.previous_position.x, p.position.z - self.previous_position.z);
            let speed = ((dx * dx + dz * dz).sqrt() as f32).min(0.1);
            self.walk.0 = self.walk.1;
            self.walk.1 += (dx * dx + dz * dz).sqrt() as f32 * 0.6;
            self.bob.0 = self.bob.1;
            let target = if p.ground.on_ground && !p.health.dead_or_dying() { speed } else { 0.0 };
            self.bob.1 += (target - self.bob.1) * 0.4;
        }
        self.tick_block_entities()?;
        let facts = &self.res.facts;
        let world = &self.world;
        self.particles.retain_mut(|p| p.tick(world, |id| facts.states[id as usize].full_collision));
        if self.particles.len() > 4000 {
            let excess = self.particles.len() - 4000;
            self.particles.drain(..excess);
        }
        if self.right && self.use_delay == 0 && self.screen == Screen::Playing {
            self.use_item()?;
            self.use_delay = 4;
        }
        Ok(())
    }
    fn capture(&mut self, capture: bool) {
        let Some(window) = &self.window else { return };
        if capture {
            self.captured = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
        } else {
            let _ = window.set_cursor_grab(CursorGrabMode::None);
            self.captured = false;
        }
        window.set_cursor_visible(!self.captured);
    }
    fn icon(&mut self, item: &str) -> Option<Icon> {
        if let Some(icon) = self.res.icons.get(item) {
            return icon.clone();
        }
        let icon = self.resolve_icon(item);
        self.res.icons.insert(item.to_owned(), icon.clone());
        icon
    }
    fn resolve_icon(&self, item: &str) -> Option<Icon> {
        let root = &self.options.resources;
        let name = item.trim_start_matches("minecraft:");
        let info = self.res.items.get(item);
        let model_name = info.map_or(name.to_owned(), |i| i.model.trim_start_matches("minecraft:").to_owned());
        let def: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(format!("assets/minecraft/items/{model_name}.json"))).ok()?,
        )
        .ok()?;
        // Find the first plain model reference in the item model definition.
        fn first_model(v: &serde_json::Value) -> Option<String> {
            match v {
                serde_json::Value::Object(o) => {
                    if o.get("type").and_then(|t| t.as_str()) == Some("minecraft:model") {
                        return o.get("model").and_then(|m| m.as_str()).map(str::to_owned);
                    }
                    for key in ["model", "fallback", "on_false", "on_true", "cases", "entries", "tints"] {
                        if let Some(m) = o.get(key).and_then(first_model) {
                            return Some(m);
                        }
                    }
                    o.values().find_map(first_model)
                }
                serde_json::Value::Array(a) => a.iter().find_map(first_model),
                _ => None,
            }
        }
        let Some(model) = first_model(&def["model"]) else {
            // Special item models (chests, beds, banners...): show the block's particle texture.
            let block = self.res.assets.registry.default_state(item).or_else(|| info.and_then(|i| i.block))?;
            let (sprite, tint) = self.particle_sprite(block).or_else(|| self.model_particle(name).map(|s| (s, 0xFFFFFF)))?;
            return Some(Icon::Cube { top: sprite.clone(), left: sprite.clone(), right: sprite, tint });
        };
        let path = model.trim_start_matches("minecraft:").to_owned();
        let mut current = path.clone();
        let mut textures = serde_json::Map::new();
        for _ in 0..16 {
            let text = std::fs::read(root.join(format!("assets/minecraft/models/{current}.json"))).ok()?;
            let m: serde_json::Value = serde_json::from_slice(&text).ok()?;
            if let Some(t) = m["textures"].as_object() {
                for (k, v) in t {
                    textures.entry(k.clone()).or_insert(v.clone());
                }
            }
            match m["parent"].as_str() {
                Some(p) if p.ends_with("item/generated") || p.ends_with("item/handheld") || p.contains("handheld") => {
                    let layer = textures.get("layer0")?.as_str()?;
                    return Some(Icon::Flat(layer.trim_start_matches("minecraft:").to_owned()));
                }
                Some(p) => current = p.trim_start_matches("minecraft:").to_owned(),
                None => break,
            }
        }
        if path.starts_with("block/") {
            let block = self.res.assets.registry.default_state(&format!("minecraft:{name}"))
                .or_else(|| info.and_then(|i| i.block))?;
            let models = self.res.assets.model_catalog.collect(block, horizon_core::position::BlockPos::new(0, 0, 0)).ok()?;
            let mut faces: HashMap<usize, (u32, i32)> = HashMap::new();
            for (_, m) in &models {
                for f in &m.faces {
                    faces.entry(f.quad.facing as usize).or_insert((f.layer, f.quad.tint_index));
                }
            }
            use horizon_assets::model::Direction as D;
            let get = |d: D| faces.get(&(d as usize)).and_then(|(l, _)| self.res.layer_names.get(l)).cloned();
            let top = get(D::Up)?;
            let tinted = faces.get(&(D::Up as usize)).is_some_and(|(_, t)| *t >= 0);
            let tint = if tinted {
                if name.contains("leaves") || name == "vine" { 0x48B518 } else { 0x91BD59 }
            } else {
                0xFFFFFF
            };
            return Some(Icon::Cube {
                left: get(D::South).unwrap_or(top.clone()),
                right: get(D::East).unwrap_or(top.clone()),
                top,
                tint,
            });
        }
        textures
            .get("layer0")
            .or_else(|| textures.get("all"))
            .or_else(|| textures.get("texture"))
            .and_then(|t| t.as_str())
            .map(|t| Icon::Flat(t.trim_start_matches("minecraft:").to_owned()))
    }
    fn draw_item(&mut self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        let icon = self.icon(&stack.item);
        let atlas = &self.res.atlas;
        match icon {
            Some(Icon::Flat(name)) => batch.sprite_scaled(atlas, &name, x, y, 16.0, 16.0, u32::MAX),
            Some(Icon::Cube { top, left, right, tint }) => {
                let color = |b: f32| {
                    let c = |s: u32| (((tint >> s) & 255) as f32 * b) as u8;
                    rgba(c(16), c(8), c(0), 255)
                };
                let (cx, top_y) = (x + 8.0, y + 1.0);
                let quad = |batch: &mut HudBatch, name: &str, p: [[f32; 2]; 4], c: u32| {
                    if let Some(r) = atlas.sprites.get(name).copied() {
                        batch.quad(p, [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]], c);
                    }
                };
                let (w, h) = (7.0, 3.6);
                quad(batch, &top, [[cx, top_y], [cx + w, top_y + h], [cx, top_y + 2.0 * h], [cx - w, top_y + h]], color(1.0));
                let side = 8.0;
                quad(batch, &left, [[cx - w, top_y + h], [cx, top_y + 2.0 * h], [cx, top_y + 2.0 * h + side], [cx - w, top_y + h + side]], color(0.8));
                quad(batch, &right, [[cx, top_y + 2.0 * h], [cx + w, top_y + h], [cx + w, top_y + h + side], [cx, top_y + 2.0 * h + side]], color(0.6));
            }
            None => batch.fill(atlas, x + 2.0, y + 2.0, 12.0, 12.0, rgba(200, 0, 200, 255)),
        }
        if stack.count > 1 {
            let text = stack.count.to_string();
            let w = HudBatch::text_width(atlas, &text);
            batch.text(atlas, &text, x + 17.0 - w, y + 9.0, u32::MAX, true);
        }
        if let Some(info) = self.res.items.get(&stack.item)
            && info.max_damage > 0
            && stack.damage > 0
        {
            let f = 1.0 - stack.damage as f32 / info.max_damage as f32;
            let width = (13.0 * f).round();
            let hue = f / 3.0;
            let (r, g) = ((255.0 * (1.0 - hue * 3.0).clamp(0.0, 1.0)) as u8, (255.0 * (hue * 3.0).clamp(0.0, 1.0)) as u8);
            batch.fill(atlas, x + 2.0, y + 13.0, 13.0, 2.0, rgba(0, 0, 0, 255));
            batch.fill(atlas, x + 2.0, y + 13.0, width, 1.0, rgba(r, g, 0, 255));
        }
    }
    fn hud(&mut self, size: [u32; 2]) -> HudBatch {
        let mut batch = HudBatch::new(size);
        let (w, h) = (batch.width, batch.height);
        if self.screen == Screen::Options {
            self.draw_options(&mut batch);
            return batch;
        }
        if let Some(title) = &self.title {
            title.draw(&self.res.atlas, &mut batch, self.cursor);
            return batch;
        }
        let atlas_ptr: *const HudAtlas = &self.res.atlas;
        let _ = atlas_ptr;
        if self.player.is_none() {
            let text = if self.spawn_search > 0 { "Finding a place to spawn..." } else { "Loading terrain..." };
            let atlas = &self.res.atlas;
            let tw = HudBatch::text_width(atlas, text);
            batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255));
            batch.text(atlas, text, (w - tw) / 2.0, h / 2.0 - 4.0, u32::MAX, true);
            return batch;
        }
        {
            let atlas = &self.res.atlas;
            // Crosshair.
            batch.sprite(atlas, "gui/sprites/hud/crosshair", (w - 15.0) / 2.0, (h - 15.0) / 2.0);
            // Hotbar.
            batch.sprite(atlas, "gui/sprites/hud/hotbar", w / 2.0 - 91.0, h - 22.0);
            batch.sprite(atlas, "gui/sprites/hud/hotbar_selection", w / 2.0 - 91.0 - 1.0 + self.selected as f32 * 20.0, h - 23.0);
        }
        for i in 0..9 {
            if let Some(stack) = self.inventory[i].clone() {
                self.draw_item(&mut batch, &stack, w / 2.0 - 90.0 + i as f32 * 20.0 + 2.0, h - 19.0);
            }
        }
        let atlas = &self.res.atlas;
        if !self.creative
            && let Some(p) = &self.player
        {
            // Experience bar and hearts/food (Gui.renderPlayerHealth).
            batch.sprite(atlas, "gui/sprites/hud/experience_bar_background", w / 2.0 - 91.0, h - 29.0);
            let filled = (self.experience.progress * 183.0).floor() / 182.0;
            if filled > 0.0 {
                batch.sprite_part(atlas, "gui/sprites/hud/experience_bar_progress", w / 2.0 - 91.0, h - 29.0, filled.min(1.0));
            }
            if self.experience.level > 0 {
                // Gui.renderExperienceLevel: green text with a black outline.
                let text = self.experience.level.to_string();
                let tw = HudBatch::text_width(atlas, &text);
                let (x, y) = ((w - tw) / 2.0, h - 31.0 - 4.0);
                for (dx, dy) in [(1.0, 0.0), (-1.0, 0.0), (0.0, 1.0), (0.0, -1.0)] {
                    batch.text(atlas, &text, x + dx, y + dy, rgba(0, 0, 0, 255), false);
                }
                batch.text(atlas, &text, x, y, rgba(128, 255, 32, 255), false);
            }
            let health = p.health.health().ceil() as i32;
            let top = h - 39.0;
            for i in 0..10 {
                let x = w / 2.0 - 91.0 + i as f32 * 8.0;
                let y = if health <= 4 { top + ((self.ticks + i as u64 * 7) % 3) as f32 - 1.0 } else { top };
                batch.sprite(atlas, "gui/sprites/hud/heart/container", x, y);
                let v = health - i * 2;
                if v >= 2 {
                    batch.sprite(atlas, "gui/sprites/hud/heart/full", x, y);
                } else if v == 1 {
                    batch.sprite(atlas, "gui/sprites/hud/heart/half", x, y);
                }
            }
            let food = p.health.food.food;
            for i in 0..10 {
                let x = w / 2.0 + 91.0 - i as f32 * 8.0 - 9.0;
                batch.sprite(atlas, "gui/sprites/hud/food_empty", x, top);
                let v = food - i * 2;
                if v >= 2 {
                    batch.sprite(atlas, "gui/sprites/hud/food_full", x, top);
                } else if v == 1 {
                    batch.sprite(atlas, "gui/sprites/hud/food_half", x, top);
                }
            }
        }
        if self.debug {
            let p = self.player.as_ref().map(|p| p.position).unwrap_or(Vec3::ZERO);
            let block = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            let biome = self
                .world
                .chunks
                .get(&(block[0] >> 4, block[2] >> 4))
                .map(|c| self.res.biomes.get(c.biome(block[0], block[1], block[2])).name.clone())
                .unwrap_or_default();
            let light = self.world.light(block[0], block[1], block[2]);
            let facing = match ((self.yaw.rem_euclid(360.0) + 45.0) / 90.0) as i32 % 4 {
                0 => "south (Towards positive Z)",
                1 => "west (Towards negative X)",
                2 => "north (Towards negative Z)",
                _ => "east (Towards positive X)",
            };
            let lines = [
                format!("Horizon 26.3 (Rust) {:.0} fps", self.fps),
                format!("C: {} chunks, {} sections drawn / {}", self.world.chunks.len(), self.renderer.as_ref().map_or(0, |r| r.drawn_sections), self.renderer.as_ref().map_or(0, |r| r.section_count())),
                format!("Pending jobs: {}", self.streaming.pending()),
                String::new(),
                format!("XYZ: {:.3} / {:.5} / {:.3}", p.x, p.y, p.z),
                format!("Block: {} {} {}", block[0], block[1], block[2]),
                format!("Chunk: {} {} {} in {} {} {}", block[0] & 15, block[1] & 15, block[2] & 15, block[0] >> 4, block[1] >> 4, block[2] >> 4),
                format!("Facing: {facing} ({:.1} / {:.1})", self.yaw.rem_euclid(360.0) - if self.yaw.rem_euclid(360.0) > 180.0 { 360.0 } else { 0.0 }, self.pitch),
                format!("Light: {} (sky {}, block {})", (light >> 4).max(light & 15), light >> 4, light & 15),
                format!("Biome: {biome}"),
                format!("Day {} time {}", self.world.day_time / 24000, self.world.day_time % 24000),
            ];
            for (i, line) in lines.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let tw = HudBatch::text_width(atlas, line);
                batch.fill(atlas, 1.0, 2.0 + i as f32 * 9.0 - 1.0, tw + 2.0, 9.0, rgba(80, 80, 80, 144));
                batch.text(atlas, line, 2.0, 2.0 + i as f32 * 9.0, rgba(224, 224, 224, 255), false);
            }
        }
        if let Some((text, at)) = &self.message
            && at.elapsed() < Duration::from_secs(3)
        {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, h - 59.0, u32::MAX, true);
        }
        self.draw_chat(&mut batch);
        let atlas = &self.res.atlas;
        if let Some((_, ticks)) = self.sleeping {
            // Gui sleep overlay fading in over 100 ticks.
            let a = ((ticks as f32 / 100.0).min(1.0) * 204.0) as u8;
            batch.fill(atlas, 0.0, 0.0, w, h, rgba(16, 16, 32, a));
        }
        match self.screen {
            Screen::Paused => self.draw_pause_menu(&mut batch),
            Screen::Dead => self.draw_death_screen(&mut batch),
            Screen::Inventory => {
                self.draw_inventory(&mut batch);
            }
            Screen::Creative => {
                self.draw_creative(&mut batch);
            }
            Screen::Chat => self.draw_chat_screen(&mut batch),
            Screen::Playing | Screen::Options => {}
        }
        batch
    }
    fn inventory_origin(&self, batch: &HudBatch) -> (f32, f32) {
        ((batch.width - 176.0) / 2.0, (batch.height - 166.0) / 2.0)
    }
    /// Slot ids: 0..36 player inventory, 100..109 crafting grid, 200 result.
    fn container_kind(&self) -> Option<&BlockEntity> {
        self.container.and_then(|k| self.world.block_entities.get(&k))
    }
    /// 300.. are the open container's slots.
    fn slots(&self) -> Vec<usize> {
        let mut v: Vec<usize> = (0..36).collect();
        match self.container_kind() {
            Some(BlockEntity::Chest(items)) => v.extend((0..items.len()).map(|i| 300 + i)),
            Some(BlockEntity::Furnace(_)) => v.extend(300..303),
            None => {
                v.extend((0..self.craft_size * self.craft_size).map(|i| 100 + i));
                v.push(200);
                if self.craft_size == 2 {
                    v.extend(500..504);
                }
            }
        }
        v
    }
    /// InventoryMenu / CraftingMenu slot layouts.
    fn slot_position(&self, i: usize) -> (f32, f32) {
        let table = self.craft_size == 3;
        // ContainerScreen (3 rows) shifts the player inventory down by one pixel.
        let shift = if matches!(self.container_kind(), Some(BlockEntity::Chest(_))) { 1.0 } else { 0.0 };
        match i {
            0..9 => (8.0 + i as f32 * 18.0, 142.0 + shift),
            9..36 => (8.0 + ((i - 9) % 9) as f32 * 18.0, 84.0 + shift + ((i - 9) / 9) as f32 * 18.0),
            500..504 => (8.0, 8.0 + (i - 500) as f32 * 18.0),
            300..500 => match self.container_kind() {
                Some(BlockEntity::Furnace(_)) => [(56.0, 17.0), (56.0, 53.0), (116.0, 35.0)][(i - 300).min(2)],
                _ => (8.0 + ((i - 300) % 9) as f32 * 18.0, 18.0 + ((i - 300) / 9) as f32 * 18.0),
            },
            200 => if table { (124.0, 35.0) } else { (154.0, 28.0) },
            _ => {
                let g = i - 100;
                let (r, c) = (g / self.craft_size, g % self.craft_size);
                if table {
                    (30.0 + c as f32 * 18.0, 17.0 + r as f32 * 18.0)
                } else {
                    (98.0 + c as f32 * 18.0, 18.0 + r as f32 * 18.0)
                }
            }
        }
    }
    fn slot_at(&self, batch: &HudBatch, x: f32, y: f32) -> Option<usize> {
        let (ox, oy) = self.inventory_origin(batch);
        self.slots().into_iter().find(|&i| {
            let (sx, sy) = self.slot_position(i);
            x >= ox + sx && x < ox + sx + 16.0 && y >= oy + sy && y < oy + sy + 16.0
        })
    }
    fn slot_mut(&mut self, i: usize) -> &mut Option<ItemStack> {
        if (500..504).contains(&i) {
            return &mut self.armor[i - 500];
        }
        if i >= 300 {
            let key = self.container;
            return match key.and_then(|k| self.world.block_entities.get_mut(&k)) {
                Some(e) => e.slots_mut().get_mut(i - 300).unwrap_or(&mut self.scratch_slot),
                None => &mut self.scratch_slot,
            };
        }
        if (100..109).contains(&i) {
            &mut self.craft_grid[i - 100]
        } else {
            &mut self.inventory[i]
        }
    }
    fn craft_result(&self) -> Option<ItemStack> {
        let n = self.craft_size;
        let grid: Vec<Option<ItemStack>> = (0..n * n).map(|i| self.craft_grid[i].clone()).collect();
        self.res.recipes.craft(&grid, n)
    }
    fn draw_inventory(&mut self, batch: &mut HudBatch) {
        let (ox, oy) = self.inventory_origin(batch);
        {
            let atlas = &self.res.atlas;
            batch.fill(atlas, 0.0, 0.0, batch.width, batch.height, rgba(16, 16, 16, 160));
            let part = |batch: &mut HudBatch, name: &str, sx: f32, sy: f32, w: f32, h: f32, dx: f32, dy: f32| {
                if let Some(mut r) = atlas.sprites.get(name).copied() {
                    let (du, dv) = ((r.u1 - r.u0) / r.w as f32, (r.v1 - r.v0) / r.h as f32);
                    r.u0 += sx * du;
                    r.v0 += sy * dv;
                    r.u1 = r.u0 + w * du;
                    r.v1 = r.v0 + h * dv;
                    batch.rect_uv(dx, dy, w, h, r, u32::MAX);
                }
            };
            let grey = rgba(64, 64, 64, 255);
            match self.container.and_then(|k| self.world.block_entities.get(&k)) {
                Some(BlockEntity::Chest(_)) => {
                    // ContainerScreen with 3 rows: top 71px, then the player inventory from v=126.
                    part(batch, "gui/container/generic_54", 0.0, 0.0, 176.0, 71.0, ox, oy);
                    part(batch, "gui/container/generic_54", 0.0, 126.0, 176.0, 96.0, ox, oy + 71.0);
                    let name = self.container.map(|k| self.world.block(k.0, k.1, k.2)).map(|s| self.res.facts.get(s).name.clone());
                    let title = if name.as_deref() == Some("minecraft:barrel") { "Barrel" } else { "Chest" };
                    batch.text(atlas, title, ox + 8.0, oy + 6.0, grey, false);
                    batch.text(atlas, "Inventory", ox + 8.0, oy + 73.0, grey, false);
                }
                Some(BlockEntity::Furnace(f)) => {
                    let (tex, title) = match f.kind {
                        horizon_client::containers::FurnaceKind::Furnace => ("gui/container/furnace", "Furnace"),
                        horizon_client::containers::FurnaceKind::BlastFurnace => ("gui/container/blast_furnace", "Blast Furnace"),
                        horizon_client::containers::FurnaceKind::Smoker => ("gui/container/smoker", "Smoker"),
                    };
                    part(batch, tex, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    let tw = HudBatch::text_width(atlas, title);
                    batch.text(atlas, title, ox + (176.0 - tw) / 2.0, oy + 6.0, grey, false);
                    batch.text(atlas, "Inventory", ox + 8.0, oy + 72.0, grey, false);
                    if f.lit_remaining > 0 {
                        // AbstractFurnaceScreen: flame height ceil(lit * 13) + 1.
                        let total = if f.lit_total == 0 { 200 } else { f.lit_total };
                        let h = ((f.lit_remaining as f32 / total as f32) * 13.0).ceil() + 1.0;
                        part(batch, "gui/sprites/container/furnace/lit_progress", 0.0, 14.0 - h, 14.0, h, ox + 56.0, oy + 36.0 + 14.0 - h);
                    }
                    if f.cooking_timer > 0 && f.cooking_total > 0 {
                        let w = ((f.cooking_timer as f32 / f.cooking_total as f32) * 24.0).ceil();
                        part(batch, "gui/sprites/container/furnace/burn_progress", 0.0, 0.0, w, 16.0, ox + 79.0, oy + 34.0);
                    }
                }
                None => {
                    let texture = if self.craft_size == 3 { "gui/container/crafting_table" } else { "gui/container/inventory" };
                    part(batch, texture, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    let (title, tx) = if self.craft_size == 3 { ("Crafting", 29.0) } else { ("Crafting", 97.0) };
                    batch.text(atlas, title, ox + tx, oy + if self.craft_size == 3 { 6.0 } else { 8.0 }, grey, false);
                    if self.craft_size == 3 {
                        batch.text(atlas, "Inventory", ox + 8.0, oy + 72.0, grey, false);
                    }
                }
            }
        }
        for i in self.slots() {
            let (sx, sy) = self.slot_position(i);
            if (500..504).contains(&i) && self.armor[i - 500].is_none() {
                let icon = ["helmet", "chestplate", "leggings", "boots"][i - 500];
                batch.sprite(&self.res.atlas, &format!("gui/sprites/container/slot/{icon}"), ox + sx, oy + sy);
            }
            let stack = if i == 200 { self.craft_result() } else { self.slot_mut(i).clone() };
            if let Some(stack) = stack {
                self.draw_item(batch, &stack, ox + sx, oy + sy);
            }
        }
        if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
            let (sx, sy) = self.slot_position(slot);
            batch.fill(&self.res.atlas, ox + sx, oy + sy, 16.0, 16.0, rgba(255, 255, 255, 128));
        }
        if let Some(stack) = self.carried.clone() {
            self.draw_item(batch, &stack, self.cursor.0 - 8.0, self.cursor.1 - 8.0);
        }
    }
    fn close_container(&mut self) {
        let mut returned: Vec<ItemStack> = self.carried.take().into_iter().collect();
        returned.extend(self.craft_grid.iter_mut().filter_map(Option::take));
        for stack in returned {
            if let Some(rest) = self.give(stack)
                && let Some(eye) = self.eye_position()
            {
                self.spawn_item(rest, [eye[0], eye[1] - 0.3, eye[2]], [0.0, 0.1, 0.0], 40);
            }
        }
        self.craft_size = 2;
        if let Some(key) = self.container.take() {
            let name = self.res.facts.get(self.world.block(key.0, key.1, key.2)).name.clone();
            let sound = if name.contains("chest") { Some("minecraft:block.chest.close") } else if &*name == "minecraft:barrel" { Some("minecraft:block.barrel.close") } else { None };
            if let Some(sound) = sound {
                self.audio.play(sound, Some([f64::from(key.0) + 0.5, f64::from(key.1) + 0.5, f64::from(key.2) + 0.5]), 0.5, 1.0);
            }
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
    }
    fn click_inventory(&mut self, right: bool) {
        let Some(size) = self.renderer.as_ref().map(|r| [r.config.width, r.config.height]) else { return };
        let batch = HudBatch::new(size);
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else { return };
        let max = |items: &Items, s: &ItemStack| items.get(&s.item).map_or(64, |i| i.max_stack);
        let shift = self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        if slot >= 300 {
            if let Some(key) = self.container {
                self.world.modified.insert((key.0 >> 4, key.2 >> 4));
            }
        }
        let is_furnace = matches!(self.container_kind(), Some(BlockEntity::Furnace(_)));
        if is_furnace && slot == 302 {
            // FurnaceResultSlot: take only.
            let Some(result) = self.slot_mut(302).take() else { return };
            match &mut self.carried {
                None => self.carried = Some(result),
                Some(c) if c.item == result.item && c.count + result.count <= max(&self.res.items, c) => c.count += result.count,
                _ => *self.slot_mut(302) = Some(result),
            }
            return;
        }
        if is_furnace
            && slot == 300
            && let Some(BlockEntity::Furnace(f)) = self.container.and_then(|k| self.world.block_entities.get_mut(&k))
        {
            // Changing the input restarts cooking (AbstractFurnaceBlockEntity.setItem).
            f.cooking_timer = 0;
        }
        if slot == 200 {
            // ResultSlot: take the crafted stack and consume one of each ingredient.
            let mut crafted = 0;
            while let Some(result) = self.craft_result() {
                if shift {
                    if self.give(result).is_some() {
                        break;
                    }
                } else {
                    match &mut self.carried {
                        None => self.carried = Some(result),
                        Some(c) if c.item == result.item && c.count + result.count <= max(&self.res.items, c) => c.count += result.count,
                        _ => return,
                    }
                }
                for cell in self.craft_grid.iter_mut() {
                    if let Some(s) = cell {
                        s.count -= 1;
                        if s.count == 0 {
                            *cell = None;
                        }
                    }
                }
                crafted += 1;
                if !shift || crafted >= 64 {
                    break;
                }
            }
            return;
        }
        if shift && slot < 36 {
            // Quick move between hotbar and main inventory.
            if let Some(stack) = self.inventory[slot].take() {
                let range = if slot < 9 { 9..36 } else { 0..9 };
                let mut stack = stack;
                for i in range.clone() {
                    if let Some(s) = &mut self.inventory[i]
                        && s.item == stack.item
                        && s.damage == stack.damage
                    {
                        let moved = max(&self.res.items, s).saturating_sub(s.count).min(stack.count);
                        s.count += moved;
                        stack.count -= moved;
                    }
                }
                if stack.count > 0
                    && let Some(i) = range.clone().find(|&i| self.inventory[i].is_none())
                {
                    self.inventory[i] = Some(stack);
                    stack = ItemStack { count: 0, ..self.inventory[i].clone().unwrap() };
                }
                if stack.count > 0 {
                    self.inventory[slot] = Some(stack);
                }
            }
            return;
        }
        if (500..504).contains(&slot) {
            let wanted = ["head", "chest", "legs", "feet"][slot - 500];
            let fits = self.carried.as_ref().is_none_or(|c| self.res.items.get(&c.item).and_then(|i| i.equip_slot.as_deref()) == Some(wanted));
            if !fits {
                return;
            }
            std::mem::swap(&mut self.carried, &mut self.armor[slot - 500]);
            self.apply_equipment();
            return;
        }
        let carried = self.carried.take();
        let current = self.slot_mut(slot).take();
        let items = &self.res.items;
        let (new_slot, new_carried) = match (carried, current) {
            (None, Some(mut s)) if right => {
                let half = s.count.div_ceil(2);
                s.count -= half;
                let taken = ItemStack { count: half, ..s.clone() };
                ((s.count > 0).then_some(s), Some(taken))
            }
            (None, s) => (None, s),
            (Some(mut c), None) if right => {
                let one = ItemStack { count: 1, ..c.clone() };
                c.count -= 1;
                (Some(one), (c.count > 0).then_some(c))
            }
            (Some(c), None) => (Some(c), None),
            (Some(mut c), Some(mut s)) if c.item == s.item && c.damage == s.damage => {
                let room = max(items, &s).saturating_sub(s.count);
                let moved = if right { 1.min(room).min(c.count) } else { room.min(c.count) };
                s.count += moved;
                c.count -= moved;
                (Some(s), (c.count > 0).then_some(c))
            }
            (Some(c), Some(s)) => (Some(c), Some(s)),
        };
        *self.slot_mut(slot) = new_slot;
        self.carried = new_carried;
    }
    /// ItemEntityRenderer: items bob and spin; blocks render as small cubes.
    fn entity_geometry(&mut self, camera: glam::DVec3, alpha: f64) -> Vec<Vertex> {
        let mut out = Vec::new();
        let items = self.dropped.clone();
        for item in &items {
            let p = item.previous;
            let q = item.position;
            let pos = glam::DVec3::new(
                p.x + (q.x - p.x) * alpha,
                p.y + (q.y - p.y) * alpha,
                p.z + (q.z - p.z) * alpha,
            ) - camera;
            if pos.length_squared() > 64.0 * 64.0 {
                continue;
            }
            let age = item.age as f32 + alpha as f32;
            let bob = (age / 10.0 + item.bob_offset).sin() * 0.1 + 0.1;
            let spin = age / 20.0 + item.bob_offset;
            let light = self.world.light(q.x.floor() as i32, (q.y + 0.1).floor() as i32, q.z.floor() as i32);
            let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
            let center = pos.as_vec3() + glam::Vec3::new(0.0, bob + 0.125, 0.0);
            let rot = glam::Mat3::from_rotation_y(spin);
            let icon = self.icon(&item.stack.item);
            let atlas = &self.res.atlas;
            let mut quad = |corners: [glam::Vec3; 4], name: &str, shade: f32, tint: u32| {
                let Some(r) = atlas.sprites.get(name).copied() else { return };
                let uv = [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
                let c = |sft: u32| (((tint >> sft) & 255) as f32 * shade) as u32;
                let color = 0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16);
                for (corner, uv) in corners.iter().zip(uv) {
                    let v = center + rot * *corner;
                    out.push(Vertex { pos: v.to_array(), uv, layer: 0, color, light: packed });
                }
            };
            let copies = match item.stack.count {
                0..=1 => 1,
                2..=16 => 2,
                17..=32 => 3,
                _ => 4,
            };
            for k in 0..copies {
                let o = glam::Vec3::new(k as f32 * 0.03, k as f32 * 0.03, k as f32 * 0.05);
                match &icon {
                    Some(Icon::Cube { top, left, right, tint }) => {
                        let h = 0.125;
                        let v = |x: f32, y: f32, z: f32| glam::Vec3::new(x * h, y * h, z * h) + o;
                        quad([v(-1., 1., 1.), v(1., 1., 1.), v(1., 1., -1.), v(-1., 1., -1.)], top, 1.0, *tint);
                        quad([v(-1., -1., -1.), v(1., -1., -1.), v(1., -1., 1.), v(-1., -1., 1.)], top, 0.5, *tint);
                        quad([v(-1., -1., 1.), v(1., -1., 1.), v(1., 1., 1.), v(-1., 1., 1.)], left, 0.8, 0xFFFFFF);
                        quad([v(1., -1., -1.), v(-1., -1., -1.), v(-1., 1., -1.), v(1., 1., -1.)], left, 0.8, 0xFFFFFF);
                        quad([v(1., -1., 1.), v(1., -1., -1.), v(1., 1., -1.), v(1., 1., 1.)], right, 0.6, 0xFFFFFF);
                        quad([v(-1., -1., -1.), v(-1., -1., 1.), v(-1., 1., 1.), v(-1., 1., -1.)], right, 0.6, 0xFFFFFF);
                    }
                    Some(Icon::Flat(name)) => {
                        let h = 0.25;
                        let v = |x: f32, y: f32| glam::Vec3::new(x * h, y * h + 0.125, 0.0) + o;
                        quad([v(-1., -1.), v(1., -1.), v(1., 1.), v(-1., 1.)], name, 1.0, 0xFFFFFF);
                    }
                    None => {}
                }
            }
        }
        // Mobs.
        for mob in &self.mobs {
            let d = glam::DVec3::new(mob.position.x, mob.position.y, mob.position.z) - camera;
            if d.length_squared() > 128.0 * 128.0 {
                continue;
            }
            let Some(r) = self.res.atlas.sprites.get(&mob.kind.texture).copied() else { continue };
            let light = self.world.light(mob.position.x.floor() as i32, (mob.position.y + mob.kind.eye_height).floor() as i32, mob.position.z.floor() as i32);
            let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
            horizon_client::mobs::render(mob, alpha as f32, camera, [r.u0, r.v0, r.u1, r.v1], packed, &mut out);
        }
        // Arrows: two crossed quads along the flight direction.
        if let Some(r) = self.res.atlas.sprites.get("entity/projectiles/arrow").copied() {
            for a in &self.arrows {
                let p = glam::DVec3::new(
                    a.previous.x + (a.position.x - a.previous.x) * alpha,
                    a.previous.y + (a.position.y - a.previous.y) * alpha,
                    a.previous.z + (a.position.z - a.previous.z) * alpha,
                ) - camera;
                let dir = glam::Vec3::new(a.velocity.x as f32, a.velocity.y as f32, a.velocity.z as f32).normalize_or(glam::Vec3::X);
                let side = dir.cross(glam::Vec3::Y).normalize_or(glam::Vec3::Z) * 0.08;
                let up = dir.cross(side).normalize_or(glam::Vec3::Y) * 0.08;
                let c = p.as_vec3();
                let (u1, v1) = (r.u0 + (r.u1 - r.u0) * 0.5, r.v0 + (r.v1 - r.v0) * (5.0 / 32.0));
                for off in [side, up] {
                    let q = [c - dir * 0.25 - off, c + dir * 0.25 - off, c + dir * 0.25 + off, c - dir * 0.25 + off];
                    for (v, uv) in q.iter().zip([[r.u0, v1], [u1, v1], [u1, r.v0], [r.u0, r.v0]]) {
                        out.push(Vertex { pos: v.to_array(), uv, layer: 0, color: 0xFFFF_FFFF, light: 240 | 240 << 8 });
                    }
                }
            }
        }
        // Chests (ChestRenderer): rotate by facing around the block centre, lift the lid.
        if let Some(model) = &self.res.chest_model {
            for (pos, entity) in &self.world.block_entities {
                if !matches!(entity, BlockEntity::Chest(_)) {
                    continue;
                }
                let state = self.world.block(pos.0, pos.1, pos.2);
                let name = &self.res.facts.get(state).name;
                let texture = match &**name {
                    "minecraft:chest" => "entity/chest/normal",
                    "minecraft:trapped_chest" => "entity/chest/trapped",
                    _ => continue,
                };
                let origin = glam::DVec3::new(f64::from(pos.0), f64::from(pos.1), f64::from(pos.2)) - camera;
                if origin.length_squared() > 64.0 * 64.0 {
                    continue;
                }
                let Some(r) = self.res.atlas.sprites.get(texture).copied() else { continue };
                let facing = self.res.assets.registry.state(state).and_then(|s| s.properties.get("facing").cloned()).unwrap_or_default();
                let yrot: f32 = match facing.as_str() {
                    "west" => 90.0,
                    "north" => 180.0,
                    "east" => 270.0,
                    _ => 0.0,
                };
                let (prev, cur) = self.chest_open.get(pos).copied().unwrap_or((0.0, 0.0));
                let o = prev + (cur - prev) * alpha as f32;
                let open = 1.0 - (1.0 - o).powi(3);
                let base = glam::Mat4::from_translation(origin.as_vec3())
                    * glam::Mat4::from_translation(glam::Vec3::new(0.5, 0.0, 0.5))
                    * glam::Mat4::from_rotation_y((-yrot).to_radians())
                    * glam::Mat4::from_translation(glam::Vec3::new(-0.5, 0.0, -0.5));
                let light = self.world.light(pos.0, pos.1, pos.2);
                let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
                let pose = |part: &horizon_client::mobs::Part| {
                    let mut p = part.pose;
                    if part.name == "lid" || part.name == "lock" {
                        p[3] = -(open * std::f32::consts::FRAC_PI_2);
                    }
                    p
                };
                horizon_client::mobs::render_model(model, base, &pose, [r.u0, r.v0, r.u1, r.v1], packed, 0, &mut out);
            }
        }
        // Experience orbs (ExperienceOrbRenderer): pulsing billboards from a 4x4 icon sheet.
        if let Some(r) = self.res.atlas.sprites.get("entity/experience_orb").copied() {
            let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
            let forward = glam::Vec3::new(-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos());
            let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
            let up = right.cross(forward).normalize_or_zero();
            for o in &self.orbs {
                let p = glam::DVec3::new(
                    o.previous.x + (o.position.x - o.previous.x) * alpha,
                    o.previous.y + (o.position.y - o.previous.y) * alpha + 0.1,
                    o.previous.z + (o.position.z - o.previous.z) * alpha,
                ) - camera;
                let icon = o.icon();
                let (du, dv) = ((r.u1 - r.u0) / 4.0, (r.v1 - r.v0) / 4.0);
                let (u0, v0) = (r.u0 + (icon % 4) as f32 * du, r.v0 + (icon / 4) as f32 * dv);
                let t = (o.age as f32 + alpha as f32) / 2.0;
                let red = ((t.sin() + 1.0) * 0.5 * 255.0) as u32;
                let blue = (((t + 4.188_79).sin() + 1.0) * 0.1 * 255.0) as u32;
                let color = 0x8000_0000 | blue << 16 | 255 << 8 | red;
                let light = self.world.light(o.position.x.floor() as i32, o.position.y.floor() as i32, o.position.z.floor() as i32);
                let packed = u32::from(light >> 4).max(10) * 16 | (u32::from(light & 15).max(10) * 16) << 8;
                let c = p.as_vec3();
                let s = 0.15;
                for (k, uv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().zip([[u0, v0 + dv], [u0 + du, v0 + dv], [u0 + du, v0], [u0, v0]]) {
                    let v = c + right * (k.0 * s) + up * (k.1 * s);
                    out.push(Vertex { pos: v.to_array(), uv, layer: 0, color, light: packed });
                }
            }
        }
        // Falling blocks: a unit cube with the block's texture.
        for f in self.falling.clone() {
            let Some((sprite, tint)) = self.particle_sprite(f.state) else { continue };
            let Some(r) = self.res.atlas.sprites.get(&sprite).copied() else { continue };
            let p = glam::DVec3::new(
                f.previous.x + (f.position.x - f.previous.x) * alpha - 0.5,
                f.previous.y + (f.position.y - f.previous.y) * alpha,
                f.previous.z + (f.position.z - f.previous.z) * alpha - 0.5,
            ) - camera;
            let o = p.as_vec3();
            let light = self.world.light(f.position.x.floor() as i32, f.position.y.floor() as i32 + 1, f.position.z.floor() as i32);
            let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
            let v = |x: f32, y: f32, z: f32| o + glam::Vec3::new(x, y, z);
            let faces = [
                ([v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.), v(0., 1., 0.)], 1.0),
                ([v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)], 0.5),
                ([v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)], 0.8),
                ([v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)], 0.8),
                ([v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)], 0.6),
                ([v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)], 0.6),
            ];
            for (corners, shade) in faces {
                let c = |sft: u32| (((tint >> sft) & 255) as f32 * shade) as u32;
                let color = 0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16);
                for (p, uv) in corners.iter().zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]]) {
                    out.push(Vertex { pos: p.to_array(), uv, layer: 0, color, light: packed });
                }
            }
        }
        // Camera-facing particle quads.
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let forward = glam::Vec3::new(-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos());
        let right = forward.cross(glam::Vec3::Y).normalize_or_zero();
        let up = right.cross(forward).normalize_or_zero();
        let atlas = &self.res.atlas;
        for p in &self.particles {
            let Some(r) = atlas.sprites.get(&p.sprite).copied() else { continue };
            let a = alpha;
            let pos = glam::DVec3::new(
                p.previous[0] + (p.position[0] - p.previous[0]) * a,
                p.previous[1] + (p.position[1] - p.previous[1]) * a,
                p.previous[2] + (p.position[2] - p.previous[2]) * a,
            ) - camera;
            let c = pos.as_vec3();
            let light = self.world.light(p.position[0].floor() as i32, p.position[1].floor() as i32, p.position[2].floor() as i32);
            let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
            // TerrainParticle colour: 0.6 base brightness times the block tint.
            let ch = |sft: u32| (((p.tint >> sft) & 255) as f32 * 0.6) as u32;
            let color = 0xFF00_0000 | ch(0) << 16 | ch(8) << 8 | ch(16);
            let (du, dv) = (r.u1 - r.u0, r.v1 - r.v0);
            let (u0, v0, u1, v1) = (r.u0 + du * p.uv[0], r.v0 + dv * p.uv[1], r.u0 + du * p.uv[2], r.v0 + dv * p.uv[3]);
            let s = p.size;
            for (k, uv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)].iter().zip([[u0, v1], [u1, v1], [u1, v0], [u0, v0]]) {
                let v = c + right * (k.0 * s) + up * (k.1 * s);
                out.push(Vertex { pos: v.to_array(), uv, layer: 0, color, light: packed });
            }
        }
        // Breaking progress overlay (destroy_stage_0..9).
        if let Some(m) = &self.mining
            && m.progress > 0.0
        {
            let stage = ((m.progress * 10.0) as u32).min(9);
            if let Some(r) = atlas.sprites.get(&format!("block/destroy_stage_{stage}")).copied() {
                let o = (glam::DVec3::new(f64::from(m.pos[0]), f64::from(m.pos[1]), f64::from(m.pos[2])) - camera).as_vec3();
                let e = 0.002;
                let corner = |x: f32, y: f32, z: f32| o + glam::Vec3::new(x * (1.0 + 2.0 * e) - e, y * (1.0 + 2.0 * e) - e, z * (1.0 + 2.0 * e) - e);
                let faces = [
                    [corner(0., 1., 0.), corner(1., 1., 0.), corner(1., 1., 1.), corner(0., 1., 1.)],
                    [corner(0., 0., 1.), corner(1., 0., 1.), corner(1., 0., 0.), corner(0., 0., 0.)],
                    [corner(0., 0., 0.), corner(1., 0., 0.), corner(1., 1., 0.), corner(0., 1., 0.)],
                    [corner(1., 0., 1.), corner(0., 0., 1.), corner(0., 1., 1.), corner(1., 1., 1.)],
                    [corner(0., 0., 1.), corner(0., 0., 0.), corner(0., 1., 0.), corner(0., 1., 1.)],
                    [corner(1., 0., 0.), corner(1., 0., 1.), corner(1., 1., 1.), corner(1., 1., 0.)],
                ];
                for f in faces {
                    for (v, uv) in f.iter().zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]]) {
                        out.push(Vertex { pos: v.to_array(), uv, layer: 0, color: 0xB4FF_FFFF, light: 15 * 16 | (15 * 16) << 8 });
                    }
                }
            }
        }
        out
    }

    fn sky_darken(&self) -> i32 {
        if self.dim == Dim::Nether {
            return 11;
        }
        let mut sky = self.res.environment.sample(self.world.day_time, 0, 0);
        let (rain, thunder) = self.world.weather.levels(1.0);
        horizon_client::environment::apply_weather(&mut sky, rain, thunder);
        // Level.updateSkyBrightness: darkening from the sky light level.
        (15.0 - 15.0 * sky.sky_light_level).round() as i32
    }
    fn weighted<'b>(&mut self, entries: &'b [horizon_client::biome::SpawnEntry]) -> Option<&'b horizon_client::biome::SpawnEntry> {
        let supported: Vec<&horizon_client::biome::SpawnEntry> =
            entries.iter().filter(|e| self.res.mob_types.contains_key(&e.entity)).collect();
        let total: u32 = supported.iter().map(|e| e.weight).sum();
        if total == 0 {
            return None;
        }
        let mut r = self.world.random.next_int_bound(total as i32).ok()? as u32;
        supported.into_iter().find(|e| {
            if r < e.weight {
                true
            } else {
                r -= e.weight;
                false
            }
        })
    }
    fn add_mob(&mut self, kind: &str, x: f64, y: f64, z: f64, persistent: bool) {
        let Some(t) = self.res.mob_types.get(kind).cloned() else { return };
        self.next_entity += 1;
        let yaw = self.world.random.next_float() * 360.0;
        let mut mob = Mob::new(self.next_entity, t, Vec3::new(x, y, z), yaw);
        mob.persistent = persistent;
        self.mobs.push(mob);
    }
    /// NaturalSpawner.spawnMobsForChunkGeneration for the creature category.
    fn spawn_chunk_creatures(&mut self, cx: i32, cz: i32) {
        let Some(chunk) = self.world.chunks.get(&(cx, cz)).cloned() else { return };
        let biome = self.res.biomes.get(chunk.biome(8, 64, 8)).clone();
        let Some(entries) = biome.spawns.get("creature").cloned() else { return };
        while self.world.random.next_float() < biome.creature_probability {
            let Some(entry) = self.weighted(&entries).cloned() else { return };
            let count = entry.min + self.world.random.next_int_bound((entry.max - entry.min + 1) as i32).unwrap_or(0) as u32;
            let mut x = cx * 16 + self.world.random.next_int_bound(16).unwrap_or(0);
            let mut z = cz * 16 + self.world.random.next_int_bound(16).unwrap_or(0);
            for _ in 0..count {
                if let Some(y) = self.land_column(x, z) {
                    let below = self.world.block(x, y - 1, z);
                    if self.res.facts.get(below).name.contains("grass") {
                        self.add_mob(&entry.entity, f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5, true);
                    }
                }
                x = (x + self.world.random.next_int_bound(5).unwrap_or(0) - self.world.random.next_int_bound(5).unwrap_or(0)).clamp(cx * 16, cx * 16 + 15);
                z = (z + self.world.random.next_int_bound(5).unwrap_or(0) - self.world.random.next_int_bound(5).unwrap_or(0)).clamp(cz * 16, cz * 16 + 15);
            }
        }
    }
    /// NaturalSpawner.spawnCategoryForPosition for monsters near the player.
    fn spawn_monsters(&mut self) {
        let Some(p) = self.player.as_ref().map(|p| p.position) else { return };
        if self.creative && false {
            return;
        }
        let monsters = self.mobs.iter().filter(|m| m.hostile()).count();
        if monsters >= 70 {
            return;
        }
        let r = &mut self.world.random;
        let cx = (p.x.floor() as i32 >> 4) + r.next_int_bound(17).unwrap_or(8) - 8;
        let cz = (p.z.floor() as i32 >> 4) + r.next_int_bound(17).unwrap_or(8) - 8;
        let Some(chunk) = self.world.chunks.get(&(cx, cz)).cloned() else { return };
        let x = cx * 16 + r.next_int_bound(16).unwrap_or(0);
        let z = cz * 16 + r.next_int_bound(16).unwrap_or(0);
        let top = self.land_column(x, z).unwrap_or(MIN_Y + 1).max(MIN_Y + 1);
        let y = MIN_Y + self.world.random.next_int_bound((top - MIN_Y + 1).max(1)).unwrap_or(0);
        let biome = self.res.biomes.get(chunk.biome(x, y, z)).clone();
        let Some(entries) = biome.spawns.get("monster").cloned() else { return };
        let Some(entry) = self.weighted(&entries).cloned() else { return };
        let darken = self.sky_darken();
        let pack = entry.min + self.world.random.next_int_bound((entry.max - entry.min + 1) as i32).unwrap_or(0) as u32;
        let (mut sx, mut sz) = (x, z);
        for _ in 0..pack.min(4) {
            sx += self.world.random.next_int_bound(6).unwrap_or(0) - self.world.random.next_int_bound(6).unwrap_or(0);
            sz += self.world.random.next_int_bound(6).unwrap_or(0) - self.world.random.next_int_bound(6).unwrap_or(0);
            let (fx, fy, fz) = (f64::from(sx) + 0.5, f64::from(y), f64::from(sz) + 0.5);
            let d2 = (fx - p.x).powi(2) + (fy - p.y).powi(2) + (fz - p.z).powi(2);
            if d2 < 24.0 * 24.0 || d2 > 128.0 * 128.0 {
                continue;
            }
            let free = |g: &Self, yy: i32| {
                let f = g.res.facts.get(g.world.block(sx, yy, sz));
                !f.full_collision && f.fluid == Fluid::None
            };
            let below = self.res.facts.get(self.world.block(sx, y - 1, sz));
            if !below.full_collision || below.fluid != Fluid::None || !free(self, y) || !free(self, y + 1) {
                continue;
            }
            // Monster.isDarkEnoughToSpawn: sky light vs random(32), then
            // block light 0 and darkened brightness within uniform(0, 7).
            let light = self.world.light(sx, y, sz);
            let (sky, block) = (i32::from(light >> 4), i32::from(light & 15));
            if sky > self.world.random.next_int_bound(32).unwrap_or(0) || block > 0 {
                continue;
            }
            if (sky - darken).max(0) > self.world.random.next_int_bound(8).unwrap_or(0) {
                continue;
            }
            self.add_mob(&entry.entity, fx, fy, fz, false);
        }
    }
    fn mob_hit(&self) -> Option<(usize, f64)> {
        let eye = self.eye_position()?;
        let dir = view_vector(self.yaw, self.pitch);
        let reach: f64 = if self.creative { 5.0 } else { 3.0 };
        let mut best: Option<(usize, f64)> = None;
        for (i, m) in self.mobs.iter().enumerate() {
            if !m.alive() {
                continue;
            }
            let b = m.bounds();
            let (min, max) = (b.min_position(), b.max_position());
            let (lo, hi) = ([min.x, min.y, min.z], [max.x, max.y, max.z]);
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for a in 0..3 {
                if dir[a].abs() < 1e-9 {
                    if eye[a] < lo[a] || eye[a] > hi[a] {
                        hit = false;
                    }
                    continue;
                }
                let (mut ta, mut tb) = ((lo[a] - eye[a]) / dir[a], (hi[a] - eye[a]) / dir[a]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((i, t0));
            }
        }
        best
    }
    /// Player.attack with the attack-strength charge, crits and knockback.
    fn attack(&mut self) -> bool {
        let Some((index, distance)) = self.mob_hit() else { return false };
        if let Some(hit) = self.pick() {
            let eye = self.eye_position().unwrap_or_default();
            let d = ((hit.point[0] - eye[0]).powi(2) + (hit.point[1] - eye[1]).powi(2) + (hit.point[2] - eye[2]).powi(2)).sqrt();
            if d < distance {
                return false;
            }
        }
        let held = self.held().cloned();
        let modifiers = held.as_ref().and_then(|h| self.res.items.get(&h.item)).map(|i| (i.attack_damage, i.attack_speed)).unwrap_or((0.0, 0.0));
        let damage = 1.0 + modifiers.0;
        let speed = (4.0 + modifiers.1).max(0.1);
        let delay = 20.0 / speed;
        let scale = ((self.attack_ticks as f32 + 0.5) / delay).clamp(0.0, 1.0);
        let mut amount = damage * (0.2 + scale * scale * 0.8);
        let player = self.player.as_ref();
        let falling = player.is_some_and(|p| !p.ground.on_ground && p.velocity.y < 0.0 && !p.sprinting);
        let crit = scale > 0.9 && falling;
        if crit {
            amount *= 1.5;
        }
        self.attack_ticks = 0;
        if let Some(p) = self.player.as_mut()
            && !self.creative
        {
            p.health.food.exhaustion = (p.health.food.exhaustion + 0.1).min(40.0);
        }
        let player = self.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        self.swing_arm();
        let mob = &mut self.mobs[index];
        let center = [mob.position.x, mob.position.y + mob.kind.height / 2.0, mob.position.z];
        if mob.hurt(amount, from, &mut self.world.random) {
            self.last_hurt_by_player.insert(mob.id);
            let kind = mob.kind.clone();
            let dead = !mob.alive();
            let sound = if crit { "minecraft:entity.player.attack.crit" } else if scale > 0.9 { "minecraft:entity.player.attack.strong" } else { "minecraft:entity.player.attack.weak" };
            self.audio.play(sound, Some(center), 1.0, 1.0);
            let event = format!("minecraft:{}", kind.sounds[if dead { 2 } else { 1 }]);
            let pitch = (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
            self.audio.play(&event, Some(center), 1.0, pitch);
            if let Some(stack) = self.inventory[self.selected].as_mut()
                && let Some(info) = self.res.items.get(&stack.item)
                && info.max_damage > 0
            {
                stack.damage += 1;
                if stack.damage >= info.max_damage {
                    self.inventory[self.selected] = None;
                }
            }
        }
        true
    }
    /// ServerExplosion: rays from a 16^3 grid lose (resistance + 0.3) * 0.3 per step.
    fn explode(&mut self, center: [f64; 3], radius: f32) -> Result<()> {
        let mut destroyed = HashSet::new();
        for j in 0..16 {
            for k in 0..16 {
                for l in 0..16 {
                    if !(j == 0 || j == 15 || k == 0 || k == 15 || l == 0 || l == 15) {
                        continue;
                    }
                    let mut d = [f64::from(j) / 15.0 * 2.0 - 1.0, f64::from(k) / 15.0 * 2.0 - 1.0, f64::from(l) / 15.0 * 2.0 - 1.0];
                    let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
                    d = d.map(|v| v / len);
                    let mut h = radius * (0.7 + self.world.random.next_float() * 0.6);
                    let mut p = center;
                    while h > 0.0 {
                        let b = [p[0].floor() as i32, p[1].floor() as i32, p[2].floor() as i32];
                        let state = self.world.block(b[0], b[1], b[2]);
                        if !self.res.facts.is_air(state) {
                            let f = self.res.facts.get(state);
                            let resistance = if f.fluid != Fluid::None && !f.waterlogged { 100.0 } else { f.explosion_resistance };
                            h -= (resistance + 0.3) * 0.3;
                            if h > 0.0 && f.hardness >= 0.0 {
                                destroyed.insert(b);
                            }
                        }
                        p = [p[0] + d[0] * 0.3, p[1] + d[1] * 0.3, p[2] + d[2] * 0.3];
                        h -= 0.225_000_01;
                    }
                }
            }
        }
        for b in destroyed {
            let state = self.world.block(b[0], b[1], b[2]);
            if self.res.facts.is_air(state) {
                continue;
            }
            // Explosion decay: each drop survives with probability 1 / radius.
            if self.world.random.next_float() < 1.0 / radius {
                let drops = self.res.items.block_drops(&self.res.assets.registry, state, None, &mut self.world.random);
                for d in drops {
                    self.pop_resource(b, d);
                }
            }
            self.world.set(b[0], b[1], b[2], self.world.air)?;
        }
        self.audio.play("minecraft:entity.generic.explode", Some(center), 4.0, (1.0 + (self.world.random.next_float() - self.world.random.next_float()) * 0.2) * 0.7);
        // Entity damage by distance (exposure approximated as unobstructed).
        let diameter = f64::from(radius) * 2.0;
        let mut armor_damage = None;
        if let Some(player) = self.player.as_mut() {
            let p = [player.position.x, player.position.y, player.position.z];
            let dist = ((p[0] - center[0]).powi(2) + (p[1] - center[1]).powi(2) + (p[2] - center[2]).powi(2)).sqrt() / diameter;
            if dist <= 1.0 {
                let impact = 1.0 - dist;
                let damage = ((impact * impact + impact) / 2.0 * 7.0 * diameter + 1.0) as f32;
                if let Some(id) = self.motion.health.damage.id("minecraft:explosion") {
                    let _ = player.hurt(&self.motion.health, self.world.health_rules, id, damage);
                    armor_damage = Some(damage);
                }
                let (dx, dy, dz) = (p[0] - center[0], p[1] - center[1], p[2] - center[2]);
                let l = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-4);
                player.velocity = Vec3::new(player.velocity.x + dx / l * impact, player.velocity.y + dy / l * impact, player.velocity.z + dz / l * impact);
            }
        }
        if let Some(d) = armor_damage
            && !self.creative
        {
            self.hurt_armor(d);
        }
        for m in &mut self.mobs {
            let dist = ((m.position.x - center[0]).powi(2) + (m.position.y - center[1]).powi(2) + (m.position.z - center[2]).powi(2)).sqrt() / diameter;
            if dist <= 1.0 && m.alive() {
                let impact = 1.0 - dist;
                m.hurt(((impact * impact + impact) / 2.0 * 7.0 * diameter + 1.0) as f32, Some(center), &mut self.world.random);
            }
        }
        Ok(())
    }
    fn tick_block_entities(&mut self) -> Result<()> {
        // ChestLidController: 0.1 per tick towards open/closed.
        if let Some(key) = self.container {
            self.chest_open.entry(key).or_insert((0.0, 0.0));
        }
        let open = self.container;
        self.chest_open.retain(|k, (prev, cur)| {
            *prev = *cur;
            *cur = if Some(*k) == open { (*cur + 0.1).min(1.0) } else { (*cur - 0.1).max(0.0) };
            *cur > 0.0 || *prev > 0.0
        });
        let mut lit_changes = Vec::new();
        for (pos, entity) in self.world.block_entities.iter_mut() {
            if let BlockEntity::Furnace(f) = entity
                && let Some(lit) = f.tick(&self.res.recipes, &self.res.items, &self.res.providers)
            {
                lit_changes.push((*pos, lit));
            }
        }
        for ((x, y, z), lit) in lit_changes {
            let state = self.world.block(x, y, z);
            if let Some(mut s) = self.res.assets.registry.state(state).cloned()
                && s.properties.contains_key("lit")
            {
                s.properties.insert("lit".into(), lit.to_string());
                if let Ok(id) = self.res.assets.registry.resolve(&s) {
                    self.world.set(x, y, z, id)?;
                }
            }
        }
        Ok(())
    }
    /// A copy of a chunk carrying its block entities, for saving.
    fn with_entities(&self, chunk: &Arc<horizon_client::chunk::Chunk>) -> Arc<horizon_client::chunk::Chunk> {
        let mut c = (**chunk).clone();
        c.block_entities = self
            .world
            .block_entities
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, e)| (*p, e.clone()))
            .collect();
        Arc::new(c)
    }
    fn tick_mobs(&mut self) -> Result<()> {
        self.attack_ticks = self.attack_ticks.saturating_add(1);
        // Generation-time creature packs for newly generated chunks.
        let fresh: Vec<(i32, i32)> = self.world.chunks.iter().filter(|(_, c)| c.fresh && c.lit).map(|(k, _)| *k).collect();
        for key in fresh {
            if let Some(c) = self.world.chunks.get_mut(&key) {
                Arc::make_mut(c).fresh = false;
            }
            self.spawn_chunk_creatures(key.0, key.1);
        }
        if self.ticks % 2 == 0 {
            self.spawn_monsters();
        }
        let player = self.player.as_ref().map(|p| ([p.position.x, p.position.y + 0.5, p.position.z], !p.health.dead_or_dying()));
        let daylight = self.sky_darken() < 4 && self.world.weather.levels(1.0).0 < 0.2;
        let mut events = Vec::new();
        let mut mobs = std::mem::take(&mut self.mobs);
        {
            let w = MobWorld { world: &self.world, facts: &self.res.facts, collision: self.collision, player, daylight };
            let random = &mut self.world.random.clone();
            let loaded = |m: &Mob| w.world.loaded(m.position.x.floor() as i32, m.position.z.floor() as i32);
            let mut dead = Vec::new();
            mobs.retain_mut(|m| {
                if !loaded(m) {
                    return !m.hostile() || m.persistent;
                }
                let keep = m.tick(&w, random, &mut events);
                if !keep && !m.alive() {
                    dead.push((m.kind.id.clone(), [m.position.x, m.position.y, m.position.z], m.id));
                }
                keep
            });
            self.world.random = random.clone();
            for (kind, pos, id) in dead {
                let by_player = self.last_hurt_by_player.remove(&id);
                let held = self.held().cloned();
                let drops = self.res.items.entity_drops(&kind, by_player, held.as_ref(), &mut self.world.random);
                for d in drops {
                    self.spawn_item(d, [pos[0], pos[1] + 0.5, pos[2]], [0.0, 0.2, 0.0], 10);
                }
                if by_player {
                    // Animal: 1 + random(3); Monster: 5.
                    let hostile = self.res.mob_types.get(&kind).is_some_and(|t| t.category == "monster");
                    let xp = if hostile { 5 } else { 1 + self.world.random.next_int_bound(3).unwrap_or(0) as u32 };
                    self.spawn_orbs([pos[0], pos[1] + 0.5, pos[2]], xp);
                }
            }
        }
        self.mobs = mobs;
        for e in events {
            match e {
                MobEvent::Sound(name, at, volume, pitch) => self.audio.play(&format!("minecraft:{name}"), Some(at), volume, pitch),
                MobEvent::AttackPlayer(amount, from) => {
                    if self.creative {
                        continue;
                    }
                    if let Some(player) = self.player.as_mut()
                        && let Some(id) = self.motion.health.damage.id("minecraft:mob_attack")
                        && player.hurt(&self.motion.health, self.world.health_rules, id, amount).unwrap_or(false)
                    {
                        let (dx, dz) = (player.position.x - from[0], player.position.z - from[2]);
                        let l = (dx * dx + dz * dz).sqrt().max(1e-4);
                        player.velocity = Vec3::new(player.velocity.x / 2.0 + dx / l * 0.4, (player.velocity.y / 2.0 + 0.4).min(0.4), player.velocity.z / 2.0 + dz / l * 0.4);
                        let at = [player.position.x, player.position.y, player.position.z];
                        self.audio.play("minecraft:entity.player.hurt", Some(at), 1.0, 1.0);
                        self.hurt_armor(amount);
                    }
                }
                MobEvent::Shoot(from, target) => {
                    // Skeleton.performRangedAttack: aim above by 20% of distance, inaccuracy 14 - 4 * difficulty(2).
                    let (dx, dz) = (target[0] - from[0], target[2] - from[2]);
                    let horizontal = (dx * dx + dz * dz).sqrt();
                    let dy = target[1] + 0.5 - from[1] + horizontal * 0.2;
                    let len = (dx * dx + dy * dy + dz * dz).sqrt().max(1e-4);
                    let spread = 0.0075 * 6.0;
                    let r = &mut self.world.random;
                    let mut gauss = || f64::from(r.next_float() + r.next_float() - 1.0) * spread;
                    let v = [(dx / len + gauss()) * 1.6, (dy / len + gauss()) * 1.6, (dz / len + gauss()) * 1.6];
                    let p = Vec3::new(from[0], from[1] - 0.1, from[2]);
                    self.arrows.push(Arrow { from_player: false, critical: false, position: p, previous: p, velocity: Vec3::new(v[0], v[1], v[2]), stuck: false, age: 0, base_damage: 2.0 });
                    self.audio.play("minecraft:entity.skeleton.shoot", Some(from), 1.0, 1.0 / (self.world.random.next_float() * 0.4 + 0.8));
                }
                MobEvent::Explode(at, radius) => self.explode(at, radius)?,
            }
        }
        // Arrows.
        let bounds = self.player.as_ref().map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            horizon_core::geometry::Aabb::new(
                Vec3::new(p.position.x - w, p.position.y, p.position.z - w),
                Vec3::new(p.position.x + w, p.position.y + f64::from(p.dimensions.height), p.position.z + w),
            )
        });
        let mut arrows = std::mem::take(&mut self.arrows);
        let mut hits = Vec::new();
        let mobs = &mut self.mobs;
        let mut random_state = self.world.random.clone();
        let random = &mut random_state;
        let mut mob_hits = Vec::new();
        arrows.retain_mut(|a| {
            if a.from_player && !a.stuck {
                let (keep, _) = a.tick(&self.world, &self.res.facts, None);
                for m in mobs.iter_mut().filter(|m| m.alive()) {
                    let b = m.bounds();
                    let (lo, hi) = (b.min_position(), b.max_position());
                    let p = a.position;
                    if p.x > lo.x - 0.3 && p.x < hi.x + 0.3 && p.y > lo.y - 0.3 && p.y < hi.y + 0.3 && p.z > lo.z - 0.3 && p.z < hi.z + 0.3 {
                        // AbstractArrow.onHitEntity: ceil(speed * base), crits add random(damage / 2 + 2).
                        let speed = (a.velocity.x.powi(2) + a.velocity.y.powi(2) + a.velocity.z.powi(2)).sqrt();
                        let mut damage = (speed * a.base_damage).ceil().max(0.0) as i32;
                        if a.critical {
                            damage += random.next_int_bound(damage / 2 + 2).unwrap_or(0);
                        }
                        if m.hurt(damage as f32, Some([a.previous.x, a.previous.y, a.previous.z]), random) {
                            mob_hits.push((m.id, [p.x, p.y, p.z]));
                        }
                        return false;
                    }
                }
                return keep;
            }
            let (keep, hit) = a.tick(&self.world, &self.res.facts, if a.from_player { None } else { bounds });
            if let ArrowHit::Player(d) = hit {
                hits.push(d);
            }
            keep
        });
        self.arrows = arrows;
        self.world.random = random_state;
        for (id, at) in mob_hits {
            self.last_hurt_by_player.insert(id);
            self.audio.play("minecraft:entity.arrow.hit", Some(at), 1.0, 1.2 / (self.world.random.next_float() * 0.2 + 0.9));
        }
        for d in hits {
            if self.creative {
                continue;
            }
            if let Some(player) = self.player.as_mut()
                && let Some(id) = self.motion.health.damage.id("minecraft:arrow")
            {
                let _ = player.hurt(&self.motion.health, self.world.health_rules, id, d);
                let at = [player.position.x, player.position.y, player.position.z];
                self.audio.play("minecraft:entity.arrow.hit_player", Some(at), 1.0, 1.0);
                self.hurt_armor(d);
            }
        }
        Ok(())
    }
    /// SkyRenderer.renderSunMoonAndStars: bodies on a frame rotated Y -90 then X by angle.
    fn celestial_geometry(&self, sky: &horizon_client::environment::Sky) -> Vec<Vertex> {
        let mut out = Vec::new();
        let atlas = &self.res.atlas;
        let frame = |angle: f32| glam::Mat4::from_rotation_y((-90.0f32).to_radians()) * glam::Mat4::from_rotation_x(angle.to_radians());
        let mut body = |name: &str, angle: f32, size: f32, color: u32| {
            let Some(r) = atlas.sprites.get(name).copied() else { return };
            let m = frame(angle);
            let corners = [(-size, -size), (size, -size), (size, size), (-size, size)];
            let uv = [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]];
            for ((x, z), uv) in corners.iter().zip(uv) {
                let p = m.transform_point3(glam::Vec3::new(*x, 100.0, *z));
                out.push(Vertex { pos: p.to_array(), uv, layer: 2, color: color, light: 0 });
            }
        };
        body("environment/celestial/sun", sky.sun_angle, 30.0, 0xFFFF_FFFF);
        let phases = ["full_moon", "waning_gibbous", "third_quarter", "waning_crescent", "new_moon", "waxing_crescent", "first_quarter", "waxing_gibbous"];
        let phase = phases[(self.world.day_time / 24000).rem_euclid(8) as usize];
        body(&format!("environment/celestial/moon/{phase}"), sky.moon_angle, 20.0, 0xFFFF_FFFF);
        if sky.star_brightness > 0.0 {
            let b = (sky.star_brightness.clamp(0.0, 1.0) * 255.0) as u32;
            let color = b << 24 | b << 16 | b << 8 | b;
            let white = atlas.sprites.get("horizon/white").copied();
            let m = frame(sky.star_angle);
            if let Some(r) = white {
                let (cu, cv) = ((r.u0 + r.u1) / 2.0, (r.v0 + r.v1) / 2.0);
                // SkyRenderer.buildStars with its fixed seed.
                let mut random = horizon_core::random::LegacyRandom::new(10842);
                for _ in 0..1500 {
                    let x = random.next_float() * 2.0 - 1.0;
                    let y = random.next_float() * 2.0 - 1.0;
                    let z = random.next_float() * 2.0 - 1.0;
                    let size = 0.15 + random.next_float() * 0.1;
                    let l = x * x + y * y + z * z;
                    if l <= 0.010_000_001 || l >= 1.0 {
                        continue;
                    }
                    let center = glam::Vec3::new(x, y, z).normalize() * 100.0;
                    let z_rot = (random.next_double() * std::f64::consts::TAU) as f32;
                    let forward = -center.normalize();
                    let right = glam::Vec3::Y.cross(forward).normalize_or(glam::Vec3::X);
                    let up = forward.cross(right);
                    let (s, c) = (-z_rot).sin_cos();
                    let (r2, u2) = (right * c + up * s, up * c - right * s);
                    for (a, b) in [(size, -size), (size, size), (-size, size), (-size, -size)] {
                        let p = m.transform_point3(center + r2 * a + u2 * b);
                        out.push(Vertex { pos: p.to_array(), uv: [cu, cv], layer: 2, color, light: 0 });
                    }
                }
            }
        }
        out
    }
    /// CloudRenderer (fancy): 12x4x12 cells from clouds.png drifting east.
    fn cloud_geometry(&self, sky: &horizon_client::environment::Sky, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let (w, h, cells) = &self.res.clouds;
        let (w, h) = (i64::from(*w), i64::from(*h));
        let Some(white) = self.res.atlas.sprites.get("horizon/white").copied() else { return };
        let uv = [(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0];
        let cloud_height = 192.33;
        let offset = (self.world.game_time.rem_euclid(w * 400)) as f64 + alpha;
        let mut cloud_x = camera.x + offset * 0.030_000_001;
        let mut cloud_z = camera.z + 3.96;
        let (tw, th) = (w as f64 * 12.0, h as f64 * 12.0);
        cloud_x -= (cloud_x / tw).floor() * tw;
        cloud_z -= (cloud_z / th).floor() * th;
        let cell_x = (cloud_x / 12.0).floor() as i64;
        let cell_z = (cloud_z / 12.0).floor() as i64;
        let x_in = (cloud_x - cell_x as f64 * 12.0) as f32;
        let z_in = (cloud_z - cell_z as f64 * 12.0) as f32;
        let bottom = (cloud_height - camera.y) as f32;
        let filled = |cx: i64, cz: i64| cells[(cz.rem_euclid(h) * w + cx.rem_euclid(w)) as usize];
        let radius = (self.streaming.radius * 16 / 12 + 1) as i64;
        let c = sky.cloud_color;
        let color = |shade: f32| {
            let ch = |v: f32| ((v * shade).clamp(0.0, 1.0) * 255.0) as u32;
            ch(c[3].max(0.0).min(1.0)) << 24 | ch(c[2]) << 16 | ch(c[1]) << 8 | ch(c[0])
        };
        let light = 240 | 240 << 8;
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                if dx * dx + dz * dz > radius * radius {
                    continue;
                }
                let (cx, cz) = (cell_x + dx, cell_z + dz);
                if !filled(cx, cz) {
                    continue;
                }
                let x0 = dx as f32 * 12.0 - x_in;
                let z0 = dz as f32 * 12.0 - z_in;
                let (x1, z1, y0, y1) = (x0 + 12.0, z0 + 12.0, bottom, bottom + 4.0);
                let mut quad = |p: [[f32; 3]; 4], shade: f32| {
                    for v in p {
                        out.push(Vertex { pos: v, uv, layer: 2, color: color(shade), light });
                    }
                };
                quad([[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]], 1.0);
                quad([[x1, y0, z0], [x1, y0, z1], [x0, y0, z1], [x0, y0, z0]], 0.7);
                if !filled(cx, cz - 1) {
                    quad([[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]], 0.8);
                }
                if !filled(cx, cz + 1) {
                    quad([[x1, y0, z1], [x1, y1, z1], [x0, y1, z1], [x0, y0, z1]], 0.8);
                }
                if !filled(cx - 1, cz) {
                    quad([[x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [x0, y0, z0]], 0.9);
                }
                if !filled(cx + 1, cz) {
                    quad([[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]], 0.9);
                }
            }
        }
    }
    /// Load (or create) a world folder under saves/ and start streaming it.
    fn open_world(&mut self, folder: &str, name: Option<&str>, seed: Option<i64>, creative: bool) -> Result<()> {
        let save_root = std::env::current_dir()?.join("saves").join(folder);
        let level = Level::read(&save_root).unwrap_or_else(|| Level {
            name: name.unwrap_or(folder).to_owned(),
            // WorldOptions.randomSeed when no seed was given.
            seed: seed.unwrap_or_else(|| {
                let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64);
                (t ^ (t >> 29)).wrapping_mul(0x9E37_79B9_7F4A_7C15) as i64
            }),
            creative,
            day_time: 0,
            health: 20.0,
            food: 20,
            saturation: 5.0,
            ..Level::default()
        });
        let mut level = level;
        if let Some(d) = &self.options.dimension {
            level.dimension = d.clone();
            level.position = None;
        }
        let dim = Dim::from_id(&level.dimension);
        let (mut streaming, save) = self.make_streaming(&save_root, dim, level.seed)?;
        streaming.radius = self.settings.render_distance;
        let mut world = World::new(self.res.facts.air, level.seed);
        world.game_time = level.time;
        world.day_time = self.options.time.unwrap_or(level.day_time);
        world.weather = level.weather;
        if let Some(w) = self.options.weather.as_deref() {
            // /weather: clear, rain or thunder for a long duration.
            let (rain, thunder) = (w != "clear", w == "thunder");
            world.weather = horizon_client::world::Weather {
                clear_time: if w == "clear" { 6000 } else { 0 },
                rain_time: 6000,
                thunder_time: 6000,
                raining: rain,
                thundering: thunder,
                rain_level: if rain { (1.0, 1.0) } else { (0.0, 0.0) },
                thunder_level: if thunder { (1.0, 1.0) } else { (0.0, 0.0) },
            };
        }
        self.creative = level.creative;
        let mut inventory = vec![None; 36];
        self.armor = [None, None, None, None];
        for (slot, id, count, damage) in &level.inventory {
            let stack = ItemStack { item: Arc::from(id.as_str()), count: *count, damage: *damage };
            if (100..104).contains(slot) {
                self.armor[usize::from(103 - slot)] = Some(stack);
            } else if let Some(cell) = inventory.get_mut(usize::from(*slot)) {
                *cell = Some(stack);
            }
        }
        if self.creative && level.inventory.is_empty() && level.position.is_none() {
            for (i, name) in ["minecraft:grass_block", "minecraft:stone", "minecraft:oak_planks", "minecraft:glass", "minecraft:torch", "minecraft:oak_log", "minecraft:chest", "minecraft:furnace", "minecraft:diamond_pickaxe"].iter().enumerate() {
                inventory[i] = Some(ItemStack { item: Arc::from(*name), count: 1, damage: 0 });
            }
        }
        let (yaw, pitch) = self.options.look.unwrap_or((level.rotation[0], level.rotation[1]));
        self.yaw = yaw;
        self.pitch = pitch;
        self.streaming = streaming;
        self.world = world;
        self.inventory = inventory;
        self.save = save;
        self.level = level;
        self.dim = dim;
        self.world_root = save_root.clone();
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.falling.clear();
        self.travelling = None;
        self.player = None;
        self.spawn = None;
        self.spawn_search = 0;
        self.mobs.clear();
        self.arrows.clear();
        self.dropped.clear();
        self.particles.clear();
        self.orbs.clear();
        self.experience = Experience { level: self.level.xp_level, progress: self.level.xp_progress, total: self.level.xp_total };
        self.mining = None;
        self.container = None;
        self.carried = None;
        self.using = None;
        self.screen = Screen::Playing;
        self.title = None;
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.apply_settings();
        self.streaming.sky_light = dim == Dim::Overworld;
        self.streaming.nether_shading = dim == Dim::Nether;
        // New worlds get their level.dat right away.
        if !save_root.join("level.dat").exists() {
            self.level.write(&save_root)?;
        }
        self.load_world_state();
        Ok(())
    }
    /// Streaming and region storage for one dimension of a world.
    fn make_streaming(&self, world_root: &std::path::Path, dim: Dim, seed: i64) -> Result<(Streaming, Arc<SaveDir>)> {
        let region_root = match dim.folder() {
            Some(f) => world_root.join(f),
            None => world_root.to_owned(),
        };
        let save = Arc::new(SaveDir::new(region_root));
        let res = &*self.res;
        let mut streaming = if self.options.test_terrain && dim == Dim::Overworld {
            let inner = Arc::new(TestTerrain::new(res.assets.registry, &res.biomes)?);
            Streaming::new(Arc::new(Persistent { inner, save: save.clone(), registry: res.assets.clone(), biomes: res.biomes.clone() }), res.context.clone(), res.threads)
        } else {
            let inner = Arc::new(VanillaTerrain::for_dimension(&self.options.resources.join("worldgen"), res.assets.registry, &res.biomes, seed, dim.id()).map_err(|e| {
                format!("{e}\nPrepare the worldgen bundle with: python3 tools/prepare_native_client.py")
            })?);
            Streaming::new(Arc::new(Persistent { inner, save: save.clone(), registry: res.assets.clone(), biomes: res.biomes.clone() }), res.context.clone(), res.threads)
        };
        streaming.radius = self.settings.render_distance;
        streaming.sky_light = dim == Dim::Overworld;
        streaming.nether_shading = dim == Dim::Nether;
        Ok((streaming, save))
    }
    /// Change dimension through a portal (Entity.changeDimension with
    /// PortalForcer placement once the destination has loaded).
    fn enter_dimension(&mut self, target: Dim) -> Result<()> {
        let Some(player) = self.player.take() else { return Ok(()) };
        // Save the departing dimension's changed chunks.
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks: Vec<_> = keys.iter().filter_map(|k| self.world.chunks.get(k)).map(|c| self.with_entities(c)).collect();
        self.save.save(&chunks, &self.res.assets.registry, &self.res.biomes)?;
        // Coordinates scale by 8 between the Overworld and the Nether.
        let scale = match (self.dim, target) {
            (Dim::Overworld, Dim::Nether) => 1.0 / 8.0,
            (Dim::Nether, Dim::Overworld) => 8.0,
            _ => 1.0,
        };
        let destination = Vec3::new(player.position.x * scale, player.position.y, player.position.z * scale);
        let (streaming, save) = self.make_streaming(&self.world_root.clone(), target, self.level.seed)?;
        let mut world = World::new(self.res.facts.air, self.level.seed);
        world.game_time = self.world.game_time;
        world.day_time = self.world.day_time;
        world.weather = self.world.weather;
        self.streaming = streaming;
        self.world = world;
        self.save = save;
        self.dim = target;
        self.level.dimension = target.id().to_owned();
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.falling.clear();
        self.mobs.clear();
        self.arrows.clear();
        self.dropped.clear();
        self.particles.clear();
        self.orbs.clear();
        self.mining = None;
        self.travelling = Some((player, destination));
        self.portal_cooldown = 300;
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.apply_settings();
        Ok(())
    }
    /// PortalShape: an obsidian frame around 2..=21 by 3..=21 empty blocks.
    fn portal_shape(&self, start: [i32; 3], x_axis: bool) -> Option<([i32; 3], i32, i32)> {
        let name = |p: [i32; 3]| self.res.facts.get(self.world.block(p[0], p[1], p[2])).name.clone();
        let empty = |p: [i32; 3]| {
            let n = name(p);
            matches!(&*n, "minecraft:air" | "minecraft:cave_air" | "minecraft:fire" | "minecraft:soul_fire" | "minecraft:nether_portal")
        };
        let frame = |p: [i32; 3]| &*name(p) == "minecraft:obsidian";
        let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
        let mut bottom = start;
        let mut n = 0;
        while empty([bottom[0], bottom[1] - 1, bottom[2]]) && n < 21 {
            bottom[1] -= 1;
            n += 1;
        }
        if !frame([bottom[0], bottom[1] - 1, bottom[2]]) {
            return None;
        }
        let mut left = bottom;
        n = 0;
        while empty([left[0] - d[0], left[1], left[2] - d[2]]) && n < 21 {
            left = [left[0] - d[0], left[1], left[2] - d[2]];
            n += 1;
        }
        if !frame([left[0] - d[0], left[1], left[2] - d[2]]) {
            return None;
        }
        let mut width = 0;
        while width <= 21 {
            let p = [left[0] + d[0] * width, left[1], left[2] + d[2] * width];
            if !empty(p) || !frame([p[0], p[1] - 1, p[2]]) {
                break;
            }
            width += 1;
        }
        if !(2..=21).contains(&width) || !frame([left[0] + d[0] * width, left[1], left[2] + d[2] * width]) {
            return None;
        }
        let mut height = 0;
        'rows: while height <= 21 {
            let y = left[1] + height;
            if !frame([left[0] - d[0], y, left[2] - d[2]]) || !frame([left[0] + d[0] * width, y, left[2] + d[2] * width]) {
                break;
            }
            for i in 0..width {
                if !empty([left[0] + d[0] * i, y, left[2] + d[2] * i]) {
                    break 'rows;
                }
            }
            height += 1;
        }
        let top_ok = (0..width).all(|i| frame([left[0] + d[0] * i, left[1] + height, left[2] + d[2] * i]));
        ((3..=21).contains(&height) && top_ok).then_some((left, width, height))
    }
    /// BaseFireBlock.onPlace: lighting fire inside a frame creates a portal.
    fn try_light_portal(&mut self, at: [i32; 3]) -> Result<bool> {
        for x_axis in [true, false] {
            if let Some((origin, width, height)) = self.portal_shape(at, x_axis) {
                let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
                let registry = &self.res.assets.registry;
                let Some(default) = registry.default_state("minecraft:nether_portal") else { return Ok(false) };
                let mut portal = registry.state(default).cloned().ok_or("portal state")?;
                portal.properties.insert("axis".into(), if x_axis { "x" } else { "z" }.into());
                let id = registry.resolve(&portal).unwrap_or(default);
                for h in 0..height {
                    for w in 0..width {
                        self.world.set(origin[0] + d[0] * w, origin[1] + h, origin[2] + d[2] * w, id)?;
                    }
                }
                return Ok(true);
            }
        }
        Ok(false)
    }
    /// PortalForcer: reuse a nearby portal, else build a 2x3 obsidian portal.
    fn place_after_travel(&mut self, mut player: PlayerMotion, target: Vec3) -> Result<()> {
        let (tx, tz) = (target.x.floor() as i32, target.z.floor() as i32);
        let radius = if self.dim == Dim::Nether { 16 } else { 32 };
        let (min_y, max_y) = if self.dim == Dim::Nether { (1, 126) } else { (MIN_Y + 1, MIN_Y + HEIGHT - 3) };
        let portal_name = "minecraft:nether_portal";
        let mut best: Option<(i64, [i32; 3])> = None;
        for z in tz - radius..=tz + radius {
            for x in tx - radius..=tx + radius {
                if !self.world.loaded(x, z) {
                    continue;
                }
                for y in min_y..max_y {
                    if &*self.res.facts.get(self.world.block(x, y, z)).name == portal_name
                        && &*self.res.facts.get(self.world.block(x, y - 1, z)).name != portal_name
                    {
                        let d = i64::from(x - tx).pow(2) + i64::from(z - tz).pow(2) + i64::from(y - target.y as i32).pow(2);
                        if best.is_none_or(|b| d < b.0) {
                            best = Some((d, [x, y, z]));
                        }
                    }
                }
            }
        }
        let spot = match best {
            Some((_, p)) => p,
            None => {
                // Build: obsidian frame 4 wide, 5 tall, with a floor, carved into the terrain.
                let y = (target.y.floor() as i32).clamp(min_y + 1, max_y - 5);
                let set = |g: &mut Self, p: [i32; 3], name: &str| -> Result<()> {
                    if let Some(id) = g.res.assets.registry.default_state(name) {
                        g.world.set(p[0], p[1], p[2], id)?;
                    }
                    Ok(())
                };
                for dz in -1..=1 {
                    for dx in -1..=2 {
                        set(self, [tx + dx, y - 1, tz + dz], "minecraft:obsidian")?;
                        for dy in 0..4 {
                            set(self, [tx + dx, y + dy, tz + dz], "minecraft:air")?;
                        }
                    }
                }
                for w in -1..=2 {
                    for h in -1..=3 {
                        if w == -1 || w == 2 || h == -1 || h == 3 {
                            set(self, [tx + w, y + h, tz], "minecraft:obsidian")?;
                        }
                    }
                }
                self.try_light_portal([tx, y, tz])?;
                [tx, y, tz]
            }
        };
        player.position = Vec3::new(f64::from(spot[0]) + 0.5, f64::from(spot[1]), f64::from(spot[2]) + 0.5);
        player.velocity = Vec3::ZERO;
        self.previous_position = player.position;
        self.player = Some(player);
        self.apply_equipment();
        let at = [f64::from(spot[0]), f64::from(spot[1]), f64::from(spot[2])];
        self.audio.play("minecraft:block.portal.travel", Some(at), 0.25, 1.0);
        Ok(())
    }
    fn tick_portal(&mut self) -> Result<()> {
        let Some(p) = self.player.as_ref() else { return Ok(()) };
        let w = f64::from(p.dimensions.width) / 2.0;
        let (lo, hi) = ([p.position.x - w, p.position.y, p.position.z - w], [p.position.x + w, p.position.y + f64::from(p.dimensions.height), p.position.z + w]);
        let mut inside = false;
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    if &*self.res.facts.get(self.world.block(x, y, z)).name == "minecraft:nether_portal" {
                        inside = true;
                    }
                }
            }
        }
        if !inside {
            self.portal_ticks = 0;
            self.portal_cooldown = self.portal_cooldown.saturating_sub(1);
            return Ok(());
        }
        if self.portal_cooldown > 0 {
            return Ok(());
        }
        if self.portal_ticks == 0 {
            let at = self.eye_position();
            self.audio.play("minecraft:block.portal.trigger", at, 0.25, self.world.random.next_float() * 0.4 + 0.8);
        }
        self.portal_ticks += 1;
        // Player.getPortalWaitTime: 80 ticks, or 1 when invulnerable (creative).
        let wait = if self.creative { 1 } else { 80 };
        if self.portal_ticks >= wait {
            self.portal_ticks = 0;
            let target = if self.dim == Dim::Overworld { Dim::Nether } else { Dim::Overworld };
            self.enter_dimension(target)?;
        }
        Ok(())
    }
    fn close_world(&mut self) -> Result<()> {
        self.save_game()?;
        self.streaming.paused = true;
        self.world = World::new(self.res.facts.air, 0);
        self.player = None;
        self.mobs.clear();
        self.arrows.clear();
        self.dropped.clear();
        self.particles.clear();
        self.screen = Screen::Playing;
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.title = Some(Title::new(std::env::current_dir()?.join("saves")));
        self.capture(false);
        Ok(())
    }
    /// The title panorama as six camera-centred faces (panorama_0..5:
    /// front, right, back, left, up, down).
    fn panorama_geometry(&self) -> Vec<Vertex> {
        let mut out = Vec::new();
        let d = 100.0;
        let faces: [(&str, [glam::Vec3; 4]); 6] = [
            ("0", [glam::Vec3::new(-d, d, -d), glam::Vec3::new(d, d, -d), glam::Vec3::new(d, -d, -d), glam::Vec3::new(-d, -d, -d)]),
            ("1", [glam::Vec3::new(d, d, -d), glam::Vec3::new(d, d, d), glam::Vec3::new(d, -d, d), glam::Vec3::new(d, -d, -d)]),
            ("2", [glam::Vec3::new(d, d, d), glam::Vec3::new(-d, d, d), glam::Vec3::new(-d, -d, d), glam::Vec3::new(d, -d, d)]),
            ("3", [glam::Vec3::new(-d, d, d), glam::Vec3::new(-d, d, -d), glam::Vec3::new(-d, -d, -d), glam::Vec3::new(-d, -d, d)]),
            ("4", [glam::Vec3::new(-d, d, d), glam::Vec3::new(d, d, d), glam::Vec3::new(d, d, -d), glam::Vec3::new(-d, d, -d)]),
            ("5", [glam::Vec3::new(-d, -d, -d), glam::Vec3::new(d, -d, -d), glam::Vec3::new(d, -d, d), glam::Vec3::new(-d, -d, d)]),
        ];
        for (index, corners) in faces {
            let Some(r) = self.res.atlas.sprites.get(&format!("gui/title/background/panorama_{index}")).copied() else { continue };
            let uv = [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]];
            for (p, uv) in corners.iter().zip(uv) {
                out.push(Vertex { pos: p.to_array(), uv, layer: 2, color: 0xFFFF_FFFF, light: 0 });
            }
        }
        out
    }
    /// Equipment attribute modifiers (LivingEntity.collectEquipmentChanges).
    fn apply_equipment(&mut self) {
        let Some(player) = self.player.as_mut() else { return };
        for (attribute, id) in std::mem::take(&mut self.applied_modifiers) {
            let _ = player.set_modifier(&attribute, &id, 0.0, "add_value", false);
        }
        let slots = ["head", "chest", "legs", "feet"];
        let mut equipped: Vec<(Option<&ItemStack>, &str)> = self.armor.iter().zip(slots).map(|(s, n)| (s.as_ref(), n)).collect();
        equipped.push((self.inventory[self.selected].as_ref(), "mainhand"));
        for (stack, slot) in equipped {
            let Some(info) = stack.and_then(|s| self.res.items.get(&s.item)) else { continue };
            for (attribute, id, amount, op, group) in &info.modifiers {
                let matches = group == slot
                    || group == "any"
                    || (group == "armor" && slot != "mainhand")
                    || (group == "hand" && slot == "mainhand");
                // Main-hand attack modifiers are read by the attack code directly.
                if !matches || slot == "mainhand" {
                    continue;
                }
                if player.set_modifier(attribute, id, *amount, op, true).is_ok() {
                    self.applied_modifiers.push((attribute.clone(), id.clone()));
                }
            }
        }
    }
    /// Inventory.hurtArmor: each piece loses max(1, damage / 4) durability.
    fn hurt_armor(&mut self, damage: f32) {
        let amount = ((damage / 4.0) as u32).max(1);
        let mut broke = false;
        for slot in &mut self.armor {
            if let Some(stack) = slot.as_mut()
                && let Some(info) = self.res.items.get(&stack.item)
                && info.max_damage > 0
            {
                stack.damage += amount;
                if stack.damage >= info.max_damage {
                    *slot = None;
                    broke = true;
                }
            }
        }
        if broke {
            if let Some(eye) = self.eye_position() {
                self.audio.play("minecraft:entity.item.break", Some(eye), 0.8, 0.8 + self.world.random.next_float() * 0.4);
            }
            self.apply_equipment();
        }
    }
    /// Voxel traversal that also stops at fluid source blocks (bucket pick).
    fn pick_fluid(&self) -> Option<Hit> {
        let origin = self.eye_position()?;
        let dir = view_vector(self.yaw, self.pitch);
        let mut t = 0.0;
        let mut last = origin.map(|v| v.floor() as i32);
        while t <= self.reach() {
            let p = [origin[0] + dir[0] * t, origin[1] + dir[1] * t, origin[2] + dir[2] * t];
            let b = p.map(|v| v.floor() as i32);
            if b != last {
                let state = self.world.block(b[0], b[1], b[2]);
                let f = self.res.facts.get(state);
                let face = [last[0] - b[0], last[1] - b[1], last[2] - b[2]];
                if f.fluid != Fluid::None && f.fluid_level == 8 && !f.fluid_falling && !f.waterlogged && f.render_shape != horizon_client::blocks::RenderShape::Model {
                    return Some(Hit { pos: b, face, point: p });
                }
                if !self.res.facts.is_air(state) && f.fluid == Fluid::None {
                    return Some(Hit { pos: b, face, point: p });
                }
                last = b;
            }
            t += 0.05;
        }
        None
    }
    fn set_state_named(&mut self, pos: [i32; 3], name: &str, keep: &[&str]) -> Result<bool> {
        let registry = &self.res.assets.registry;
        let old = self.world.block(pos[0], pos[1], pos[2]);
        let Some(default) = registry.default_state(name) else { return Ok(false) };
        let mut state = registry.state(default).cloned().ok_or("state")?;
        if let Some(previous) = registry.state(old) {
            for k in keep {
                if let Some(v) = previous.properties.get(*k) {
                    state.properties.insert((*k).to_owned(), v.clone());
                }
            }
        }
        let id = registry.resolve(&state).unwrap_or(default);
        self.level_set(pos, id)?;
        Ok(true)
    }
    fn damage_held(&mut self, amount: u32) {
        if self.creative {
            return;
        }
        if let Some(stack) = self.inventory[self.selected].as_mut()
            && let Some(info) = self.res.items.get(&stack.item)
            && info.max_damage > 0
        {
            stack.damage += amount;
            if stack.damage >= info.max_damage {
                self.inventory[self.selected] = None;
                if let Some(eye) = self.eye_position() {
                    self.audio.play("minecraft:entity.item.break", Some(eye), 0.8, 0.8 + self.world.random.next_float() * 0.4);
                }
            }
        }
    }
    /// Item use on blocks for tools, buckets and fire starters; returns true if handled.
    fn use_tool(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = stack.item.trim_start_matches("minecraft:").to_owned();
        let at = |p: [i32; 3]| [f64::from(p[0]) + 0.5, f64::from(p[1]) + 0.5, f64::from(p[2]) + 0.5];
        if item == "bucket" {
            let Some(hit) = self.pick_fluid() else { return Ok(false) };
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let f = self.res.facts.get(state).clone();
            if f.fluid == Fluid::None || f.fluid_level != 8 || f.fluid_falling {
                return Ok(false);
            }
            let (filled, sound) = match f.fluid {
                Fluid::Water => ("minecraft:water_bucket", "minecraft:item.bucket.fill"),
                _ => ("minecraft:lava_bucket", "minecraft:item.bucket.fill_lava"),
            };
            self.world.set(hit.pos[0], hit.pos[1], hit.pos[2], self.world.air)?;
            self.audio.play(sound, Some(at(hit.pos)), 1.0, 1.0);
            if !self.creative {
                self.consume_held();
                let filled = ItemStack { item: Arc::from(filled), count: 1, damage: 0 };
                if self.inventory[self.selected].is_none() {
                    self.inventory[self.selected] = Some(filled);
                } else if let Some(rest) = self.give(filled)
                    && let Some(eye) = self.eye_position()
                {
                    self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
                }
            }
            return Ok(true);
        }
        let Some(hit) = self.pick() else { return Ok(false) };
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let name = self.res.facts.get(clicked).name.trim_start_matches("minecraft:").to_owned();
        let above_air = self.res.facts.is_air(self.world.block(hit.pos[0], hit.pos[1] + 1, hit.pos[2]));
        if item == "water_bucket" || item == "lava_bucket" {
            let target = if self.res.facts.get(clicked).replaceable { hit.pos } else { [hit.pos[0] + hit.face[0], hit.pos[1] + hit.face[1], hit.pos[2] + hit.face[2]] };
            let existing = self.world.block(target[0], target[1], target[2]);
            if !self.res.facts.get(existing).replaceable && !self.res.facts.is_air(existing) {
                return Ok(true);
            }
            let fluid = if item == "water_bucket" { "minecraft:water" } else { "minecraft:lava" };
            self.set_state_named(target, fluid, &[])?;
            let sound = if item == "water_bucket" { "minecraft:item.bucket.empty" } else { "minecraft:item.bucket.empty_lava" };
            self.audio.play(sound, Some(at(target)), 1.0, 1.0);
            if !self.creative {
                self.inventory[self.selected] = Some(ItemStack { item: Arc::from("minecraft:bucket"), count: 1, damage: 0 });
            }
            return Ok(true);
        }
        if item.ends_with("_hoe") && hit.face != [0, -1, 0] && above_air {
            // HoeItem.TILLABLES
            let result = match name.as_str() {
                "grass_block" | "dirt_path" | "dirt" => Some("minecraft:farmland"),
                "coarse_dirt" | "rooted_dirt" => Some("minecraft:dirt"),
                _ => None,
            };
            if let Some(result) = result {
                self.set_state_named(hit.pos, result, &[])?;
                if name == "rooted_dirt" {
                    self.pop_resource([hit.pos[0], hit.pos[1] - 1, hit.pos[2]], ItemStack { item: Arc::from("minecraft:hanging_roots"), count: 1, damage: 0 });
                }
                self.audio.play("minecraft:item.hoe.till", Some(at(hit.pos)), 1.0, 1.0);
                self.damage_held(1);
                return Ok(true);
            }
        }
        if item.ends_with("_shovel") && hit.face != [0, -1, 0] && above_air
            && matches!(name.as_str(), "grass_block" | "dirt" | "podzol" | "coarse_dirt" | "mycelium" | "rooted_dirt")
        {
            // ShovelItem.FLATTENABLES
            self.set_state_named(hit.pos, "minecraft:dirt_path", &[])?;
            self.audio.play("minecraft:item.shovel.flatten", Some(at(hit.pos)), 1.0, 1.0);
            self.damage_held(1);
            return Ok(true);
        }
        if item.ends_with("_axe") && !name.starts_with("stripped_") {
            // AxeItem.STRIPPABLES
            let strippable = name.ends_with("_log") || name.ends_with("_wood") || name.ends_with("_stem") || name.ends_with("_hyphae") || name == "bamboo_block";
            let stripped = format!("minecraft:stripped_{name}");
            if strippable && self.res.assets.registry.default_state(&stripped).is_some() {
                self.set_state_named(hit.pos, &stripped, &["axis"])?;
                self.audio.play("minecraft:item.axe.strip", Some(at(hit.pos)), 1.0, 1.0);
                self.damage_held(1);
                return Ok(true);
            }
        }
        if item == "flint_and_steel" || item == "fire_charge" {
            let target = [hit.pos[0] + hit.face[0], hit.pos[1] + hit.face[1], hit.pos[2] + hit.face[2]];
            if self.res.facts.is_air(self.world.block(target[0], target[1], target[2])) {
                if !self.try_light_portal(target)? {
                    self.set_state_named(target, "minecraft:fire", &[])?;
                }
                let sound = if item == "fire_charge" { "minecraft:item.firecharge.use" } else { "minecraft:item.flintandsteel.use" };
                self.audio.play(sound, Some(at(target)), 1.0, self.world.random.next_float() * 0.4 + 0.8);
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(true);
            }
        }
        // Equippable items swap into their armor slot (Equippable.swapWithEquipmentSlot).
        if let Some(slot) = self.res.items.get(&stack.item).and_then(|i| i.equip_slot.clone()) {
            let index = match slot.as_str() {
                "head" => 0,
                "chest" => 1,
                "legs" => 2,
                "feet" => 3,
                _ => return Ok(false),
            };
            let held = self.inventory[self.selected].take();
            self.inventory[self.selected] = self.armor[index].take();
            self.armor[index] = held;
            if let Some(eye) = self.eye_position() {
                self.audio.play("minecraft:item.armor.equip_generic", Some(eye), 1.0, 1.0);
            }
            self.apply_equipment();
            return Ok(true);
        }
        Ok(false)
    }
    /// ExperienceOrb.award: split into vanilla orb sizes.
    fn spawn_orbs(&mut self, at: [f64; 3], mut value: u32) {
        while value > 0 {
            let v = ExperienceOrb::split(value);
            value -= v;
            let r = &mut self.world.random;
            let p = Vec3::new(at[0], at[1], at[2]);
            self.orbs.push(ExperienceOrb {
                position: p,
                previous: p,
                velocity: Vec3::new((r.next_double() * 0.2 - 0.1) * 2.0, r.next_double() * 0.2 * 2.0, (r.next_double() * 0.2 - 0.1) * 2.0),
                value: v,
                age: 0,
                on_ground: false,
            });
        }
    }
    /// DropExperienceBlock / RedStoneOreBlock / SculkBlock experience ranges.
    fn block_experience(&mut self, name: &str) -> u32 {
        let (min, max) = match name.trim_start_matches("minecraft:") {
            "coal_ore" | "deepslate_coal_ore" => (0, 2),
            "nether_gold_ore" => (0, 1),
            "lapis_ore" | "deepslate_lapis_ore" | "nether_quartz_ore" => (2, 5),
            "diamond_ore" | "deepslate_diamond_ore" | "emerald_ore" | "deepslate_emerald_ore" => (3, 7),
            "redstone_ore" | "deepslate_redstone_ore" => (1, 5),
            "sculk" => (1, 1),
            "sculk_catalyst" | "sculk_sensor" | "calibrated_sculk_sensor" | "sculk_shrieker" => (5, 5),
            "spawner" => (15, 43),
            _ => (0, 0),
        };
        if max == 0 {
            return 0;
        }
        (min + self.world.random.next_int_bound(max - min + 1).unwrap_or(0)) as u32
    }
    fn tick_experience(&mut self) {
        if self.xp_delay > 0 {
            self.xp_delay -= 1;
        }
        let player = self.player.as_ref().filter(|p| !p.health.dead_or_dying());
        let target = player.map(|p| [p.position.x, p.position.y + f64::from(p.dimensions.eye_height) / 2.0, p.position.z]);
        let bounds = player.map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            ([p.position.x - w, p.position.y, p.position.z - w], [p.position.x + w, p.position.y + f64::from(p.dimensions.height), p.position.z + w])
        });
        let mut orbs = std::mem::take(&mut self.orbs);
        orbs.retain_mut(|o| o.tick(&self.world, &self.res.facts, self.collision, target));
        let mut gained = 0;
        if let Some((min, max)) = bounds {
            orbs.retain(|o| {
                let touching = o.position.x + 0.25 > min[0] && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1] && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2] && o.position.z - 0.25 < max[2];
                if touching && self.xp_delay == 0 {
                    // ExperienceOrb.playerTouch: one orb every 2 ticks.
                    self.xp_delay = 2;
                    gained += o.value;
                    return false;
                }
                true
            });
        }
        self.orbs = orbs;
        if gained > 0 {
            let at = self.eye_position();
            let pitch = (self.world.random.next_float() - self.world.random.next_float()) * 0.35 + 0.9;
            self.audio.play("minecraft:entity.experience_orb.pickup", at, 0.1, pitch);
            if self.experience.give(gained as i32) && self.experience.level % 5 == 0 && self.ticks > self.last_level_up + 100 {
                self.last_level_up = self.ticks;
                let volume = if self.experience.level > 30 { 1.0 } else { self.experience.level as f32 / 30.0 };
                self.audio.play("minecraft:entity.player.levelup", at, volume * 0.75, 1.0);
            }
        }
    }
    /// BowItem.releaseUsing: power from charge time, arrows at power * 3.
    fn release_bow(&mut self, ticks: u32) {
        let f = ticks as f32 / 20.0;
        let power = ((f * f + f * 2.0) / 3.0).min(1.0);
        if power < 0.1 {
            return;
        }
        let arrow_slot = self.inventory.iter().position(|s| s.as_ref().is_some_and(|s| &*s.item == "minecraft:arrow"));
        if arrow_slot.is_none() && !self.creative {
            return;
        }
        let Some(eye) = self.eye_position() else { return };
        let dir = view_vector(self.yaw, self.pitch);
        // Projectile.shoot: inaccuracy 1.0 as gaussian * 0.0075.
        let r = &mut self.world.random;
        let mut jitter = || f64::from(r.next_float() + r.next_float() - 1.0) * 0.0075;
        let speed = f64::from(power) * 3.0;
        let v = [(dir[0] + jitter()) * speed, (dir[1] + jitter()) * speed, (dir[2] + jitter()) * speed];
        let p = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        self.arrows.push(Arrow { from_player: true, critical: power >= 1.0, position: p, previous: p, velocity: Vec3::new(v[0], v[1], v[2]), stuck: false, age: 0, base_damage: 2.0 });
        let pitch = 1.0 / (self.world.random.next_float() * 0.4 + 1.2) + power * 0.5;
        self.audio.play("minecraft:entity.arrow.shoot", Some(eye), 1.0, pitch);
        if !self.creative
            && let Some(i) = arrow_slot
            && let Some(s) = self.inventory[i].as_mut()
        {
            s.count -= 1;
            if s.count == 0 {
                self.inventory[i] = None;
            }
        }
        self.damage_held(1);
    }
    /// LivingEntity.swing: restart the 6-tick arm swing.
    fn swing_arm(&mut self) {
        if self.swing.is_none_or(|t| t >= 3) {
            self.swing = Some(0);
        }
    }
    /// LivingEntity.updateSwingTime and ItemInHandRenderer.tick (equip height).
    fn tick_hand(&mut self) {
        self.swing_anim.0 = self.swing_anim.1;
        if let Some(t) = self.swing {
            let t = t + 1;
            if t >= 6 {
                self.swing = None;
                self.swing_anim.1 = 0.0;
            } else {
                self.swing = Some(t);
                self.swing_anim.1 = t as f32 / 6.0;
            }
        }
        self.hand_height.0 = self.hand_height.1;
        let held = self.inventory[self.selected].clone();
        let same = match (&held, &self.hand_item) {
            (Some(a), Some(b)) => a.item == b.item,
            (None, None) => true,
            _ => false,
        };
        let target = if same { 1.0 } else { 0.0 };
        self.hand_height.1 += (target - self.hand_height.1).clamp(-0.4, 0.4);
        if self.hand_height.1 < 0.1 {
            self.hand_item = held;
        }
    }
    fn display_transform(&mut self, item: &str) -> Option<([f32; 3], [f32; 3], [f32; 3])> {
        if let Some(d) = self.res.displays.get(item) {
            return *d;
        }
        let root = &self.options.resources;
        let name = item.trim_start_matches("minecraft:");
        let model_name = self.res.items.get(item).map_or(name.to_owned(), |i| i.model.trim_start_matches("minecraft:").to_owned());
        let mut found = None;
        let def: Option<serde_json::Value> = std::fs::read(root.join(format!("assets/minecraft/items/{model_name}.json"))).ok().and_then(|b| serde_json::from_slice(&b).ok());
        let mut current = def.and_then(|d| {
            fn first(v: &serde_json::Value) -> Option<String> {
                match v {
                    serde_json::Value::Object(o) => {
                        if o.get("type").and_then(|t| t.as_str()) == Some("minecraft:model") {
                            return o.get("model").and_then(|m| m.as_str()).map(str::to_owned);
                        }
                        o.values().find_map(first)
                    }
                    serde_json::Value::Array(a) => a.iter().find_map(first),
                    _ => None,
                }
            }
            first(&d["model"])
        });
        for _ in 0..16 {
            let Some(m) = current.take() else { break };
            let path = m.trim_start_matches("minecraft:").to_owned();
            let Some(model) = std::fs::read(root.join(format!("assets/minecraft/models/{path}.json"))).ok().and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok()) else { break };
            let d = &model["display"]["firstperson_righthand"];
            if d.is_object() {
                let v = |k: &str, default: f32| -> [f32; 3] {
                    let a = d[k].as_array();
                    std::array::from_fn(|i| a.and_then(|a| a.get(i)).and_then(|x| x.as_f64()).map_or(default, |x| x as f32))
                };
                found = Some((v("rotation", 0.0), v("translation", 0.0), v("scale", 1.0)));
                break;
            }
            current = model["parent"].as_str().map(str::to_owned);
        }
        self.res.displays.insert(item.to_owned(), found);
        found
    }
    /// FirstPersonHandsAndItemsRenderer: the right arm or held item in view
    /// space, converted to camera-relative world space.
    fn hand_geometry(&mut self, alpha: f64) -> Vec<Vertex> {
        let mut out = Vec::new();
        if self.player.as_ref().is_none_or(|p| p.health.dead_or_dying()) || self.screen == Screen::Dead {
            return out;
        }
        let a = alpha as f32;
        let attack = self.swing_anim.0 + (self.swing_anim.1 - self.swing_anim.0) * a;
        let attack = if self.swing.is_none() && self.swing_anim.1 == 0.0 { 0.0 } else { attack };
        let height = self.hand_height.0 + (self.hand_height.1 - self.hand_height.0) * a;
        let inverse_height = 1.0 - height;
        let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
        let forward = glam::Vec3::new(-yaw.sin() * pitch.cos(), -pitch.sin(), yaw.cos() * pitch.cos());
        let view = glam::Mat4::look_to_rh(glam::Vec3::ZERO, forward, glam::Vec3::Y);
        let to_world = view.inverse();
        let eye = self.eye_position().unwrap_or_default();
        let light = self.world.light(eye[0].floor() as i32, eye[1].floor() as i32, eye[2].floor() as i32);
        let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
        let deg = |d: f32| d.to_radians();
        let sqrt_attack = attack.sqrt();
        match self.hand_item.clone() {
            None => {
                let Some(arm) = self.res.player_arm.clone() else { return out };
                let Some(r) = self.res.atlas.sprites.get("entity/player/wide/steve").copied() else { return out };
                // renderPlayerArm (right arm)
                let x_swing = -0.3 * (sqrt_attack * std::f32::consts::PI).sin();
                let y_swing = 0.4 * (sqrt_attack * std::f32::consts::TAU).sin();
                let z_swing = -0.4 * (attack * std::f32::consts::PI).sin();
                let z_rot = (attack * attack * std::f32::consts::PI).sin();
                let y_rot = (sqrt_attack * std::f32::consts::PI).sin();
                let m = to_world
                    * glam::Mat4::from_translation(glam::Vec3::new(x_swing + 0.64, y_swing - 0.6 + inverse_height * -0.6, z_swing - 0.72))
                    * glam::Mat4::from_rotation_y(deg(45.0))
                    * glam::Mat4::from_rotation_y(deg(y_rot * 70.0))
                    * glam::Mat4::from_rotation_z(deg(z_rot * -20.0))
                    * glam::Mat4::from_translation(glam::Vec3::new(-1.0, 3.6, 3.5))
                    * glam::Mat4::from_rotation_z(deg(120.0))
                    * glam::Mat4::from_rotation_x(deg(200.0))
                    * glam::Mat4::from_rotation_y(deg(-135.0))
                    * glam::Mat4::from_translation(glam::Vec3::new(5.6, 0.0, 0.0));
                let pose = |p: &horizon_client::mobs::Part| {
                    let mut pose = p.pose;
                    pose[3] = 0.0;
                    pose[4] = 0.0;
                    pose[5] = 0.0;
                    pose
                };
                horizon_client::mobs::render_model(&arm, m, &pose, [r.u0, r.v0, r.u1, r.v1], packed, 0, &mut out);
            }
            Some(stack) => {
                let display = self.display_transform(&stack.item).unwrap_or(([0.0; 3], [0.0; 3], [1.0; 3]));
                let icon = self.icon(&stack.item);
                // applyItemArmTransform + swingArm (applyItemArmAttackTransform)
                let x = -0.4 * (sqrt_attack * std::f32::consts::PI).sin();
                let y = 0.2 * (sqrt_attack * std::f32::consts::TAU).sin();
                let z = -0.2 * (attack * std::f32::consts::PI).sin();
                let y_swing_rot = (attack * attack * std::f32::consts::PI).sin();
                let xz_swing_rot = (sqrt_attack * std::f32::consts::PI).sin();
                let (rot, trans, scale) = display;
                let m = to_world
                    * glam::Mat4::from_translation(glam::Vec3::new(0.56, -0.52 + inverse_height * -0.6, -0.72))
                    * glam::Mat4::from_translation(glam::Vec3::new(x, y, z))
                    * glam::Mat4::from_rotation_y(deg(45.0 + y_swing_rot * -20.0))
                    * glam::Mat4::from_rotation_z(deg(xz_swing_rot * -20.0))
                    * glam::Mat4::from_rotation_x(deg(xz_swing_rot * -80.0))
                    * glam::Mat4::from_rotation_y(deg(-45.0))
                    // ItemTransform.apply: translation is in sixteenths, rotation XYZ, then scale.
                    * glam::Mat4::from_translation(glam::Vec3::from(trans) / 16.0)
                    * glam::Mat4::from_quat(glam::Quat::from_euler(glam::EulerRot::XYZ, deg(rot[0]), deg(rot[1]), deg(rot[2])))
                    * glam::Mat4::from_scale(glam::Vec3::from(scale))
                    * glam::Mat4::from_translation(glam::Vec3::splat(-0.5));
                let atlas = &self.res.atlas;
                let quad = |corners: [glam::Vec3; 4], name: &str, shade: f32, tint: u32, out: &mut Vec<Vertex>| {
                    let Some(r) = atlas.sprites.get(name).copied() else { return };
                    let uv = [[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]];
                    let c = |sft: u32| (((tint >> sft) & 255) as f32 * shade) as u32;
                    let color = 0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16);
                    for (p, uv) in corners.iter().zip(uv) {
                        out.push(Vertex { pos: m.transform_point3(*p).to_array(), uv, layer: 0, color, light: packed });
                    }
                };
                let v = glam::Vec3::new;
                match icon {
                    Some(Icon::Cube { top, left, right, tint }) => {
                        quad([v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.), v(0., 1., 0.)], &top, 1.0, tint, &mut out);
                        quad([v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)], &left, 0.8, 0xFFFFFF, &mut out);
                        quad([v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)], &left, 0.8, 0xFFFFFF, &mut out);
                        quad([v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)], &right, 0.6, 0xFFFFFF, &mut out);
                        quad([v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)], &right, 0.6, 0xFFFFFF, &mut out);
                    }
                    Some(Icon::Flat(name)) => {
                        // Generated item models are a sprite on the z = 8/16 plane.
                        quad([v(0., 0., 0.5), v(1., 0., 0.5), v(1., 1., 0.5), v(0., 1., 0.5)], &name, 1.0, 0xFFFFFF, &mut out);
                    }
                    None => {}
                }
            }
        }
        out
    }
    fn apply_settings(&mut self) {
        self.streaming.radius = self.settings.render_distance;
        self.audio.master = self.settings.master_volume;
        horizon_client::hud::GUI_SCALE.store(self.settings.gui_scale, std::sync::atomic::Ordering::Relaxed);
        self.apply_ui_settings();
    }
    /// Whether precipitation at a column is snow (Biome.coldEnoughToSnow:
    /// base temperature lowered above y 80) or none (no precipitation).
    fn precipitation(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        let chunk = self.world.chunks.get(&(x >> 4, z >> 4))?;
        let b = self.res.biomes.get(chunk.biome(x, y, z));
        if !b.precipitation {
            return None;
        }
        let t = if y > 80 { b.temperature - (y - 80) as f32 * 0.05 / 40.0 } else { b.temperature };
        Some(t < 0.15)
    }
    /// The first motion-blocking block from the top (Heightmap MOTION_BLOCKING).
    fn rain_height(&self, x: i32, z: i32, from: i32) -> i32 {
        let mut y = from;
        while y > MIN_Y {
            let s = self.world.block(x, y, z);
            let f = self.res.facts.get(s);
            if f.full_collision || f.fluid != Fluid::None || f.name.contains("leaves") {
                return y + 1;
            }
            y -= 1;
        }
        MIN_Y
    }
    fn tick_weather_effects(&mut self) {
        if self.flash > 0 {
            self.flash -= 1;
        }
        let (rain, thunder) = self.world.weather.levels(1.0);
        let Some(eye) = self.eye_position() else { return };
        // WeatherEffectRenderer.tickRainParticles: occasional rain sounds.
        if rain > 0.0 && self.world.random.next_int_bound(3).unwrap_or(1) < (rain * 3.0) as i32 {
            let r = &mut self.world.random;
            let (dx, dz) = (r.next_int_bound(21).unwrap_or(10) - 10, r.next_int_bound(21).unwrap_or(10) - 10);
            let (x, z) = (eye[0].floor() as i32 + dx, eye[2].floor() as i32 + dz);
            let top = self.rain_height(x, z, eye[1] as i32 + 10);
            if self.precipitation(x, top, z) == Some(false) && (top - eye[1] as i32).abs() < 10 && self.world.random.next_int_bound(5).unwrap_or(1) == 0 {
                let above = f64::from(top) > eye[1] + 1.0;
                let (name, volume, pitch) = if above { ("minecraft:weather.rain.above", 0.1, 0.5) } else { ("minecraft:weather.rain", 0.2, 1.0) };
                self.audio.play(name, Some([f64::from(x) + 0.5, f64::from(top), f64::from(z) + 0.5]), volume, pitch);
            }
        }
        // ServerLevel.tickChunk lightning: 1 in 100000 per chunk per tick while thundering.
        if thunder > 0.0 && self.world.weather.thundering {
            let chunks = self.world.chunks.len().max(1) as i32;
            if self.world.random.next_int_bound((100_000 / chunks).max(1)).unwrap_or(1) == 0 {
                self.flash = 2;
                let r = &mut self.world.random;
                let (dx, dz) = (f64::from(r.next_int_bound(160).unwrap_or(0) - 80), f64::from(r.next_int_bound(160).unwrap_or(0) - 80));
                let at = [eye[0] + dx, eye[1], eye[2] + dz];
                let pitch = 0.8 + self.world.random.next_float() * 0.2;
                self.audio.play("minecraft:entity.lightning_bolt.thunder", Some(at), 10000.0, pitch);
                self.audio.play("minecraft:entity.lightning_bolt.impact", Some(at), 2.0, 0.5 + self.world.random.next_float() * 0.2);
            }
        }
    }
    /// WeatherEffectRenderer: rain/snow columns within 10 blocks.
    fn weather_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        let (rain, _) = self.world.weather.levels(alpha as f32);
        if rain <= 0.0 {
            return;
        }
        let atlas = &self.res.atlas;
        let (Some(rain_tex), Some(snow_tex)) = (atlas.sprites.get("environment/rain").copied(), atlas.sprites.get("environment/snow").copied()) else { return };
        let (cx, cy, cz) = (camera.x.floor() as i32, camera.y.floor() as i32, camera.z.floor() as i32);
        let radius = 10;
        let time = self.ticks as f32 + alpha as f32;
        for dz in -radius..=radius {
            for dx in -radius..=radius {
                let (x, z) = (cx + dx, cz + dz);
                let d2 = (dx * dx + dz * dz) as f32;
                if d2 > (radius * radius) as f32 {
                    continue;
                }
                let top = self.rain_height(x, z, cy + radius + 10);
                let y0 = (cy - radius).max(top);
                let y1 = (cy + radius).max(top);
                if y0 >= y1 {
                    continue;
                }
                let Some(snow) = self.precipitation(x, top, z) else { continue };
                let hash = (x.wrapping_mul(x).wrapping_mul(3121).wrapping_add(x.wrapping_mul(45_238_971)) ^ z.wrapping_mul(z).wrapping_mul(418_711).wrapping_add(z.wrapping_mul(13761))) & 31;
                let r = if snow { snow_tex } else { rain_tex };
                // Columns face the camera horizontally.
                let (fx, fz) = (f64::from(x) + 0.5 - camera.x, f64::from(z) + 0.5 - camera.z);
                let len = (fx * fx + fz * fz).sqrt().max(1e-3);
                let (sx, sz) = ((-fz / len * 0.5) as f32, (fx / len * 0.5) as f32);
                let center = glam::Vec3::new(fx as f32, 0.0, fz as f32);
                let scroll = if snow {
                    (time / 512.0 + hash as f32 / 32.0) % 1.0
                } else {
                    ((time + hash as f32) / 32.0 * (3.0 + (hash % 5) as f32 * 0.25)) % 1.0
                };
                let fade = ((1.0 - d2 / (radius * radius) as f32) * 0.5 + 0.5) * rain;
                let a = (fade.clamp(0.0, 1.0) * 255.0) as u32;
                let color = a << 24 | 0xFF_FFFF;
                let light = self.world.light(x, top.max(cy), z);
                let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
                let (b, t) = ((f64::from(y0) - camera.y) as f32, (f64::from(y1) - camera.y) as f32);
                let vh = r.v1 - r.v0;
                let corners = [
                    center + glam::Vec3::new(-sx, b, -sz),
                    center + glam::Vec3::new(sx, b, sz),
                    center + glam::Vec3::new(sx, t, sz),
                    center + glam::Vec3::new(-sx, t, -sz),
                ];
                // The rain texture is a vertical strip; sample a scrolling window of it.
                let s0 = scroll * 0.75;
                let uv0 = r.v0 + vh * s0;
                let uv1 = r.v0 + vh * (s0 + 0.25);
                let uvs = [[r.u0, uv1], [r.u1, uv1], [r.u1, uv0], [r.u0, uv0]];
                for (p, uv) in corners.iter().zip(uvs) {
                    out.push(Vertex { pos: p.to_array(), uv, layer: 0, color, light: packed });
                }
            }
        }
    }
    /// Player.startSleepInBed: night or thunder, no monsters within 8 blocks;
    /// the bed becomes the respawn point either way.
    fn try_sleep(&mut self, pos: [i32; 3]) {
        let Some(p) = self.player.as_ref() else { return };
        let time = self.world.day_time.rem_euclid(24000);
        let night = (12542..23460).contains(&time) || self.world.weather.thundering;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let props = self.res.assets.registry.state(state).map(|s| s.properties.clone()).unwrap_or_default();
        let head = if props.get("part").map(String::as_str) == Some("foot") {
            let d = facing_offset(props.get("facing").map_or("north", String::as_str));
            [pos[0] + d[0], pos[1], pos[2] + d[2]]
        } else {
            pos
        };
        self.spawn = Some(Vec3::new(f64::from(head[0]) + 0.5, f64::from(head[1]) + 0.6, f64::from(head[2]) + 0.5));
        self.level.spawn = Some([f64::from(head[0]), f64::from(head[1]) + 1.0, f64::from(head[2])]);
        let dist = ((p.position.x - f64::from(head[0]) - 0.5).powi(2) + (p.position.z - f64::from(head[2]) - 0.5).powi(2)).sqrt();
        if !night {
            self.message = Some(("You can sleep only at night or during thunderstorms".into(), Instant::now()));
            return;
        }
        if dist > 3.0 {
            self.message = Some(("You may not rest now; the bed is too far away".into(), Instant::now()));
            return;
        }
        let monsters = self.mobs.iter().any(|m| {
            m.hostile() && m.alive() && (m.position.x - p.position.x).abs() < 8.0 && (m.position.y - p.position.y).abs() < 5.0 && (m.position.z - p.position.z).abs() < 8.0
        });
        if monsters {
            self.message = Some(("You may not rest now; there are monsters nearby".into(), Instant::now()));
            return;
        }
        self.message = Some(("Respawn point set".into(), Instant::now()));
        self.sleeping = Some((head, 0));
    }
    fn tick_sleep(&mut self) {
        let Some((head, ticks)) = self.sleeping else { return };
        if self.keys.contains(&KeyCode::ShiftLeft) || self.player.as_ref().is_none_or(|p| p.health.dead_or_dying()) {
            self.sleeping = None;
            return;
        }
        if let Some(p) = self.player.as_mut() {
            p.position = Vec3::new(f64::from(head[0]) + 0.5, f64::from(head[1]) + 0.5625, f64::from(head[2]) + 0.5);
            p.velocity = Vec3::ZERO;
        }
        let ticks = ticks + 1;
        // ServerLevel: once everyone has slept 100 ticks, skip to morning and clear weather.
        if ticks >= 100 {
            let t = self.world.day_time;
            self.world.day_time = t + 24000 - t.rem_euclid(24000);
            self.world.weather.raining = false;
            self.world.weather.thundering = false;
            self.world.weather.rain_time = 0;
            self.world.weather.thunder_time = 0;
            self.sleeping = None;
        } else {
            self.sleeping = Some((head, ticks));
        }
    }
    fn creative_origin(batch: &HudBatch) -> (f32, f32) {
        ((batch.width - Creative::WIDTH) / 2.0, (batch.height - Creative::HEIGHT) / 2.0)
    }
    /// Vanilla tooltip: dark box with a purple gradient border.
    fn tooltip(&self, batch: &mut HudBatch, text: &str, x: f32, y: f32) {
        let atlas = &self.res.atlas;
        let w = HudBatch::text_width(atlas, text);
        let (x, y) = (x + 12.0, y - 12.0);
        batch.fill(atlas, x - 3.0, y - 4.0, w + 6.0, 16.0, rgba(16, 0, 16, 240));
        batch.fill(atlas, x - 3.0, y - 3.0, w + 6.0, 1.0, rgba(80, 0, 255, 80));
        batch.fill(atlas, x - 3.0, y + 10.0, w + 6.0, 1.0, rgba(40, 0, 127, 80));
        batch.text(atlas, text, x, y, u32::MAX, true);
    }
    fn draw_creative(&mut self, batch: &mut HudBatch) {
        let (ox, oy) = Self::creative_origin(batch);
        let selected = self.creative_screen.selected;
        let tabs = self.creative_screen.tabs.clone();
        let tab = self.creative_screen.tab().clone();
        let tab_sprite = |t: &horizon_client::creative::Tab, on: bool| {
            format!("gui/sprites/container/creative_inventory/tab_{}_{}_{}", if t.top { "top" } else { "bottom" }, if on { "selected" } else { "unselected" }, t.column + 1)
        };
        batch.fill(&self.res.atlas, 0.0, 0.0, batch.width, batch.height, rgba(16, 16, 16, 160));
        // Unselected tabs behind the panel, then the panel, then the selected tab.
        for (i, t) in tabs.iter().enumerate() {
            if i != selected {
                let (tx, ty) = Creative::tab_position(t);
                batch.sprite(&self.res.atlas, &tab_sprite(t, false), ox + tx, oy + ty);
            }
        }
        let background = if tab.search {
            "gui/container/creative_inventory/tab_item_search"
        } else if tab.inventory {
            "gui/container/creative_inventory/tab_inventory"
        } else {
            "gui/container/creative_inventory/tab_items"
        };
        if let Some(mut r) = self.res.atlas.sprites.get(background).copied() {
            r.u1 = r.u0 + (r.u1 - r.u0) * Creative::WIDTH / r.w as f32;
            r.v1 = r.v0 + (r.v1 - r.v0) * Creative::HEIGHT / r.h as f32;
            batch.rect_uv(ox, oy, Creative::WIDTH, Creative::HEIGHT, r, u32::MAX);
        }
        let (tx, ty) = Creative::tab_position(&tab);
        batch.sprite(&self.res.atlas, &tab_sprite(&tab, true), ox + tx, oy + ty);
        for t in &tabs {
            let (tx, ty) = Creative::tab_position(t);
            let stack = ItemStack { item: Arc::from(t.icon.as_str()), count: 1, damage: 0 };
            self.draw_item(batch, &stack, ox + tx + 5.0, oy + ty + if t.top { 9.0 } else { 7.0 });
        }
        let atlas = &self.res.atlas;
        batch.text(atlas, &tab.title, ox + 8.0, oy + 6.0, rgba(64, 64, 64, 255), false);
        if tab.search {
            let shown = format!("{}_", self.creative_screen.search);
            batch.text(atlas, &shown, ox + 82.0, oy + 6.0, u32::MAX, true);
        }
        let items = self.creative_screen.items(&self.res.names);
        let first = self.creative_screen.first_row(&self.res.names);
        let mut hovered: Option<String> = None;
        for r in 0..5 {
            for c in 0..9 {
                let Some(item) = items.get((first + r) * 9 + c) else { continue };
                let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 18.0 + r as f32 * 18.0);
                let stack = ItemStack { item: Arc::from(item.as_str()), count: 1, damage: 0 };
                self.draw_item(batch, &stack, x, y);
                if self.cursor.0 >= x && self.cursor.0 < x + 16.0 && self.cursor.1 >= y && self.cursor.1 < y + 16.0 {
                    batch.fill(&self.res.atlas, x, y, 16.0, 16.0, rgba(255, 255, 255, 128));
                    hovered = Some(item.clone());
                }
            }
        }
        for c in 0..9 {
            let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 112.0);
            if let Some(stack) = self.inventory[c].clone() {
                self.draw_item(batch, &stack, x, y);
                if self.cursor.0 >= x && self.cursor.0 < x + 16.0 && self.cursor.1 >= y && self.cursor.1 < y + 16.0 {
                    hovered = Some(stack.item.to_string());
                }
            }
        }
        let scrollable = self.creative_screen.rows(&self.res.names) > 5;
        let scroller = if scrollable {
            "gui/sprites/container/creative_inventory/scroller"
        } else {
            "gui/sprites/container/creative_inventory/scroller_disabled"
        };
        batch.sprite(&self.res.atlas, scroller, ox + 175.0, oy + 18.0 + self.creative_screen.scroll * (112.0 - 15.0));
        for t in &tabs {
            let (tx, ty) = Creative::tab_position(t);
            if self.cursor.0 >= ox + tx && self.cursor.0 < ox + tx + 26.0 && self.cursor.1 >= oy + ty && self.cursor.1 < oy + ty + 32.0 {
                self.tooltip(batch, &t.title, self.cursor.0, self.cursor.1);
            }
        }
        if let Some(stack) = self.carried.clone() {
            self.draw_item(batch, &stack, self.cursor.0 - 8.0, self.cursor.1 - 8.0);
        } else if let Some(item) = hovered {
            let name = self.res.names.item(&item);
            self.tooltip(batch, &name, self.cursor.0, self.cursor.1);
        }
    }
    fn click_creative(&mut self, right: bool) {
        let Some(size) = self.renderer.as_ref().map(|r| [r.config.width, r.config.height]) else { return };
        let batch = HudBatch::new(size);
        let (ox, oy) = Self::creative_origin(&batch);
        let (cx, cy) = self.cursor;
        let tabs = self.creative_screen.tabs.clone();
        for (i, t) in tabs.iter().enumerate() {
            let (tx, ty) = Creative::tab_position(t);
            if cx >= ox + tx && cx < ox + tx + 26.0 && cy >= oy + ty && cy < oy + ty + 32.0 {
                if t.inventory {
                    self.screen = Screen::Inventory;
                    return;
                }
                self.creative_screen.selected = i;
                self.creative_screen.scroll = 0.0;
                return;
            }
        }
        if cx >= ox + 175.0 && cx < ox + 187.0 && cy >= oy + 18.0 && cy < oy + 130.0 {
            self.creative_screen.scroll = ((cy - oy - 18.0 - 7.5) / (112.0 - 15.0)).clamp(0.0, 1.0);
            return;
        }
        let shift = self.keys.contains(&KeyCode::ShiftLeft);
        let items = self.creative_screen.items(&self.res.names);
        let first = self.creative_screen.first_row(&self.res.names);
        for r in 0..5 {
            for c in 0..9 {
                let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 18.0 + r as f32 * 18.0);
                if !(cx >= x && cx < x + 16.0 && cy >= y && cy < y + 16.0) {
                    continue;
                }
                // Clicking the grid with an item deletes it; otherwise take a stack.
                if self.carried.take().is_some() {
                    return;
                }
                let Some(item) = items.get((first + r) * 9 + c) else { return };
                let max = self.res.items.get(item).map_or(64, |i| i.max_stack);
                let stack = ItemStack { item: Arc::from(item.as_str()), count: if right { 1 } else { max }, damage: 0 };
                if shift {
                    let _ = self.give(stack);
                } else {
                    self.carried = Some(stack);
                }
                return;
            }
        }
        for c in 0..9 {
            let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 112.0);
            if cx >= x && cx < x + 16.0 && cy >= y && cy < y + 16.0 {
                std::mem::swap(&mut self.carried, &mut self.inventory[c]);
                return;
            }
        }
        // Clicking outside the panel throws the carried stack away.
        if !(cx >= ox && cx < ox + Creative::WIDTH && cy >= oy && cy < oy + Creative::HEIGHT) {
            self.carried = None;
        }
    }
    fn save_game(&mut self) -> Result<()> {
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks: Vec<_> = keys
            .iter()
            .filter_map(|k| self.world.chunks.get(k))
            .map(|c| self.with_entities(c))
            .collect();
        self.save.save(&chunks, &self.res.assets.registry, &self.res.biomes)?;
        let level = &mut self.level;
        level.creative = self.creative;
        level.time = self.world.game_time;
        level.weather = self.world.weather;
        level.day_time = self.world.day_time;
        level.spawn = self.spawn.map(|s| [s.x, s.y, s.z]);
        level.rotation = [self.yaw, self.pitch];
        level.xp_level = self.experience.level;
        level.xp_progress = self.experience.progress;
        level.xp_total = self.experience.total;
        if let Some(p) = &self.player {
            level.position = Some([p.position.x, p.position.y, p.position.z]);
            level.health = p.health.health();
            level.food = p.health.food.food;
            level.saturation = p.health.food.saturation;
        }
        level.inventory = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| s.as_ref().map(|s| (i as u8, s.item.to_string(), s.count, s.damage)))
            .collect();
        // Vanilla stores armor in slots 100 (feet) to 103 (head).
        for (i, s) in self.armor.iter().enumerate() {
            if let Some(s) = s {
                level.inventory.push((103 - i as u8, s.item.to_string(), s.count, s.damage));
            }
        }
        level.dimension = self.dim.id().to_owned();
        level.write(&self.world_root)?;
        self.save_world_state();
        self.message = Some(("Saved the game".into(), Instant::now()));
        Ok(())
    }
    fn render(&mut self) -> Result<bool> {
        let Some(size) = self.renderer.as_ref().map(|r| [r.config.width, r.config.height]) else {
            return Ok(false);
        };
        let alpha = self.clock.fraction();
        let (camera, underwater) = match &self.player {
            Some(p) => {
                let (a, b) = (self.previous_position, p.position);
                let eye = self.previous_eye + alpha as f32 * (self.eye - self.previous_eye);
                let c = glam::DVec3::new(
                    a.x + (b.x - a.x) * alpha,
                    a.y + (b.y - a.y) * alpha + f64::from(eye),
                    a.z + (b.z - a.z) * alpha,
                );
                let s = self.world.block(c.x.floor() as i32, c.y.floor() as i32, c.z.floor() as i32);
                (c, self.res.facts.get(s).fluid == Fluid::Water)
            }
            None => (glam::DVec3::new(0.5, 100.0, 0.5), false),
        };
        self.audio.listener = [camera.x, camera.y, camera.z];
        self.audio.listener_yaw = self.yaw;
        let block = [camera.x.floor() as i32, camera.y.floor() as i32, camera.z.floor() as i32];
        let biome = self
            .world
            .chunks
            .get(&(block[0] >> 4, block[2] >> 4))
            .map(|c| c.biome(block[0], block[1], block[2]))
            .unwrap_or(0);
        let info = self.res.biomes.get(biome).clone();
        let environment = if self.dim == Dim::Nether { &self.res.nether_environment } else { &self.res.environment };
        let mut sky = environment.sample(self.world.day_time, info.sky, info.fog);
        let (rain, thunder) = if self.dim == Dim::Overworld { self.world.weather.levels(alpha as f32) } else { (0.0, 0.0) };
        horizon_client::environment::apply_weather(&mut sky, rain, thunder);
        if self.flash > 0 {
            // Lightning flash brightens the sky (LevelRenderer skyFlashTime).
            sky.sky_color = [0.9, 0.9, 1.0];
            sky.sky_light_factor = 1.0;
        }
        let outline = if self.screen == Screen::Playing {
            self.pick().map(|h| {
                let p = h.pos.map(f64::from);
                [p[0], p[1], p[2], p[0] + 1.0, p[1] + 1.0, p[2] + 1.0]
            })
        } else {
            None
        };
        let hud = self.hud(size);
        let title = self.title.is_some();
        let entities = if title { Vec::new() } else { self.entity_geometry(camera, alpha) };
        let celestial = if title {
            self.panorama_geometry()
        } else if self.dim == Dim::Overworld {
            self.celestial_geometry(&sky)
        } else {
            Vec::new()
        };
        let hand = if title || self.screen == Screen::Inventory { Vec::new() } else { self.hand_geometry(alpha) };
        let mut entities = entities;
        if !title && self.dim == Dim::Overworld {
            self.cloud_geometry(&sky, camera, alpha, &mut entities);
            self.weather_geometry(camera, alpha, &mut entities);
        }
        // PanoramaRenderer: the title cube spins at 0.1 degrees per tick, pitched 10 degrees.
        let (yaw, pitch) = if title { (-(self.started.elapsed().as_secs_f32() * 2.0) % 360.0, 10.0) } else { (self.yaw, self.pitch) };
        let water_fog = {
            let c = info.water_fog;
            [(c >> 16 & 255) as f32 / 255.0, (c >> 8 & 255) as f32 / 255.0, (c & 255) as f32 / 255.0]
        };
        let frame = Frame {
            camera,
            yaw,
            pitch,
            fov_degrees: if title { 85.0 } else { self.settings.fov },
            bob: (self.settings.view_bobbing && !title && self.player.is_some()).then(|| {
                let a = alpha as f32;
                (self.walk.0 + (self.walk.1 - self.walk.0) * a, self.bob.0 + (self.bob.1 - self.bob.0) * a)
            }),
            sky,
            distance: (self.streaming.radius * 16) as f32,
            underwater,
            title,
            skybox: self.dim == Dim::Overworld,
            environment_fog: (self.dim == Dim::Nether).then_some((sky.fog_start, sky.fog_end)),
            // The camera inside an opaque block sees nothing through it: disable.
            cave_culling: !self.res.facts.get(self.world.block(block[0], block[1], block[2])).solid_render,
            water_fog,
            outline,
            hud: &hud,
            entities: &entities,
            celestial: &celestial,
            hand: &hand,
            game_time: self.world.game_time as f32,
            ticks: self.ticks,
        };
        self.renderer.as_mut().ok_or("renderer")?.render(&frame)
    }
    /// A key press while a world is open (keyboard or control socket).
    fn key_pressed(&mut self, key: KeyCode) -> Result<()> {
        match key {
            KeyCode::Escape => match self.screen {
                Screen::Options => self.options_key(KeyCode::Escape, None)?,
                Screen::Playing => {
                    self.screen = Screen::Paused;
                    self.capture(false);
                }
                Screen::Creative => {
                    self.carried = None;
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
                Screen::Paused | Screen::Inventory => {
                    self.close_container();
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
                Screen::Dead => {}
                Screen::Chat => self.close_chat(),
            },
            KeyCode::KeyT if self.screen == Screen::Playing => self.open_chat(""),
            KeyCode::Slash if self.screen == Screen::Playing => self.open_chat("/"),
            KeyCode::F1 => self.toggle_hide_gui(),
            KeyCode::F11 => self.toggle_fullscreen(),
            KeyCode::F2 => self.take_screenshot(),
            KeyCode::KeyE if self.screen == Screen::Creative && self.creative_screen.tab().search => {}
            KeyCode::KeyE => match self.screen {
                Screen::Playing => {
                    self.screen = if self.creative { Screen::Creative } else { Screen::Inventory };
                    self.capture(false);
                }
                Screen::Creative => {
                    if let Some(c) = self.carried.take() {
                        let _ = self.give(c);
                    }
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
                Screen::Inventory => {
                    self.close_container();
                    self.screen = Screen::Playing;
                    self.capture(true);
                }
                _ => {}
            },
            KeyCode::F3 => self.debug = !self.debug,
            KeyCode::KeyQ if self.screen == Screen::Playing => {
                let all = self.keys.contains(&KeyCode::ControlLeft);
                self.drop_held(all);
            }
            KeyCode::Digit1 | KeyCode::Digit2 | KeyCode::Digit3 | KeyCode::Digit4 | KeyCode::Digit5 | KeyCode::Digit6 | KeyCode::Digit7 | KeyCode::Digit8 | KeyCode::Digit9 => {
                self.selected = match key {
                    KeyCode::Digit1 => 0,
                    KeyCode::Digit2 => 1,
                    KeyCode::Digit3 => 2,
                    KeyCode::Digit4 => 3,
                    KeyCode::Digit5 => 4,
                    KeyCode::Digit6 => 5,
                    KeyCode::Digit7 => 6,
                    KeyCode::Digit8 => 7,
                    _ => 8,
                };
            }
            _ => {}
        }
        Ok(())
    }
    /// A mouse button while playing (window or control socket).
    fn play_mouse(&mut self, button: MouseButton, pressed: bool) -> Result<()> {
        match button {
            MouseButton::Left => {
                if pressed && self.attack() {
                    return Ok(());
                }
                self.left = pressed;
                if pressed {
                    self.destroy_delay = 0;
                }
            }
            MouseButton::Right => {
                self.right = pressed;
                if pressed {
                    self.use_item()?;
                    self.use_delay = 4;
                }
            }
            MouseButton::Middle if pressed && self.creative => {
                if let Some(hit) = self.pick() {
                    let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                    let item = self.res.facts.get(state).item.clone();
                    if &*item != "minecraft:air" {
                        self.inventory[self.selected] = Some(ItemStack { item, count: 1, damage: 0 });
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn update_world(&mut self) {
        if let Some(p) = &self.player {
            self.streaming.center = ((p.position.x.floor() as i32) >> 4, (p.position.z.floor() as i32) >> 4);
        }
        let (uploads, unloaded) = self.streaming.update(&mut self.world);
        let dirty: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        let modified: Vec<_> = dirty.iter().map(|c| self.with_entities(c)).collect();
        for c in &unloaded {
            self.world.block_entities.retain(|p, _| !(p.0 >> 4 == c.x && p.2 >> 4 == c.z));
        }
        if !modified.is_empty()
            && let Err(e) = self.save.save(&modified, &self.res.assets.registry, &self.res.biomes)
        {
            eprintln!("Horizon: failed to save unloaded chunks: {e}");
        }
        if let Some(renderer) = self.renderer.as_mut() {
            for (key, mesh) in uploads {
                renderer.upload(key, &mesh);
            }
            for c in unloaded {
                for sy in 0..(HEIGHT / 16) {
                    renderer.remove((c.x, sy, c.z));
                }
            }
        }
    }
}
impl ApplicationHandler for Game<'_> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Horizon")
                .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0)),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.to_string());
                event_loop.exit();
                return;
            }
        };
        let assets = &self.res.assets;
        match pollster::block_on(Renderer::new(
            window.clone(),
            &assets.pixels,
            assets.sprite_size,
            assets.texture_layers,
            &assets.animations,
            (&self.res.atlas.pixels, self.res.atlas.width, self.res.atlas.height),
        )) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                self.error = Some(e.to_string());
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.previous = Instant::now();
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        let result: Result<()> = (|| {
            match event {
                WindowEvent::CloseRequested => {
                    if self.title.is_none() {
                        self.save_game()?;
                    }
                    event_loop.exit();
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    if !focused && self.control.is_none() {
                        self.keys.clear();
                        self.left = false;
                        self.right = false;
                        self.capture(false);
                        if self.screen == Screen::Playing && self.player.is_some() && !self.options.no_pause {
                            self.screen = Screen::Paused;
                        }
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(r) = &self.renderer {
                        let b = HudBatch::new([r.config.width, r.config.height]);
                        self.cursor = (position.x as f32 / b.scale, position.y as f32 / b.scale);
                        if self.dragging && self.screen == Screen::Options {
                            let _ = b;
                            self.options_drag()?;
                        }
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let d = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y,
                        MouseScrollDelta::PixelDelta(p) => p.y as f32,
                    };
                    if self.screen == Screen::Chat && d != 0.0 {
                        self.chat_scroll(d);
                    }
                    if self.screen == Screen::Options && d != 0.0 {
                        self.options_scroll(d);
                    }
                    if self.screen == Screen::Creative && d != 0.0 {
                        let rows = self.creative_screen.rows(&self.res.names).saturating_sub(5).max(1) as f32;
                        self.creative_screen.scroll = (self.creative_screen.scroll - d.signum() / rows).clamp(0.0, 1.0);
                    }
                    if self.screen == Screen::Playing && d != 0.0 {
                        self.selected = (self.selected as i32 - d.signum() as i32).rem_euclid(9) as usize;
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.screen == Screen::Options => {
                    if self.options_waiting_for_key() && state == ElementState::Pressed {
                        self.options_bind_mouse(button)?;
                    } else if matches!(button, MouseButton::Left | MouseButton::Right) {
                        self.options_mouse(state == ElementState::Pressed, button == MouseButton::Right)?;
                    }
                }
                WindowEvent::MouseInput { state, button: MouseButton::Left, .. }
                    if self.title.is_some() && state == ElementState::Pressed =>
                {
                    let size = self.renderer.as_ref().map(|r| [r.config.width, r.config.height]).unwrap_or([1, 1]);
                    let b = HudBatch::new(size);
                    let action = self.title.as_mut().map_or(Action::None, |t| t.click(self.cursor.0, self.cursor.1, (b.width, b.height)));
                    match action {
                        Action::Quit => event_loop.exit(),
                        Action::Options => self.open_options(true),
                        Action::Play { folder } => self.open_world(&folder, None, None, false)?,
                        Action::Create { folder, name, seed, creative } => self.open_world(&folder, Some(&name), seed, creative)?,
                        Action::None => {}
                    }
                }
                WindowEvent::MouseInput { .. } if self.title.is_some() => {}
                WindowEvent::MouseInput { state, button, .. } => {
                    let pressed = state == ElementState::Pressed;
                    match self.screen {
                        Screen::Playing => {
                            if pressed && !self.captured {
                                self.capture(true);
                                return Ok(());
                            }
                            match self.settings.keys.translate_mouse(button) {
                                Some(Err(b)) => self.play_mouse(b, pressed)?,
                                Some(Ok(k)) if pressed => {
                                    self.keys.insert(k);
                                    self.key_pressed(k)?;
                                }
                                Some(Ok(k)) => {
                                    self.keys.remove(&k);
                                }
                                None => {}
                            }
                        }
                        Screen::Inventory if pressed => {
                            self.click_inventory(button == MouseButton::Right);
                        }
                        Screen::Chat if pressed && button == MouseButton::Left => self.chat_click(),
                        Screen::Creative if pressed => {
                            self.click_creative(button == MouseButton::Right);
                        }
                        Screen::Paused if pressed => self.pause_click()?,
                        Screen::Dead if pressed => self.death_click()?,
                        _ => {}
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::Chat && self.title.is_none() => {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        if event.state == ElementState::Pressed {
                            self.keys.insert(key);
                            self.chat_key(&event);
                        } else {
                            self.keys.remove(&key);
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Creative && self.creative_screen.tab().search && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    match event.physical_key {
                        PhysicalKey::Code(KeyCode::Backspace) => {
                            self.creative_screen.search.pop();
                        }
                        _ => {
                            if let Some(text) = &event.text {
                                self.creative_screen.search.extend(text.chars().filter(|c| !c.is_control()));
                            }
                        }
                    }
                    self.creative_screen.scroll = 0.0;
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::Options => {
                    if event.state == ElementState::Pressed
                        && let PhysicalKey::Code(key) = event.physical_key
                    {
                        self.options_key(key, event.text.as_deref())?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.title.is_some() => {
                    if event.state == ElementState::Pressed
                        && let Some(title) = self.title.as_mut()
                    {
                        match event.physical_key {
                            PhysicalKey::Code(KeyCode::Backspace) => title.backspace(),
                            PhysicalKey::Code(KeyCode::Tab) => title.tab(),
                            PhysicalKey::Code(KeyCode::Escape) => title.escape(),
                            _ => {
                                if let Some(text) = &event.text {
                                    title.type_text(text);
                                }
                            }
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    let PhysicalKey::Code(key) = event.physical_key else { return Ok(()) };
                    // Key binds: act as the default key of the bound mapping.
                    let key = match self.settings.keys.translate(key) {
                        Some(Ok(k)) => k,
                        Some(Err(button)) => {
                            if self.screen == Screen::Playing && !event.repeat {
                                self.play_mouse(button, event.state == ElementState::Pressed)?;
                            }
                            return Ok(());
                        }
                        None => return Ok(()),
                    };
                    if event.state != ElementState::Pressed {
                        self.keys.remove(&key);
                        return Ok(());
                    }
                    self.keys.insert(key);
                    if event.repeat {
                        return Ok(());
                    }
                    self.key_pressed(key)?;
                }
                WindowEvent::RedrawRequested => {
                    let now = Instant::now();
                    let elapsed = now.duration_since(self.previous);
                    self.previous = now;
                    // Arrow keys turn the camera (accessibility / scripted testing).
                    let turn = |k| f32::from(u8::from(self.keys.contains(&k)));
                    let secs = elapsed.as_secs_f32().min(0.1);
                    self.yaw += (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0 * secs;
                    self.pitch = (self.pitch + (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0 * secs).clamp(-90.0, 90.0);
                    if self.title.is_none() {
                        self.update_world();
                        self.spawn_player()?;
                    }
                    let steps = self.clock.advance(elapsed);
                    for _ in 0..steps {
                        if self.title.is_none() {
                            self.tick()?;
                        }
                    }
                    let rendered = self.render()?;
                    let finished = self.renderer.as_mut().and_then(|r| r.screenshot_result.take());
                    if let Some(result) = &finished
                        && self.ui.screenshot_message.is_some()
                    {
                        self.finish_screenshot(result);
                    }
                    if let Some(result) = finished
                        && let Some(reply) = self.pending_screenshot.take()
                    {
                        let _ = reply.send(match result {
                            Ok([w, h]) => serde_json::json!({ "ok": true, "width": w, "height": h }),
                            Err(e) => serde_json::json!({ "error": e }),
                        }.to_string());
                    }
                    if rendered {
                        self.frame_stats.record(elapsed.as_secs_f32() * 1000.0);
                        self.frames += 1;
                        self.fps_frames += 1;
                        if self.fps_at.elapsed() >= Duration::from_secs(1) {
                            self.fps = self.fps_frames as f32 / self.fps_at.elapsed().as_secs_f32();
                            self.fps_frames = 0;
                            self.fps_at = Instant::now();
                        }
                        if self.options.frames.is_some_and(|f| self.frames >= f) {
                            event_loop.exit();
                        }
                    }
                }
                _ => {}
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.error = Some(e.to_string());
            event_loop.exit();
        }
    }
    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if self.captured
            && self.focused
            && self.screen == Screen::Playing
            && let DeviceEvent::MouseMotion { delta: (x, y) } = event
        {
            // MouseHandler.turnPlayer at default sensitivity 0.5: 0.15 degrees per count.
            let s = f64::from(self.settings.sensitivity) * 0.6 + 0.2;
            let f = s * s * s * 8.0;
            self.yaw += (x * f * 0.15) as f32;
            self.pitch = (self.pitch + (y * f * 0.15) as f32).clamp(-90.0, 90.0);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_control(event_loop);
        if self.options.frames.is_some() && self.started.elapsed() > self.options.timeout {
            self.error = Some("frame-limited run timed out".into());
            event_loop.exit();
            return;
        }
        if let Some(due) = self.next_frame_due() {
            event_loop.set_control_flow(ControlFlow::WaitUntil(due));
            return;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
}
pub fn run() -> Result<()> {
    let Some(mut options) = Options::read()? else { return Ok(()) };
    let started = Instant::now();
    let vanilla_root = options.resources.clone();
    let early_settings = Settings::load(&std::env::current_dir()?.join("options.txt"));
    options.resources = packs::startup_root(&vanilla_root, &early_settings.resource_packs);
    let root = options.resources.clone();
    let assets = Arc::new(Assets::load(&root).map_err(|e| {
        format!("{e}\nPrepare resources with: python3 tools/prepare_native_client.py --prepare")
    })?);
    eprintln!("load assets: {:.2}s", started.elapsed().as_secs_f32());
    let facts = BlockFacts::load(&root, &assets.registry)?;
    eprintln!("load facts: {:.2}s", started.elapsed().as_secs_f32());
    let items = Items::load(&root, &assets.registry, &facts)?;
    eprintln!("load items: {:.2}s", started.elapsed().as_secs_f32());
    let recipes = Recipes::load(&root, &items)?;
    let biomes = Arc::new(Biomes::load(&root)?);
    let environment = Environment::load(&root)?;
    eprintln!("load biomes+env: {:.2}s", started.elapsed().as_secs_f32());
    let visibility = models::load_visibility(&root, &assets.registry)?;
    eprintln!("load visibility: {:.2}s", started.elapsed().as_secs_f32());
    let tables = Arc::new(MeshTables::new(&assets, &facts, visibility)?);
    let light = Arc::new(LightTables::new(&facts));
    eprintln!("load tables: {:.2}s", started.elapsed().as_secs_f32());
    let atlas = HudAtlas::build(&root)?;
    horizon_client::ui::load_metadata(&root);
    horizon_client::lang::set(horizon_client::lang::Language::load(&[&root], &early_settings.language));
    eprintln!("load atlas: {:.2}s", started.elapsed().as_secs_f32());
    let layer_names = assets
        .sprite_layers
        .iter()
        .map(|(name, layer)| (*layer, name.trim_start_matches("minecraft:").to_owned()))
        .collect();
    let registry_owner = assets.clone();
    let collision = collisions::load(&root, &registry_owner.registry)?;
    let motion = MotionResources::load(&root, &registry_owner)?;
    eprintln!("load collision+motion: {:.2}s", started.elapsed().as_secs_f32());
    let context = Arc::new(WorkerContext {
        assets: assets.clone(),
        tables,
        biomes: biomes.clone(),
        light,
    });
    let threads = std::thread::available_parallelism().map_or(2, |n| n.get()).saturating_sub(1).max(1);
    eprintln!(
        "Horizon: resources loaded in {:.1}s; {} block states, {} items, {} biomes; {} worker threads",
        started.elapsed().as_secs_f32(),
        facts.states.len(),
        items.items.len(),
        biomes.list.len(),
        threads
    );
    // Until a world is opened the streamer is idle.
    let mut streaming = Streaming::new(
        Arc::new(TestTerrain::new(assets.registry, &biomes)?),
        context.clone(),
        1,
    );
    streaming.paused = true;
    let level = Level::default();
    let save = Arc::new(SaveDir::new(std::env::current_dir()?.join("saves").join(".horizon-title")));
    let audio = Audio::new(&root);
    let world = World::new(facts.air, 0);
    let creative = options.creative;
    let mut resources = Resources {
        assets,
        facts,
        items,
        recipes,
        biomes,
        environment,
        atlas,
        icons: HashMap::new(),
        layer_names,
        providers: Providers::load(&root),
        chest_model: horizon_client::mobs::load_model(&root, "minecraft:chest#main"),
        nether_environment: Environment::load_dimension(&root, "the_nether", false)?,
        behaviours: Behaviours::load(registry_owner.registry, &root)?,
        names: Names::load(&root),
        clouds: horizon_client::hud::decode(&root.join("assets/minecraft/textures/environment/clouds.png"))
            .map(|(w, h, px)| (w, h, px.chunks_exact(4).map(|p| p[3] > 0).collect()))
            .unwrap_or((1, 1, vec![false])),
        mob_types: MobTypes::load(&root)?.types.into_iter().map(|(k, v)| (k, Arc::new(v))).collect(),
        context,
        threads,
        player_arm: horizon_client::mobs::load_model(&root, "minecraft:player#main")
            .and_then(|m| m.children.into_iter().find(|c| c.name == "right_arm")),
        displays: HashMap::new(),
    };
    let inventory = vec![None; 36];
    let now = Instant::now();
    let game_root = options.resources.clone();
    let mut settings = Settings::load(&std::env::current_dir()?.join("options.txt"));
    if let Some(d) = options.distance_override {
        settings.render_distance = d;
    }
    let look = options.look;
    let mut game = Game {
        res: &mut resources,
        collision: &collision,
        motion: &motion,
        world,
        streaming,
        options,
        player: None,
        spawn: None,
        clock: FixedStep::default(),
        previous_position: Vec3::ZERO,
        eye: 1.62,
        previous_eye: 1.62,
        yaw: look.map_or(level.rotation[0], |l| l.0),
        pitch: look.map_or(level.rotation[1], |l| l.1),
        inventory,
        selected: 0,
        carried: None,
        craft_grid: vec![None; 9],
        craft_size: 2,
        container: None,
        scratch_slot: None,
        mining: None,
        destroy_delay: 0,
        use_delay: 0,
        creative,
        keys: HashSet::new(),
        left: false,
        right: false,
        cursor: (0.0, 0.0),
        captured: false,
        focused: true,
        debug: false,
        screen: Screen::Playing,
        window: None,
        renderer: None,
        started: now,
        previous: now,
        frames: 0,
        fps: 0.0,
        fps_frames: 0,
        fps_at: now,
        ticks: 0,
        error: None,
        message: None,
        save,
        level,
        audio,
        dropped: Vec::new(),
        next_entity: 0,
        particles: Vec::new(),
        spawn_search: 0,
        mobs: Vec::new(),
        arrows: Vec::new(),
        attack_ticks: 100,
        last_hurt_by_player: HashSet::new(),
        chest_open: HashMap::new(),
        using: None,
        title: None,
        armor: [None, None, None, None],
        applied_modifiers: Vec::new(),
        experience: Experience::default(),
        orbs: Vec::new(),
        xp_delay: 0,
        last_level_up: 0,
        swing: None,
        swing_anim: (0.0, 0.0),
        hand_height: (1.0, 1.0),
        hand_item: None,
        settings,
        options_from_title: false,
        dragging: false,
        walk: (0.0, 0.0),
        bob: (0.0, 0.0),
        flash: 0,
        sleeping: None,
        dim: Dim::Overworld,
        world_root: PathBuf::new(),
        travelling: None,
        portal_ticks: 0,
        portal_cooldown: 0,
        block_ticks: Ticks::new(0),
        tick_chunks: HashSet::new(),
        level_events: Vec::new(),
        falling: Vec::new(),
        creative_screen: Creative::load(&game_root)?,
        control: None,
        frame_stats: Default::default(),
        pending_screenshot: None,
        pending_bench: None,
        ui: Default::default(),
    };
    game.ui.vanilla_root = vanilla_root;
    game.res.atlas.font.uniform = game.settings.force_unicode;
    game.res.atlas.font.japanese = game.settings.japanese_glyphs;
    game.ui.chat.options = game.settings.chat_options();
    if let Some(path) = &game.options.control {
        let server = horizon_client::control::Server::start(path)
            .map_err(|e| format!("control socket {}: {e}", path.display()))?;
        eprintln!("Horizon: control socket {}", server.path().display());
        game.control = Some(server);
    }
    match game.options.world.clone() {
        Some(folder) => {
            let (seed, creative) = (game.options.seed, game.options.creative);
            game.open_world(&folder, None, Some(seed), creative)?;
        }
        None => game.title = Some(Title::new(std::env::current_dir()?.join("saves"))),
    }
    EventLoop::new()?.run_app(&mut game)?;
    if let Some(path) = &game.options.report {
        let report = serde_json::json!({
            "frames": game.frames,
            "elapsed_seconds": game.started.elapsed().as_secs_f64(),
            "chunks": game.world.chunks.len(),
            "generated": game.streaming.generated_total,
            "meshed_sections": game.streaming.meshed_total,
            "player": game.player.as_ref().map(|p| [p.position.x, p.position.y, p.position.z]),
            "error": game.error,
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    if let Some(e) = game.error {
        return Err(e.into());
    }
    Ok(())
}
