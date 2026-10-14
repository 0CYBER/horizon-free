//! The playable game loop: streaming world, player simulation at 20 ticks/s,
//! block interaction, inventory and HUD.
mod advancements;
mod advancements_screen;
mod agent;
mod allays;
mod armor_stands;
mod backdrop;
mod banners;
mod beacon;
mod block_entity_anims;
mod blockctl;
mod boats;
mod books;
mod bossbars;
mod breezes;
mod cartography;
mod chat;
mod clipboard;
mod combat;
mod command_harness;
mod commands;
mod composter;
mod control;
mod copper_golems;
mod creaking;
mod creatures;
mod crossbow;
mod debug_keys;
mod dedicated;
mod dispense_fixture;
mod dispenser;
mod dragon;
mod enchant_effects;
mod end;
mod explosion;
mod fangs;
mod fireworks;
mod fishing;
mod fov;
mod frames;
mod functions;
mod game_mode;
mod ghasts;
mod golems;
mod growth;
mod hand;
mod item_draw;
mod item_particles;
mod jukebox;
mod lan;
mod lan_actions;
mod lan_screen;
mod lan_trading;
mod leads;
mod level_particles;
mod lightning;
mod llamas;
mod loom;
mod machines;
mod maps;
mod minecarts;
mod mob_blocks;
mod mob_gear;
mod mount_menu;
mod mounts;
mod music;
mod names;
mod natural_spawning;
mod nautili;
mod net_harness;
mod options;
mod packs;
mod paintings;
mod passengers;
mod persist;
mod perspective;
mod piglins;
mod postprocess;
mod potions;
mod powder_snow;
mod raids;
mod recipe_book;
mod remote;
mod render_control;
mod render_tick;
mod scoreboard;
mod screens;
mod sculk;
mod sculk_fixture;
mod shield;
mod sign_edit;
mod sniffers;
mod spill;
mod station_control;
mod stations;
mod stats;
mod stats_screen;
mod storage;
mod structures;
mod subtitle_hud;
mod sulfur_cubes;
mod titles;
mod tnt;
mod totem;
mod trading;
mod trident;
mod villagers;
mod wandering_trader;
mod wind;
mod wither;
mod zombie_villagers;
use horizon_client::{
    Result,
    assets::Assets,
    audio::Audio,
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    chunk::{HEIGHT, MIN_Y},
    clock::FixedStep,
    collisions,
    containers::{BlockEntity, Providers},
    crafting::Recipes,
    creative::{Creative, Names},
    entities::{Arrow, Experience, ExperienceOrb, ItemEntity},
    environment::Environment,
    gfx::{Frame, Renderer},
    hud::{HudAtlas, HudBatch, rgba},
    hud_layout, hud_overlays,
    items::{ItemStack, Items},
    keymapping::KeyAction,
    light::LightTables,
    mesher::{MeshTables, Vertex},
    mobs::{Mob, MobType, MobTypes},
    models,
    motion::{MotionEvent, MotionResources, MotionWorld, PlayerMotion},
    mouse::{LookOptions, MouseLook, Rotation},
    particles::Particle,
    save::{Level, Persistent, SaveDir},
    settings::Settings,
    sprint::Keys,
    streaming::{Streaming, WorkerContext},
    testgen::TestTerrain,
    title::{Action, Title},
    vanillagen::VanillaTerrain,
    world::World,
};
use horizon_core::{
    biome::{BiomeZoom, obfuscate_seed},
    geometry::{Aabb, Direction},
    position::BlockPos,
};
use horizon_core::{geometry::Vec3, random::RandomSource};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    level::{
        Behaviours, Dimension as LevelDimension, EntityQuery, Landing, LevelAccess, LevelEvent,
        PlaceContext, PlaceOutcome, PlayerContext, PlayerEffect, Ticks, flags as level_flags,
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
    /// `HORIZON_SPIKES=MS`: print the timed spans of frames slower than MS.
    spikes: Option<f32>,
    /// Render into a texture instead of the window (`--offscreen`), so a
    /// GPU the display cannot present from can still be benchmarked.
    offscreen: bool,
    /// Multiplayer: `--server HOST[:PORT]` joins at start; `--name` is the offline player name.
    server: Option<String>,
    player_name: String,
    /// `--dedicated`: run as a dedicated server; vanilla's server arguments
    /// (`--nogui`, `--port`, `--world`, `--universe`, `--initSettings`) follow.
    dedicated: Option<Vec<String>>,
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
            offscreen: false,
            spikes: std::env::var("HORIZON_SPIKES")
                .ok()
                .and_then(|v| v.parse().ok()),
            server: None,
            player_name: std::env::var("HORIZON_PLAYER_NAME").unwrap_or_else(|_| "Player".into()),
            dedicated: None,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "Horizon — Minecraft Java Edition 26.3 in Rust\n\n  --resources PATH        Prepared resources (horizon-prepare)\n  --render-distance N     Chunks, 2..64 (default 12)\n  --world NAME            World folder under saves/ (default \"New World\")\n  --seed N                World seed for new worlds (default 0)\n  --creative              Creative mode (default survival)\n  --test-terrain          Simple test terrain instead of vanilla generation\n  --frames N              Exit after N frames (testing)\n  --smoke-report PATH     Write a JSON report on exit\n  --control               Accept horizon-ctl commands on a local socket\n  --control-socket PATH   Control socket path (default $XDG_RUNTIME_DIR/horizon-control.sock)\n  --server HOST[:PORT]    Join a multiplayer server (offline mode) at start\n  --name NAME             Player name for multiplayer (default Player)\n\nControls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,\nleft click break, right click place/use, 1-9/scroll hotbar, E inventory,\nQ drop, F3 debug, Esc pause."
                    );
                    return Ok(None);
                }
                "--dedicated" => {
                    o.dedicated = Some(args.by_ref().collect());
                    break;
                }
                "--creative" => o.creative = true,
                "--test-terrain" => o.test_terrain = true,
                "--no-pause" => o.no_pause = true,
                "--offscreen" => o.offscreen = true,
                "--control" => o.control = Some(horizon_client::control::default_path()),
                _ => {
                    let value = args
                        .next()
                        .ok_or_else(|| format!("missing value after {arg}"))?;
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
                                    .fold(0i32, |h, c| {
                                        h.wrapping_mul(31).wrapping_add(i32::from(c))
                                    })
                                    .into()
                            })
                        }
                        "--frames" => o.frames = Some(value.parse()?),
                        "--timeout-seconds" => o.timeout = Duration::from_secs(value.parse()?),
                        "--smoke-report" => o.report = Some(value.into()),
                        "--screenshot-after" => o.screenshot_after = Some(value.parse()?),
                        "--world" => o.world = Some(value),
                        "--server" => o.server = Some(value),
                        "--name" => o.player_name = value,
                        "--control-socket" => o.control = Some(value.into()),
                        "--time" => o.time = Some(value.parse()?),
                        "--weather" => o.weather = Some(value),
                        "--dimension" => {
                            o.dimension = Some(if value.contains(':') {
                                value
                            } else {
                                format!("minecraft:{value}")
                            })
                        }
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
    /// The Statistics screen.
    Stats,
    /// The Advancements screen.
    Advancements,
    /// WorldOptionsScreen (game/lan_screen.rs).
    WorldOptions,
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
    /// Minecart boxes (detector rails).
    cart_boxes: Vec<([f64; 3], [f64; 3])>,
    biomes: &'w Biomes,
    seed: i64,
    behaviours: &'w Behaviours,
    items: &'w Items,
    registry: &'static horizon_world::registry::BlockRegistry,
    rules: horizon_world::level::GameRules,
    /// Block loot random sequences (`ServerLevel.getRandomSequence`).
    rand_sequences: &'w mut horizon_client::random_sequences::RandomSequences,
    pending: &'w mut spill::Pending,
    spill: &'w mut spill::Spill,
}
impl LevelAccess for GameLevel<'_> {
    fn precipitation(&self, pos: BlockPos) -> Option<horizon_world::level::Precipitation> {
        self.behaviours.precipitation_info(self, pos)
    }
    fn is_raining_at(&self, pos: BlockPos) -> bool {
        self.behaviours.is_raining_at(self, pos)
    }
    fn block_entity_analog_signal(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        if let Some(signal) = sculk::sensor_signal(self.world, self.behaviours, state, pos) {
            return signal;
        }
        if let Some((_, comparator)) = jukebox::jukebox_signals(self.world, self.items, pos) {
            return comparator;
        }
        if let Some(signal) = creaking::heart_signal(self.world, pos) {
            return if self.registry.state(state).is_some_and(|s| {
                s.properties
                    .get("creaking_heart_state")
                    .is_some_and(|v| v == "uprooted")
            }) {
                0
            } else {
                signal
            };
        }
        if let Some(signal) = books::lectern_signal(self.world, pos) {
            return signal;
        }
        machines::container_signal(self.world, self.behaviours, self.items, self.registry, pos)
    }
    fn block_entity_signal(
        &self,
        pos: BlockPos,
        _state: BlockStateId,
        _direction: Direction,
    ) -> i32 {
        jukebox::jukebox_signals(self.world, self.items, pos).map_or(0, |(signal, _)| signal)
    }
    fn biome_name(&self, pos: BlockPos) -> Option<&str> {
        let chunk = self.world.chunks.get(&(pos.x >> 4, pos.z >> 4))?;
        Some(&self.biomes.get(chunk.biome(pos.x, pos.y, pos.z)).name)
    }
    fn world_seed(&self) -> i64 {
        self.seed
    }
    fn day_time(&self) -> i64 {
        self.world.day_time
    }
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
        if self.dimension != LevelDimension::Overworld {
            0
        } else {
            MIN_Y
        }
    }
    fn max_y(&self) -> i32 {
        // Exclusive, as LevelAccess::max_y documents.
        if self.dimension != LevelDimension::Overworld {
            256
        } else {
            MIN_Y + HEIGHT
        }
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
        let Some((lo, hi)) = self.player_box else {
            return true;
        };
        if !self.facts.get(state).full_collision {
            return true;
        }
        let (x, y, z) = (f64::from(pos.x), f64::from(pos.y), f64::from(pos.z));
        !(x < hi[0] - 1e-7
            && x + 1.0 > lo[0] + 1e-7
            && y < hi[1] - 1e-7
            && y + 1.0 > lo[1] + 1e-7
            && z < hi[2] - 1e-7
            && z + 1.0 > lo[2] + 1e-7)
    }
    /// Entities that trigger pressure plates, tripwires and detector rails:
    /// the player (a mob) and the minecarts.
    fn count_entities(&self, area: Aabb, query: EntityQuery) -> usize {
        let hit = |(lo, hi): &([f64; 3], [f64; 3])| {
            area.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ))
        };
        let carts = || self.cart_boxes.iter().filter(|b| hit(b)).count();
        match query {
            EntityQuery::Minecart => carts(),
            EntityQuery::Mobs => self.player_box.iter().filter(|b| hit(b)).count(),
            EntityQuery::Everything => self.player_box.iter().filter(|b| hit(b)).count() + carts(),
        }
    }
    fn emit(&mut self, event: LevelEvent) {
        match event {
            // Block.dropResources draws its loot and spots right away.
            LevelEvent::DropResources { pos, state } => self.drop_resources_now(pos, state),
            // Block.popResource of the block's own item (tall flowers, flower beds).
            LevelEvent::BlockEvent {
                pos,
                state,
                kind: "pop_block_item",
            } => self.pop_block_item(pos, state),
            // ComposterBlock.extractProduce: bone meal above the composter.
            LevelEvent::Player {
                pos,
                effect: PlayerEffect::DropItem(item),
                ..
            } => self.pop_above(pos, item),
            event => self.events.push(event),
        }
    }
    fn remove_block_entity(&mut self, pos: BlockPos, old: BlockStateId, side_effects: bool) -> u8 {
        self.remove_entity(pos, old, side_effects)
    }
}
/// FallingBlockEntity carrying a block state.
#[derive(Clone, Debug)]
struct FallingBlock {
    state: BlockStateId,
    position: Vec3,
    previous: Vec3,
    velocity: Vec3,
    /// `FallingBlockEntity.time`: signed, a save can start it below zero.
    age: i32,
    /// `Entity.isNoGravity`.
    no_gravity: bool,
    /// `FallingBlockEntity.dropItem`.
    drop_item: bool,
    /// `Entity.fallDistance` (powder snow holds a block that fell far at 0.9).
    fall_distance: f64,
    /// The UUID and compound it was loaded with, kept while it moves.
    saved: Option<Box<([i32; 4], horizon_world::nbt::Tag)>>,
}
pub struct Resources {
    assets: Arc<Assets>,
    facts: BlockFacts,
    items: Items,
    recipes: Recipes,
    transformers: horizon_client::block_transform::Transformers,
    biomes: Arc<Biomes>,
    environment: Environment,
    atlas: HudAtlas,
    layer_names: HashMap<u32, String>,
    mob_types: HashMap<String, Arc<MobType>>,
    mob_data: horizon_client::mobs::MobData,
    providers: Providers,
    block_entity_render: horizon_client::block_entity_render::BlockEntityRenderer,
    animate: horizon_client::animate_tick::AnimateTables,
    fog: horizon_client::fog::FogRenderer,
    music: horizon_client::music::MusicTables,
    ambient: horizon_client::ambience::AmbientTables,
    screen_fx: horizon_client::screen_effects::ScreenEffects,
    nether_environment: Environment,
    end_environment: Environment,
    behaviours: Behaviours,
    names: Names,
    /// clouds.png: (width, height, filled cells).
    clouds: (u32, u32, Vec<bool>),
    /// misc/forcefield.png: (width, height, RGBA) for the world border wall.
    forcefield: (u32, u32, Vec<u8>),
    context: Arc<WorkerContext>,
    threads: usize,
    player_arm: Option<horizon_client::mobs::Part>,
    player_type: Option<Arc<horizon_client::mobs::MobType>>,
    /// Item definitions, baked item models and special renderers.
    item_renderer: Arc<horizon_client::item_render::ItemRenderer>,
    /// Enchantments, workstation recipes and brewing (game/stations.rs).
    stations: horizon_client::stations::StationData,
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
    /// A quick craft drag over inventory slots: (button, game slots so far).
    inv_drag: Option<(i32, Vec<usize>)>,
    /// `AbstractContainerScreen`'s quick craft and skip-release state.
    inv_clicks: horizon_client::container_screen::Clicks,
    /// The last inventory click, for the double click that gathers a stack.
    last_inv_click: Option<(std::time::Instant, usize, bool)>,
    /// Open container block entity, if any.
    container: Option<(i32, i32, i32)>,
    /// Open workstation menu (enchanting table, anvil, ...).
    station: Option<stations::Station>,
    /// Status effects and thrown potions (game/potions.rs).
    potions: potions::PotionState,
    scratch_slot: Option<ItemStack>,
    mining: Option<Mining>,
    destroy_delay: u32,
    use_delay: u32,
    creative: bool,
    /// The game mode; `creative` mirrors `mode == Creative`.
    mode: horizon_client::command::args::GameMode,
    keys: HashSet<KeyCode>,
    // Physical input is separate from remapped and toggled movement keys.
    physical_keys: HashSet<KeyCode>,
    left: bool,
    right: bool,
    cursor: (f32, f32),
    captured: bool,
    focused: bool,
    /// The keyboard's modifier state (`KeyEvent.modifiers`).
    modifiers: winit::keyboard::ModifiersState,
    /// The window's fullscreen request and reported state.
    fullscreen: horizon_client::window::FullscreenState,
    /// MouseHandler's accumulated look and wheel state.
    mouse: MouseLook,
    /// An agent moved the mouse (`mouse-move`) since the pointer was last
    /// grabbed or released: its motion turns the view like a grabbed pointer.
    agent_pointer: bool,
    /// `Options.smoothCamera` (cinematic camera), not persisted.
    smooth_camera: bool,
    /// Sneak, sprint, attack and use as ToggleKeyMappings.
    toggles: horizon_client::keymapping::ToggleKeys,
    /// A world screen with the pointer captured was open last frame.
    was_playing: bool,
    /// Minecraft.pause: a single-player screen that pauses the game is open.
    pause: bool,
    debug: bool,
    screen: Screen,
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    started: Instant,
    previous: Instant,
    /// FramerateLimitTracker.latestInputTime.
    last_input: Instant,
    frames: u64,
    fps: f32,
    fps_frames: u64,
    fps_at: Instant,
    ticks: u64,
    health_hud: hud_layout::HealthHud,
    /// What ends the game with an error; kept as it is, so main can tell a debug crash.
    error: Option<Box<dyn std::error::Error>>,
    message: Option<(String, Instant)>,
    save: Arc<SaveDir>,
    level: Level,
    session_lock: Option<horizon_client::save::SessionLock>,
    audio: Audio,
    music: horizon_client::music::MusicManager,
    ambience: horizon_client::ambience::Ambience,
    /// The sound engine was told the game paused (pauseAllExcept).
    sound_paused: bool,
    dropped: Vec<ItemEntity>,
    next_entity: u32,
    particles: horizon_client::particles::ParticleEngine,
    /// Sign text being edited (`SignEditScreen`).
    sign_edit: Option<sign_edit::SignEditor>,
    /// An open book, book and quill or lectern screen.
    book: Option<books::BookScreen>,
    /// Lecterns whose page-turn pulse is running (ticks left).
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    spawn_search: u32,
    mobs: Vec<Mob>,
    arrows: Vec<Arrow>,
    creatures: creatures::Creatures,
    sculk: sculk::SculkState,
    raiding: raids::Raiding,
    attack_ticks: u32,
    last_hurt_by_player: HashSet<u32>,
    /// Mobs whose death (`LivingEntity.die`) has been handled.
    died: HashSet<u32>,
    /// Client-ticked block-entity animation state (`game/block_entity_anims.rs`).
    be_anims: horizon_client::block_entity_anim::BlockEntityAnims,
    /// Item being used (eaten): hotbar slot and ticks used.
    using: Option<(usize, u32)>,
    /// Camera.oldFovModifier / fovModifier.
    /// `Camera` (eye height and field-of-view easing, view placement).
    view: horizon_client::camera::Camera,
    /// `FramerateLimiter` and its clock.
    limiter: horizon_client::window::FramerateLimiter,
    frame_clock: horizon_client::window::SystemClock,
    /// Hud.scopeScale (spyglass overlay size).
    scope_scale: f32,
    /// Duration of the last frame in ticks (GUI easing).
    frame_ticks: f32,
    title: Option<Title>,
    /// Armor slots: head, chest, legs, feet.
    armor: [Option<ItemStack>; 4],
    /// The off hand (Inventory slot 40, saved as slot 150; menu slot 504 here).
    offhand: Option<ItemStack>,
    /// The shield's item cooldown after an axe disabled it (ticks).
    shield_cooldown: u32,
    /// The source of a command run through /execute (the player otherwise).
    command_source: Option<commands::execute::Source>,
    titles: titles::Titles,
    /// `Hud.toolHighlightTimer`: the selected item name.
    tool_highlight: horizon_client::hud_overlays::ToolHighlight,
    applied_modifiers: Vec<(String, String)>,
    experience: Experience,
    orbs: Vec<ExperienceOrb>,
    xp_delay: u32,
    last_level_up: u64,
    /// The arm swing (`LivingEntity.SwingState`) and how many swings started.
    swing: horizon_client::first_person::SwingState,
    swings_started: u32,
    /// The first-person equip heights and the items the hands show (with
    /// the slot each came from).
    heights: horizon_client::first_person::HandHeights,
    /// The hands' lag behind the view (`LocalPlayer.xBob`, `yBob`); a new
    /// LocalPlayer (joining, respawning, changing dimension) starts at 0.
    sway: horizon_client::first_person::HandSway,
    visible_hands: [(usize, Option<ItemStack>); 2],
    /// `Minecraft`'s attack handling, attack clicks since the last tick and
    /// the attack strength tickers the hand reads.
    clicks: horizon_client::first_person::Clicks,
    attack_clicks: u32,
    hand_strength: horizon_client::attack_strength::AttackStrength,
    settings: Settings,
    /// Where the options screen returns: true = title, false = pause menu.
    options_from_title: bool,
    /// The live title backdrop (game/backdrop.rs).
    backdrop: backdrop::Backdrop,
    /// When the pause menu opened (Horizon style's entrance).
    pause_since: Option<Instant>,
    dragging: bool,
    /// `ClientAvatarState`: walked distance and view bob.
    avatar: horizon_client::camera::AvatarBob,
    flash: u32,
    /// Sleeping in a bed: (bed head position, ticks asleep).
    sleeping: Option<([i32; 3], u32)>,
    dim: Dim,
    world_root: PathBuf,
    /// A player carried through a portal, re-placed once the destination loads.
    travelling: Option<(PlayerMotion, Vec3)>,
    portal_ticks: u32,
    portal_cooldown: u32,
    /// The pending travel goes through an End portal (EndPortalBlock destinations).
    end_travel: bool,
    /// The next enter_dimension call comes from an End portal.
    end_portal_travel: bool,
    end: end::EndState,
    block_ticks: Ticks,
    tick_chunks: HashSet<(i32, i32)>,
    level_events: Vec<LevelEvent>,
    falling: Vec<FallingBlock>,
    paintings: Vec<horizon_client::entities::Painting>,
    painting_variants: paintings::PaintingVariants,
    item_frames: Vec<horizon_client::entities::ItemFrame>,
    frame_model: frames::FrameModel,
    glow_frame_model: frames::FrameModel,
    /// The `map=true` frame variants.
    map_frame_model: frames::FrameModel,
    glow_map_frame_model: frames::FrameModel,
    armor_stands: Vec<horizon_client::entities::ArmorStand>,
    /// Entities of types Horizon does not simulate.
    others: Vec<commands::OtherEntity>,
    armor_stand_assets: armor_stands::ArmorStandAssets,
    boats: Vec<boats::Boat>,
    boat_models: boats::BoatModels,
    carts: Vec<minecarts::Minecart>,
    /// Songs playing from jukeboxes (tracked sound handles).
    jukebox_sounds: HashMap<(i32, i32, i32), u64>,
    bolts: Vec<lightning::Bolt>,
    maps: maps::Maps,
    /// The ender dragon fight (game/dragon.rs).
    fight: dragon::Fight,
    dragon: Option<dragon::Dragon>,
    crystals: Vec<dragon::Crystal>,
    dragon_fireballs: Vec<dragon::DragonFireball>,
    breath_clouds: Vec<dragon::Breath>,
    dragon_model: Option<horizon_client::mobs::Part>,
    /// Map textures waiting for the next HUD batch to upload them.
    map_patches: Vec<horizon_client::font::Patch>,
    /// Player skin downloads, and which skin each atlas skin slot holds
    /// (least recently shown first).
    skin_downloads: horizon_client::skins::Downloader,
    skin_slots: Vec<(String, usize)>,
    cart_models: minecarts::CartModels,
    /// The boat the player rides.
    riding: Option<u32>,
    /// The ender inventory (27 slots) and the ender chest showing it.
    ender_items: Vec<Option<ItemStack>>,
    ender_open: Option<(i32, i32, i32)>,
    /// Block entity data of broken blocks waiting for their dropped item.
    pending: spill::Pending,
    /// Item entities spawned and block entities removed while the block
    /// behaviour ran (spilled contents, popped resources).
    spill: spill::Spill,
    /// The recipe book of the open screen (game/recipe_book.rs).
    recipe_ui: recipe_book::RecipeUi,
    /// The horse inventory (game/mount_menu.rs).
    mount_menu: mount_menu::MountMenu,
    /// The player's fishing hook.
    hook: Option<fishing::Hook>,
    tridents: Vec<trident::ThrownTrident>,
    rockets: Vec<fireworks::Rocket>,
    creaking: creaking::CreakingState,
    trident_model: Option<horizon_client::mobs::Part>,
    tnts: Vec<tnt::PrimedTnt>,
    hoppers: HashMap<(i32, i32, i32), machines::HopperState>,
    creative_screen: Creative,
    control: Option<horizon_client::control::Server>,
    frame_stats: horizon_client::control::FrameStats,
    /// Control replies waiting for a rendered frame or a benchmark deadline.
    pending_screenshot: Option<std::sync::mpsc::Sender<String>>,
    pending_bench: Option<(Instant, std::sync::mpsc::Sender<String>)>,
    /// Recent frame times for the performance overlay.
    frame_graph: horizon_client::perf_overlay::FrameGraph,
    /// Meshed sections waiting for an upload budget (Horizon settings).
    pending_uploads: Vec<((i32, i32, i32), horizon_client::mesher::SectionMesh)>,
    /// This frame's view frustum, for culling entities and block entities.
    view_frustum: Option<horizon_client::gfx::Frustum>,
    /// A scripted flight from the control socket (`fly`).
    flight: Option<control::Flight>,
    /// The running agent action from the control socket (game/agent.rs).
    agent: agent::Agent,
    /// The overworld's last `sky_darken` and its inputs (time, rain, thunder).
    sky_darken_memo: std::cell::Cell<Option<((i64, u32, u32), i32)>>,
    persist: persist::EntityMemory,
    /// Connected to a server: the world comes from the network (game/remote.rs).
    remote: Option<remote::Remote>,
    /// This world is open to the network (game/lan.rs).
    lan: Option<lan::LanHost>,
    world_options: Option<lan_screen::WorldOptions>,
    /// KeyboardHandler's debug key state (game/debug_keys.rs), the physical
    /// keys held, and the debug renderers F3+B and F3+G switch on.
    debug_keys: horizon_client::debug_keys::DebugKeys,
    show_hitboxes: bool,
    show_chunk_borders: bool,
    /// PauseScreen.showPauseMenu (false after F3+Escape).
    pause_menu: bool,
    /// MultiPlayerGameMode.previousLocalPlayerMode (F3+N goes back to it).
    previous_mode: Option<horizon_client::command::args::GameMode>,
    /// The system clipboard (`KeyboardHandler.clipboardManager`).
    clipboard: horizon_client::clipboard::SystemClipboard,
    /// Select All in the append-only text fields.
    text_selection: horizon_client::clipboard::AppendSelection,
    /// The world was saved for the application ending (`save_for_exit`).
    saved_for_exit: bool,
    /// The block a guest is acting on (game/lan_actions.rs).
    forced_hit: Option<Hit>,
    /// Level events of this tick for the guests, and the guest acting now.
    lan_events: Vec<(i32, [i32; 3], i32, Option<u32>)>,
    lan_acting: Option<u32>,
    /// Command feedback to forward to a guest.
    chat_capture: Option<Vec<horizon_client::text::Text>>,
    /// State of `/spectate`, `/posteffect`, `/debug`, `/waypoint` and friends.
    cmds: commands::extra::CommandExtras,
    /// Allow Cheats: whether guests may be given commands.
    cheats: bool,
    ui: chat::UiState,
    combat: combat::CombatTracker,
    mount: mounts::MountState,
    stats: stats::Stats,
    score: scoreboard::ScoreState,
    fns: functions::FnState,
    bosses: bossbars::BossState,
    exec: commands::execute::ExecState,
    rand_sequences: horizon_client::random_sequences::RandomSequences,
    border: horizon_client::worldborder::WorldBorder,
    /// `WorldBorderRenderer` and its render state.
    border_renderer: horizon_client::border_render::BorderRenderer,
    border_state: horizon_client::border_render::BorderRenderState,
    /// `Hud.tabList`: the player list's header, footer and heart states.
    tab_list: horizon_client::tab_list::TabList,
    subtitles: subtitle_hud::SubtitleState,
    adv: advancements::AdvState,
    stats_ui: stats_screen::StatsUi,
    adv_ui: advancements_screen::AdvUi,
    camera_type: perspective::CameraType,
    fangs: fangs::FangState,
    /// The local player as a model (third person, the inventory).
    player_model: Option<Mob>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Dim {
    Overworld,
    Nether,
    End,
}
impl Dim {
    fn id(self) -> &'static str {
        match self {
            Self::Overworld => "minecraft:overworld",
            Self::Nether => "minecraft:the_nether",
            Self::End => "minecraft:the_end",
        }
    }
    fn from_id(id: &str) -> Self {
        match id {
            "minecraft:the_nether" => Self::Nether,
            "minecraft:the_end" => Self::End,
            _ => Self::Overworld,
        }
    }
    /// `Level.getMaxY`: the dimension type's minimum y plus height, less one.
    fn max_y(self) -> i32 {
        match self {
            Self::Overworld => MIN_Y + HEIGHT - 1,
            Self::Nether | Self::End => 255,
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
        Some([
            p.position.x,
            p.position.y + f64::from(p.dimensions.eye_height),
            p.position.z,
        ])
    }
    fn reach(&self) -> f64 {
        if self.creative { 5.0 } else { 4.5 }
    }
    /// A block's outline boxes in world space (`getShape` plus the random offset).
    fn outline_boxes(&self, pos: [i32; 3]) -> Vec<[f64; 6]> {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let f = self.res.facts.get(state);
        let o = if f.outline_offset {
            self.res
                .context
                .tables
                .offset(state, pos[0], pos[2])
                .unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        let base = [
            f64::from(pos[0]) + o[0],
            f64::from(pos[1]) + o[1],
            f64::from(pos[2]) + o[2],
        ];
        f.outline
            .iter()
            .map(|b| std::array::from_fn(|i| base[i % 3] + b[i]))
            .collect()
    }
    /// Voxel traversal against non-air, non-fluid blocks, clipped against
    /// each block's outline shape (`BlockGetter.clip` with `ClipContext.Block.OUTLINE`).
    fn pick(&self) -> Option<Hit> {
        // A guest of a hosted world acts on the block their client targeted.
        if let Some(hit) = &self.forced_hit {
            return Some(Hit {
                pos: hit.pos,
                face: hit.face,
                point: hit.point,
            });
        }
        let origin = self.eye_position()?;
        self.clip_ray(origin, view_vector(self.yaw, self.pitch), self.reach())
    }
    /// The first block outline hit within `reach` along the unit `dir`.
    fn clip_ray(&self, origin: [f64; 3], dir: [f64; 3], reach: f64) -> Option<Hit> {
        let mut pos = origin.map(|v| v.floor() as i32);
        let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
        let delta = dir.map(|d| {
            if d == 0.0 {
                f64::INFINITY
            } else {
                (1.0 / d).abs()
            }
        });
        let mut t_max = [0.0; 3];
        for i in 0..3 {
            let boundary = if dir[i] > 0.0 {
                f64::from(pos[i]) + 1.0
            } else {
                f64::from(pos[i])
            };
            t_max[i] = if dir[i] == 0.0 {
                f64::INFINITY
            } else {
                (boundary - origin[i]) / dir[i]
            };
        }
        let mut face = [0; 3];
        let mut t = 0.0;
        while t <= reach {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let f = self.res.facts.get(state);
            if !self.res.facts.is_air(state)
                && !(f.fluid != Fluid::None
                    && !f.waterlogged
                    && &*f.name != "minecraft:kelp"
                    && f.render_shape != horizon_client::blocks::RenderShape::Model)
                && !matches!(
                    &*f.name,
                    "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
                )
            {
                // The nearest entry into one of the outline's boxes (slab method).
                let mut best: Option<(f64, [i32; 3])> = None;
                for b in self.outline_boxes(pos) {
                    let (mut t0, mut t1, mut axis_in, mut sign) = (0.0f64, f64::INFINITY, None, 0);
                    let mut miss = false;
                    for a in 0..3 {
                        if dir[a] == 0.0 {
                            miss |= origin[a] < b[a] || origin[a] > b[a + 3];
                            continue;
                        }
                        let (lo, hi) =
                            ((b[a] - origin[a]) / dir[a], (b[a + 3] - origin[a]) / dir[a]);
                        let (near, far) = if lo < hi { (lo, hi) } else { (hi, lo) };
                        if near > t0 {
                            t0 = near;
                            axis_in = Some(a);
                            sign = if dir[a] > 0.0 { -1 } else { 1 };
                        }
                        t1 = t1.min(far);
                    }
                    if miss || t0 > t1 || t0 > reach {
                        continue;
                    }
                    let mut normal = face;
                    if let Some(a) = axis_in {
                        normal = [0; 3];
                        normal[a] = sign;
                    }
                    if best.is_none_or(|(bt, _)| t0 < bt) {
                        best = Some((t0, normal));
                    }
                }
                if let Some((t, face)) = best {
                    return Some(Hit {
                        pos,
                        face,
                        point: [
                            origin[0] + dir[0] * t,
                            origin[1] + dir[1] * t,
                            origin[2] + dir[2] * t,
                        ],
                    });
                }
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
        let order: Vec<usize> = std::iter::once(self.selected)
            .chain((0..36).filter(|&i| i != self.selected))
            .collect();
        for &i in &order {
            if let Some(slot) = &mut self.inventory[i]
                && slot.same_item_same_components(&stack)
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
                self.inventory[i] = Some(ItemStack {
                    count: moved,
                    ..stack.clone()
                });
                stack.count -= moved;
                if stack.count == 0 {
                    return None;
                }
            }
        }
        Some(stack)
    }
    fn particle_sprite(&self, state: BlockStateId) -> Option<(String, u32)> {
        let models = self
            .res
            .assets
            .model_catalog
            .collect(state, horizon_core::position::BlockPos::new(0, 0, 0))
            .ok()?;
        let face = models
            .iter()
            .flat_map(|(_, m)| m.faces.iter())
            .find(|f| f.quad.facing == horizon_assets::model::Direction::Up)
            .or_else(|| models.iter().flat_map(|(_, m)| m.faces.iter()).next())?;
        let name = self.res.layer_names.get(&face.layer)?.clone();
        let tint = if face.quad.tint_index >= 0 {
            let n = &self.res.facts.get(state).name;
            if n.contains("leaves") || n.contains("vine") {
                0x48B518
            } else {
                0x91BD59
            }
        } else {
            0xFFFFFF
        };
        Some((name, tint))
    }
    fn particle_shape(&self, pos: [i32; 3], state: BlockStateId) -> Vec<[f64; 6]> {
        let f = self.res.facts.get(state);
        let offset = if f.outline_offset {
            self.res
                .context
                .tables
                .offset(state, pos[0], pos[2])
                .unwrap_or([0.0; 3])
        } else {
            [0.0; 3]
        };
        f.outline
            .iter()
            .map(|b| std::array::from_fn(|i| b[i] + offset[i % 3]))
            .collect()
    }
    /// ClientLevel.addDestroyBlockEffect: a burst per outline box.
    fn destroy_particles(&mut self, pos: [i32; 3], state: BlockStateId) {
        let Some((sprite, tint)) = self.particle_sprite(state) else {
            return;
        };
        let boxes = self.particle_shape(pos, state);
        horizon_client::particles::destroy_positions(pos, &boxes, |p, v| {
            let particle = Particle::terrain(self.particles.random(), p, v, sprite.clone(), tint);
            self.particles.push(particle);
        });
    }
    /// ClientLevel.addBreakingParticles: one crumb on the face being mined.
    fn crack_particle(&mut self, pos: [i32; 3], face: [i32; 3], state: BlockStateId) {
        let Some((sprite, tint)) = self.particle_sprite(state) else {
            return;
        };
        let boxes = self.particle_shape(pos, state);
        // Keep client effects separate from the authoritative world RNG.
        let r = self.particles.random();
        let Some(p) = horizon_client::particles::breaking_position(r, pos, face, &boxes) else {
            return;
        };
        let mut particle = Particle::terrain(r, p, [0.0; 3], sprite, tint);
        particle.set_power(0.2);
        particle.scale(0.6);
        self.particles.push(particle);
    }
    fn spawn_item(
        &mut self,
        stack: ItemStack,
        position: [f64; 3],
        velocity: [f64; 3],
        pickup_delay: u32,
    ) {
        self.next_entity += 1;
        let p = Vec3::new(position[0], position[1], position[2]);
        // ItemEntity.bobOffs comes from the entity's own random, not the level's.
        let bob = self.particles.random().next_float() * std::f32::consts::PI * 2.0;
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
    /// `Block.getDrops`: the block's loot table, drawing from its server random
    /// sequence (`LootContext.Builder.create`), else the level random.
    fn block_loot_at(
        &mut self,
        pos: [i32; 3],
        state: BlockStateId,
        held: Option<&ItemStack>,
    ) -> Vec<ItemStack> {
        let registry = self.res.assets.registry;
        let dynamic = self.pending.dynamic(pos);
        match self.res.items.block_loot_sequence(registry, state) {
            Some(sequence) => {
                let random = self.rand_sequences.get(&sequence, self.level.seed);
                self.res
                    .items
                    .block_drops_with(registry, state, held, &dynamic, random)
            }
            None => self.res.items.block_drops_with(
                registry,
                state,
                held,
                &dynamic,
                &mut self.world.random,
            ),
        }
    }
    /// Block.popResource
    fn pop_resource(&mut self, pos: [i32; 3], mut stack: ItemStack) {
        self.pending.attach(pos, &mut stack);
        // Block.popResource: Mth.nextDouble offsets from the level random; the
        // motion is `new ItemEntity(..)`'s own, from the item's random.
        let p = spill::pop_position(&mut self.world.random, pos);
        let v = dispenser::new_item_motion();
        self.spawn_item(stack, p, v, 10);
    }
    /// Player.drop: thrown from the eyes along the view direction.
    fn drop_held(&mut self, all: bool) {
        if self.eye_position().is_none() {
            return;
        }
        let Some(stack) = self.inventory[self.selected].as_mut() else {
            return;
        };
        let count = if all { stack.count } else { 1 };
        let thrown = ItemStack {
            count,
            ..stack.clone()
        };
        stack.count -= count;
        if stack.count == 0 {
            self.inventory[self.selected] = None;
        }
        self.player_drop(thrown);
    }
    /// `Player.drop`: the statistics and a thrown item entity in front of the player.
    fn player_drop(&mut self, thrown: ItemStack) {
        let Some(eye) = self.eye_position() else {
            return;
        };
        // Player.drop: the drop statistics.
        self.stats.add("dropped", &thrown.item, thrown.count as i32);
        self.stats.custom("drop", 1);
        let (yaw, pitch) = (
            f64::from(self.yaw).to_radians(),
            f64::from(self.pitch).to_radians(),
        );
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
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        items.retain_mut(|item| {
            !ticking(
                item.position.x.floor() as i32,
                item.position.z.floor() as i32,
            ) || item.tick(&self.world, &self.res.facts, self.collision)
        });
        let Some(p) = self.player.as_ref() else {
            self.dropped = items;
            return;
        };
        if p.health.dead_or_dying() || self.spectator() {
            self.dropped = items;
            return;
        }
        let w = f64::from(p.dimensions.width) / 2.0;
        let (min, max) = (
            [
                p.position.x - w - 1.0,
                p.position.y - 0.5,
                p.position.z - w - 1.0,
            ],
            [
                p.position.x + w + 1.0,
                p.position.y + f64::from(p.dimensions.height) + 0.5,
                p.position.z + w + 1.0,
            ],
        );
        let feet = [p.position.x, p.position.y, p.position.z];
        let mut kept = Vec::with_capacity(items.len());
        for mut item in items {
            let b = item.bounds();
            let touching = b.min_position().x < max[0]
                && b.max_position().x > min[0]
                && b.min_position().y < max[1]
                && b.max_position().y > min[1]
                && b.min_position().z < max[2]
                && b.max_position().z > min[2];
            if touching && item.pickup_delay == 0 {
                let before = item.stack.count;
                let kind = item.stack.item.clone();
                match self.give(item.stack.clone()) {
                    None => {
                        self.stats.add("picked_up", &kind, before as i32);
                        let pitch = ((self.world.random.next_float()
                            - self.world.random.next_float())
                            * 0.7
                            + 1.0)
                            * 2.0;
                        self.audio
                            .play("minecraft:entity.item.pickup", Some(feet), 0.2, pitch);
                        continue;
                    }
                    Some(rest) => {
                        if rest.count < before {
                            self.stats
                                .add("picked_up", &kind, (before - rest.count) as i32);
                            self.audio
                                .play("minecraft:entity.item.pickup", Some(feet), 0.2, 1.0);
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
    fn with_level<R>(
        &mut self,
        f: impl FnOnce(&Behaviours, &mut GameLevel<'_>, &mut Ticks) -> R,
    ) -> R {
        let darken = self.sky_darken();
        let (rain, thunder) = self.world.weather.levels(1.0);
        // Spectators ignore block triggers (Entity.isIgnoringBlockTriggers).
        let spectator = self.spectator();
        let player_box = self.player.as_ref().filter(|_| !spectator).map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            (
                [p.position.x - w, p.position.y, p.position.z - w],
                [
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ],
            )
        });
        let cart_boxes = self
            .carts
            .iter()
            .map(|c| {
                let p = c.position;
                (
                    [p.x - 0.49, p.y, p.z - 0.49],
                    [p.x + 0.49, p.y + 0.7, p.z + 0.49],
                )
            })
            .collect();
        let dimension = match self.dim {
            Dim::Overworld => LevelDimension::Overworld,
            Dim::Nether => LevelDimension::Nether,
            Dim::End => LevelDimension::End,
        };
        let rules = self.level_rules();
        let mut level = GameLevel {
            cart_boxes,
            world: &mut self.world,
            facts: &self.res.facts,
            events: &mut self.level_events,
            dimension,
            darken,
            raining: rain > 0.2,
            thundering: thunder > 0.9,
            player_box,
            biomes: &self.res.biomes,
            seed: self.level.seed,
            behaviours: &self.res.behaviours,
            items: &self.res.items,
            registry: self.res.assets.registry,
            rules,
            rand_sequences: &mut self.rand_sequences,
            pending: &mut self.pending,
            spill: &mut self.spill,
        };
        let result = f(&self.res.behaviours, &mut level, &mut self.block_ticks);
        self.flush_spill();
        result
    }
    /// Level.setBlock(pos, state, UPDATE_ALL) with neighbour/shape updates.
    fn level_set(&mut self, pos: [i32; 3], state: BlockStateId) -> Result<()> {
        self.with_level(|b, level, ticks| {
            b.set_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                state,
                level_flags::UPDATE_ALL,
            );
        });
        self.process_level_events()
    }
    /// Track which chunks are ticking (Ticks needs chunk load/unload).
    fn sync_tick_chunks(&mut self) {
        let loaded: HashSet<(i32, i32)> = self.world.chunks.keys().copied().collect();
        for k in loaded.difference(&self.tick_chunks.clone()) {
            self.register_chunk_ticks(*k);
        }
        for k in self.tick_chunks.difference(&loaded) {
            self.block_ticks
                .remove_chunk(horizon_core::position::ChunkPos::new(k.0, k.1));
        }
        self.tick_chunks = loaded;
    }
    /// ServerLevel.tick block phases: scheduled ticks, random ticks within the
    /// simulation distance, block events and moving pistons.
    fn tick_blocks(&mut self) -> Result<()> {
        self.sync_tick_chunks();
        let Some(p) = self.player.as_ref() else {
            return Ok(());
        };
        let (pcx, pcz) = (
            (p.position.x.floor() as i32) >> 4,
            (p.position.z.floor() as i32) >> 4,
        );
        let simulation = self.settings.render_distance.min(8);
        let mut ticking = Vec::new();
        let facts = &self.res.facts;
        for (&(x, z), chunk) in &self.world.chunks {
            if (x - pcx).abs() > simulation || (z - pcz).abs() > simulation || !chunk.lit {
                continue;
            }
            // LevelChunkSection.isRandomlyTicking: a randomly ticking block or
            // fluid state (lava reports both through LiquidBlock). The palette
            // stands in for vanilla's exact counts.
            let sections: Vec<i32> = chunk
                .sections
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    s.blocks
                        .maybe_has(|id| facts.get(BlockStateId(id)).random_ticks)
                })
                .map(|(i, _)| MIN_Y + i as i32 * 16)
                .collect();
            ticking.push((x, z, sections));
        }
        // Entity.checkInsideBlocks for the plates, tripwires and detector rails
        // under the player and the minecarts.
        let mut inside = Vec::new();
        let boxes = self
            .player
            .iter()
            .filter(|_| !self.spectator())
            .map(|p| {
                let w = f64::from(p.dimensions.width) / 2.0;
                let h = f64::from(p.dimensions.height);
                (
                    [p.position.x - w, p.position.y, p.position.z - w],
                    [p.position.x + w, p.position.y + h, p.position.z + w],
                )
            })
            .chain(self.carts.iter().map(|c| {
                let p = c.position;
                (
                    [p.x - 0.49, p.y, p.z - 0.49],
                    [p.x + 0.49, p.y + 0.7, p.z + 0.49],
                )
            }));
        for (lo, hi) in boxes {
            for x in (lo[0] + 1e-7).floor() as i32..=(hi[0] - 1e-7).floor() as i32 {
                for y in (lo[1] + 1e-7).floor() as i32..=(hi[1] - 1e-7).floor() as i32 {
                    for z in (lo[2] + 1e-7).floor() as i32..=(hi[2] - 1e-7).floor() as i32 {
                        inside.push(BlockPos::new(x, y, z));
                    }
                }
            }
        }
        self.with_level(|b, level, ticks| {
            for pos in inside {
                b.entity_inside(level, ticks, pos);
            }
        });
        self.process_level_events()?;
        self.tick_scheduled()?;
        self.with_level(|b, level, ticks| {
            for (x, z, sections) in ticking {
                b.tick_chunk_random(
                    level,
                    ticks,
                    horizon_core::position::ChunkPos::new(x, z),
                    sections,
                );
            }
            b.run_block_events(level, ticks);
            b.tick_block_entities(level, ticks);
        });
        self.process_level_events()
    }
    /// `ServerLevel.tick`'s scheduled block then fluid ticks, one at a time:
    /// the side effects of each (a shrieker summoning a warden, drops, game
    /// events) happen before the next tick runs, as they do inline in vanilla.
    pub(super) fn tick_scheduled(&mut self) -> Result<()> {
        for fluids in [false, true] {
            if !self.with_level(|b, level, ticks| b.scheduled_begin(level, ticks, fluids)) {
                continue;
            }
            loop {
                let more =
                    self.with_level(|b, level, ticks| b.scheduled_step(level, ticks, fluids));
                if !self.level_events.is_empty() {
                    self.process_level_events()?;
                }
                if !more {
                    break;
                }
            }
            self.with_level(|b, _, ticks| b.scheduled_finish(ticks, fluids));
        }
        Ok(())
    }
    /// Side effects reported by the behaviour library, in vanilla order.
    fn process_level_events(&mut self) -> Result<()> {
        let events = std::mem::take(&mut self.level_events);
        self.lan_record_events(&events);
        dispense_fixture::record_events(&events);
        for event in events {
            match event {
                LevelEvent::LevelEvent {
                    id: 2001,
                    pos,
                    data,
                } => {
                    // LevelEventHandler: destroy particles and the block's
                    // break sound (none for air).
                    let state = BlockStateId(data as u32);
                    let p = [pos.x, pos.y, pos.z];
                    self.destroy_particles(p, state);
                    if self.res.facts.is_air(state) {
                        continue;
                    }
                    let sounds = self.res.facts.get(state).sounds.clone();
                    let c = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.audio.play(
                        &sounds.break_sound,
                        Some(c),
                        (sounds.volume + 1.0) / 2.0,
                        sounds.pitch * 0.8,
                    );
                }
                LevelEvent::LevelEvent {
                    id: 3006,
                    pos,
                    data,
                } => self.sculk_charge_event(pos, data),
                LevelEvent::LevelEvent { id: 3007, pos, .. } => self.shriek_event(pos),
                LevelEvent::LevelEvent {
                    id: id @ 3002..=3005,
                    pos,
                    data,
                } => self.block_particle_event(id, pos, data),
                LevelEvent::LevelEvent {
                    id: 1500,
                    pos,
                    data,
                } => self.composter_fill_event(pos, data == 1),
                LevelEvent::LevelEvent { id, pos, .. } => {
                    let c = Some([
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ]);
                    let sound = match id {
                        1501 => Some(("minecraft:block.lava.extinguish", 0.5)),
                        1009 => Some(("minecraft:block.fire.extinguish", 0.5)),
                        1502 => Some(("minecraft:block.redstone_torch.burnout", 0.5)),
                        1030 => Some(("minecraft:block.anvil.use", 1.0)),
                        1031 => Some(("minecraft:block.anvil.land", 0.3)),
                        1505 => Some(("minecraft:item.bone_meal.use", 1.0)),
                        _ => None,
                    };
                    if let Some((name, volume)) = sound {
                        self.audio.play(name, c, volume, 1.0);
                    }
                }
                LevelEvent::Sound {
                    pos,
                    sound,
                    volume,
                    pitch,
                } => {
                    let c = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    let name = if sound.contains(':') {
                        sound.to_string()
                    } else {
                        format!("minecraft:{sound}")
                    };
                    self.audio.play(&name, Some(c), volume, pitch);
                }
                // Handled as the behaviour emits it (GameLevel::emit).
                LevelEvent::DropResources { pos, state } => {
                    self.with_level(|_, level, _| level.drop_resources_now(pos, state));
                }
                LevelEvent::FallingBlock { pos, state } => {
                    let p = Vec3::new(
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y),
                        f64::from(pos.z) + 0.5,
                    );
                    self.falling.push(FallingBlock {
                        state,
                        position: p,
                        previous: p,
                        velocity: Vec3::ZERO,
                        age: 0,
                        no_gravity: false,
                        drop_item: true,
                        fall_distance: 0.0,
                        saved: None,
                    });
                }
                LevelEvent::Player { pos, effect, .. } => match effect {
                    PlayerEffect::OpenMenu => self.open_menu([pos.x, pos.y, pos.z]),
                    PlayerEffect::Sleep => self.try_sleep([pos.x, pos.y, pos.z]),
                    PlayerEffect::GiveItem(item) => {
                        if let Some(rest) = self.give(ItemStack {
                            item,
                            count: 1,
                            damage: 0,
                            extra: None,
                        }) {
                            self.pop_resource([pos.x, pos.y, pos.z], rest);
                        }
                    }
                    // Popped as the behaviour emits it (GameLevel::emit).
                    PlayerEffect::DropItem(_) => {}
                    PlayerEffect::EatCake => {
                        if let Some(p) = self.player.as_mut() {
                            let food = &mut p.health.food;
                            food.food = (food.food + 2).min(20);
                            food.saturation = (food.saturation + 0.4).min(food.food as f32);
                        }
                    }
                    PlayerEffect::InteractLoot(_) | PlayerEffect::RespawnAnchor => {}
                },
                LevelEvent::PrimeTnt { pos, rot } => self.add_primed_tnt(pos, rot, 80),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "dispense",
                } => self.dispense_from(pos, state)?,
                LevelEvent::BlockEvent {
                    pos, kind: "hatch", ..
                } => {
                    // FrogspawnBlock.hatchFrogspawn: 2-5 tadpoles.
                    let at = [
                        f64::from(pos.x) + 0.5,
                        f64::from(pos.y) + 0.5,
                        f64::from(pos.z) + 0.5,
                    ];
                    self.audio
                        .play("minecraft:block.frogspawn.hatch", Some(at), 1.0, 1.0);
                    let count = 2 + self.world.random.next_int_bound(4).unwrap_or(0);
                    for _ in 0..count {
                        // getRandomTadpolePositionOffset: 0.2..0.8 inside the block.
                        let r = &mut self.world.random;
                        let off = |r: &mut horizon_core::random::LegacyRandom| {
                            f64::from(r.next_float()) * 0.6 + 0.2
                        };
                        let (dx, dz) = (off(r), off(r));
                        let spawn = [
                            f64::from(pos.x) + dx,
                            f64::from(pos.y) - 0.5,
                            f64::from(pos.z) + dz,
                        ];
                        let _ = self.spawn_mob("minecraft:tadpole", spawn, true);
                    }
                }
                LevelEvent::BlockEvent {
                    pos,
                    kind: "hatch_sniffer",
                    ..
                } => self.hatch_sniffer([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos,
                    kind: "portal_spawn",
                    ..
                } => self.portal_spawn([pos.x, pos.y, pos.z]),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "spawn_ghastling",
                } => self.spawn_ghastling([pos.x, pos.y, pos.z], state),
                LevelEvent::GameEvent { pos, event, state } => {
                    self.sculk_level_event(pos, event, state)
                }
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "shrieker_respond",
                } => self.shrieker_block_event(pos, state),
                LevelEvent::BlockEntityEvent { pos, a, b, .. } => {
                    self.block_entity_event([pos.x, pos.y, pos.z], a, b)
                }
                LevelEvent::GrowTree { .. }
                | LevelEvent::PlaceFeature { .. }
                | LevelEvent::BlockEvent { .. } => {}
            }
        }
        Ok(())
    }
    /// Menus opened by PlayerEffect::OpenMenu.
    fn open_menu(&mut self, pos: [i32; 3]) {
        let name = self
            .res
            .facts
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let key = (pos[0], pos[1], pos[2]);
        if &*name == "minecraft:lectern" {
            self.open_lectern(pos);
            return;
        }
        if &*name == "minecraft:crafting_table" {
            self.craft_size = 3;
        } else if &*name == "minecraft:ender_chest" {
            self.open_ender_chest(key);
            self.container_opener_event(key, true);
            let at = [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ];
            let pitch = self.world.random.next_float() * 0.1 + 0.9;
            self.audio
                .play("minecraft:block.ender_chest.open", Some(at), 0.5, pitch);
        } else if &*name == "minecraft:beacon" && self.open_station(pos, &name) {
            self.world.block_entities.entry(key).or_insert_with(|| {
                BlockEntity::for_block(&name).unwrap_or(BlockEntity::Chest(Vec::new()))
            });
        } else if storage::is_shulker_box(&name) && !self.shulker_can_open(pos) {
            // ShulkerBoxBlock.canOpen: the lid has no room; nothing opens.
        } else if let Some(entity) = BlockEntity::for_block(&name) {
            self.world.block_entities.entry(key).or_insert(entity);
            self.container = Some(key);
            self.container_opener_event(key, true);
            self.post_block_game_event(
                "minecraft:container_open",
                [key.0, key.1, key.2],
                Some(horizon_client::mobs::Who::Player),
                None,
            );
            let stat = match &*name {
                "minecraft:chest" => "open_chest",
                "minecraft:trapped_chest" => "trigger_trapped_chest",
                "minecraft:barrel" => "open_barrel",
                "minecraft:hopper" => "inspect_hopper",
                "minecraft:dropper" => "inspect_dropper",
                "minecraft:dispenser" => "inspect_dispenser",
                n if storage::is_shulker_box(n) => "open_shulker_box",
                _ => "",
            };
            if !stat.is_empty() {
                self.stats.custom(stat, 1);
            }
            // Opening a piglin-guarded container angers the piglins who see it.
            if self.guarded_by_piglins(&name) {
                self.anger_piglins(true);
            }
            let sound = if storage::is_shulker_box(&name) {
                Some("minecraft:block.shulker_box.open")
            } else if name.contains("chest") {
                Some("minecraft:block.chest.open")
            } else if &*name == "minecraft:barrel" {
                Some("minecraft:block.barrel.open")
            } else {
                None
            };
            if let Some(sound) = sound {
                self.audio.play(
                    sound,
                    Some([
                        f64::from(key.0) + 0.5,
                        f64::from(key.1) + 0.5,
                        f64::from(key.2) + 0.5,
                    ]),
                    0.5,
                    1.0,
                );
            }
        } else if self.open_station(pos, &name) {
            // Workstation menus: game/stations.rs.
        } else {
            self.message = Some((
                format!(
                    "{} is not available yet",
                    name.trim_start_matches("minecraft:").replace('_', " ")
                ),
                Instant::now(),
            ));
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
        let mut dropped = Vec::new();
        let entity_drops = self.ui.rules.bool("entity_drops");
        falling.retain_mut(|f| {
            f.previous = f.position;
            f.age = f.age.wrapping_add(1);
            if !f.no_gravity {
                f.velocity.y -= 0.04;
            }
            let next = Vec3::new(
                f.position.x + f.velocity.x,
                f.position.y + f.velocity.y,
                f.position.z + f.velocity.z,
            );
            let below = self.world.block(
                next.x.floor() as i32,
                next.y.floor() as i32,
                next.z.floor() as i32,
            );
            let below_facts = self.res.facts.get(below);
            // PowderSnowBlock.getCollisionShape for a FallingBlockEntity: the
            // whole block, or 0.9 high after a fall of more than 2.5 blocks,
            // where the block it rests in is the snow itself.
            let powder_snow = &*below_facts.name == "minecraft:powder_snow";
            let sunk = powder_snow
                && f.fall_distance > 2.5
                && next.y - next.y.floor() < f64::from(0.9_f32);
            let solid =
                (below_facts.full_collision || powder_snow && f.fall_distance <= 2.5 || sunk)
                    && f.velocity.y < 0.0;
            // FallingBlockEntity.tick: off the ground, it gives up after 600
            // ticks (100 outside the world) and drops itself as an item.
            let y = next.y.floor() as i32;
            let outside =
                y <= horizon_client::chunk::MIN_Y || y >= horizon_client::chunk::MIN_Y + 384;
            if !solid && (f.age > 100 && outside || f.age > 600) {
                if f.drop_item && entity_drops {
                    dropped.push((f.state, next));
                }
                return false;
            }
            if solid {
                landed.push((
                    f.state,
                    [
                        next.x.floor() as i32,
                        next.y.floor() as i32 + i32::from(!sunk),
                        next.z.floor() as i32,
                    ],
                ));
                return false;
            }
            if next.y < f.position.y {
                f.fall_distance += f.position.y - next.y;
            }
            f.position = next;
            f.velocity = Vec3::new(
                f.velocity.x * 0.98,
                f.velocity.y * 0.98,
                f.velocity.z * 0.98,
            );
            true
        });
        self.falling = falling;
        for (state, at) in dropped {
            // Entity.spawnAtLocation: an item entity where the block was.
            let item = self.res.facts.get(state).item.clone();
            if &*item != "minecraft:air" {
                let r = &mut self.world.random;
                let v = [
                    r.next_double() * 0.2 - 0.1,
                    0.2,
                    r.next_double() * 0.2 - 0.1,
                ];
                let stack = ItemStack {
                    item,
                    count: 1,
                    damage: 0,
                    extra: None,
                };
                self.spawn_item(stack, [at.x, at.y, at.z], v, 10);
            }
        }
        for (state, pos) in landed {
            let landing = self.with_level(|b, level, ticks| {
                b.falling_block_landed(
                    level,
                    ticks,
                    BlockPos::new(pos[0], pos[1], pos[2]),
                    state,
                    false,
                )
            });
            if matches!(landing, Landing::Broken) {
                let item = self.res.facts.get(state).item.clone();
                if &*item != "minecraft:air" {
                    self.pop_resource(
                        pos,
                        ItemStack {
                            item,
                            count: 1,
                            damage: 0,
                            extra: None,
                        },
                    );
                }
            }
        }
        self.process_level_events()
    }
    fn break_block(&mut self, pos: [i32; 3]) -> Result<()> {
        self.with_player_actor(|game| game.break_block_as_player(pos))
    }
    fn break_block_as_player(&mut self, pos: [i32; 3]) -> Result<()> {
        // ServerPlayerGameMode.destroyBlock.
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let facts = self.res.facts.get(state).clone();
        // Block.playerWillDestroy: mining their gold angers all nearby piglins.
        if self.guarded_by_piglins(&facts.name) {
            self.anger_piglins(false);
        }
        // The block entity as it was, for what the drops copy from it.
        let key = (pos[0], pos[1], pos[2]);
        if let Some(entity) = self.world.block_entities.get(&key).cloned() {
            let name = self.world.block_names.get(&key).cloned();
            self.pending.record(pos, &entity, &facts.name, name);
        } else if let Some(tag) = spill::saved_entity(&self.world, pos) {
            self.pending.record_saved(pos, &tag);
        }
        self.creaking_heart_broken(pos);
        // playerWillDestroy and Level.removeBlock through the vanilla behaviour
        // library: the block entity's side effects (containers spill), neighbour
        // and shape updates, and the destroy event (particles, sound).
        let creative = self.creative;
        let removed = self.with_level(|b, level, ticks| {
            b.player_destroy_block(
                level,
                ticks,
                BlockPos::new(pos[0], pos[1], pos[2]),
                creative,
            )
        });
        if removed && !creative {
            let held = self.held().cloned();
            // ItemStack.mineBlock: tools lose durability on blocks with nonzero hardness.
            if facts.hardness > 0.0
                && let Some(stack) = self.inventory[self.selected].clone()
                && let Some(max) = self
                    .res
                    .items
                    .get(&stack.item)
                    .filter(|i| i.max_damage > 0 && !i.tool.is_empty())
                    .map(|i| i.max_damage)
            {
                let loss = self.durability_loss(&stack, 1);
                if let Some(stack) = self.inventory[self.selected].as_mut() {
                    stack.damage += loss;
                    if stack.damage >= max {
                        self.inventory[self.selected] = None;
                    }
                }
            }
            let harvest = !facts.requires_tool
                || self.res.items.correct_for_drops(held.as_ref(), &facts.name);
            if harvest {
                // Block.playerDestroy: the mined statistic, the exhaustion and the drops.
                self.stats.add("mined", &facts.name, 1);
                if let Some(p) = self.player.as_mut() {
                    p.health.food.exhaustion = (p.health.food.exhaustion + 0.005).min(40.0);
                }
                let drops = self.block_loot_at(pos, state, held.as_ref());
                let silk = held
                    .as_ref()
                    .is_some_and(|h| h.enchantment_level("minecraft:silk_touch") > 0);
                for d in drops {
                    if &*d.item != "minecraft:air" {
                        self.pop_resource(pos, d);
                    }
                }
                let xp = if silk {
                    0
                } else {
                    self.block_experience(&facts.name)
                };
                if xp > 0 {
                    self.spawn_orbs(
                        [
                            f64::from(pos[0]) + 0.5,
                            f64::from(pos[1]) + 0.5,
                            f64::from(pos[2]) + 0.5,
                        ],
                        xp,
                    );
                }
            }
        }
        self.pending.clear(pos);
        self.process_level_events()?;
        Ok(())
    }
    fn use_item(&mut self) -> Result<()> {
        self.with_player_actor(|game| game.use_item_as_player())
    }
    fn use_item_as_player(&mut self) -> Result<()> {
        // Spectators only look (MultiPlayerGameMode.useItemOn/useItem pass).
        if self.spectator() {
            return Ok(());
        }
        // A carrot or warped fungus on a stick boosts the ridden pig or strider.
        if self.riding.is_some()
            && let Some(item) = self.held().map(|s| s.item.clone())
            && self.use_steering_stick(&item)
        {
            return Ok(());
        }
        if self.interact_frame()
            || self.interact_armor_stand()
            || self.interact_boat()
            || self.interact_cart()
            || self.interact_mob()?
            || self.interact_knot()
        {
            return Ok(());
        }
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        // MultiPlayerGameMode.useItemOn: blocks react first unless sneaking with an item.
        if !(sneaking && self.held().is_some())
            && let Some(hit) = self.pick()
        {
            let held = self.held().cloned();
            if self.use_lectern(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(());
            }
            if self.use_jukebox(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(());
            }
            if self.use_composter(hit.pos, held.as_ref()) {
                self.swing_arm();
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
            let player = PlayerContext {
                yaw: self.yaw,
                pitch: self.pitch,
                may_build: self.may_build(),
                can_eat: self.creative
                    || self
                        .player
                        .as_ref()
                        .is_some_and(|p| p.health.food.food < 20),
                game_master: false,
            };
            let at = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
            let result = self.with_level(|b, level, ticks| {
                b.use_without_item(
                    level,
                    ticks,
                    BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                    face,
                    at,
                    player,
                )
            });
            self.process_level_events()?;
            if result.consumes_action() {
                self.swing_arm();
                return Ok(());
            }
        }
        let Some(stack) = self.held().cloned() else {
            self.use_offhand();
            return Ok(());
        };
        let Some(info) = self.res.items.get(&stack.item).cloned() else {
            return Ok(());
        };
        if self.use_potion_item(&stack)? || self.throw_item(&stack)? {
            return Ok(());
        }
        if self.raise_shield(self.selected)
            || self.use_fishing_rod(&stack)
            || self.use_crossbow(&stack)
            || self.use_trident(&stack)
            || self.boost_firework(&stack)
            || self.use_empty_map(&stack)
            || self.use_book(&stack)
            // Block items (heads, carved pumpkins) aimed at a block are
            // placed first (useItemOn before useItem).
            || (!(info.block.is_some() && self.pick().is_some()) && self.equip_from_hand(&stack))
        {
            return Ok(());
        }
        if &*stack.item == "minecraft:spyglass" {
            self.use_spyglass();
            return Ok(());
        }
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
        if self.use_bone_meal(&stack)? || self.ignite_tnt(&stack)? {
            return Ok(());
        }
        if self.use_tool(&stack)? {
            return Ok(());
        }
        if self.place_boat(&stack) {
            return Ok(());
        }
        if let Some(hit) = self.pick()
            && (self.launch_firework(&hit, &stack)
                || self.place_minecart(&hit, &stack)
                || self.place_painting(&hit, &stack)
                || self.place_item_frame(&hit, &stack)
                || self.place_armor_stand(&hit, &stack)
                || self.place_end_crystal(&hit, &stack))
        {
            return Ok(());
        }
        if self.use_ender_eye(&stack)? {
            return Ok(());
        }
        let Some(hit) = self.pick() else {
            return Ok(());
        };
        if info.block.is_none() {
            self.use_offhand();
            return Ok(());
        }
        if !self.may_build() {
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
            let sign = matches!(entity, BlockEntity::Sign(_));
            self.world
                .block_entities
                .insert((target[0], target[1], target[2]), entity);
            self.fill_placed_box(target, &stack);
            self.name_placed_container(target, &stack);
            self.fill_placed_banner(target, &stack);
            if sign {
                self.open_sign_editor((target[0], target[1], target[2]), true);
            }
        }
        self.swing_arm();
        let sounds = self.res.facts.get(state).sounds.clone();
        let center = [
            f64::from(target[0]) + 0.5,
            f64::from(target[1]) + 0.5,
            f64::from(target[2]) + 0.5,
        ];
        // SolidBucketItem.getPlaceSound: the bucket's own empty sound.
        let solid_bucket = &*stack.item == "minecraft:powder_snow_bucket";
        let place_sound = if solid_bucket {
            "minecraft:item.bucket.empty_powder_snow"
        } else {
            &sounds.place
        };
        self.audio.play(
            place_sound,
            Some(center),
            (sounds.volume + 1.0) / 2.0,
            sounds.pitch * 0.8,
        );
        self.process_level_events()?;
        // BlockItem.place: the used statistic and the placed_block trigger.
        self.stats.add("used", &stack.item, 1);
        self.trigger_block("minecraft:placed_block", target, &stack);
        self.check_wither_spawn(target)?;
        self.check_golem_spawn(target)?;
        self.consume_held();
        // SolidBucketItem.useOn: BucketItem.getEmptySuccessItem in hand.
        if solid_bucket && !self.creative {
            self.inventory[self.selected] = Some(ItemStack::new("minecraft:bucket", 1));
        }
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
        // ItemCooldowns.tick for the shield.
        self.shield_cooldown = self.shield_cooldown.saturating_sub(1);
        let Some((slot, ticks)) = self.using else {
            return;
        };
        let Some(stack) = self.stack_in(slot).cloned() else {
            self.using = None;
            return;
        };
        // Switching hotbar slots only ends a main-hand use.
        let hand_changed = slot != self.selected && slot != shield::OFFHAND_SLOT;
        if !self.right || hand_changed || self.screen != Screen::Playing {
            if &*stack.item == "minecraft:bow" && slot == self.selected {
                self.release_bow(ticks);
            }
            if &*stack.item == "minecraft:spyglass" {
                self.stop_spyglass();
            }
            if &*stack.item == "minecraft:trident" && slot == self.selected {
                self.release_trident(ticks);
            }
            self.using = None;
            return;
        }
        let Some(info) = self.res.items.get(&stack.item).cloned() else {
            self.using = None;
            return;
        };
        if info.blocks_attacks.is_some() {
            // ShieldItem: used for 72000 ticks, i.e. until released.
            self.using = Some((slot, ticks + 1));
            return;
        }
        if self.tick_drinking(slot, ticks, &stack) {
            return;
        }
        if &*stack.item == "minecraft:bow" {
            self.using = Some((slot, ticks + 1));
            return;
        }
        if &*stack.item == "minecraft:crossbow" {
            self.tick_crossbow(slot, ticks, &stack);
            return;
        }
        if &*stack.item == "minecraft:trident" {
            self.using = Some((slot, ticks + 1));
            return;
        }
        if &*stack.item == "minecraft:spyglass" {
            // SpyglassItem.USE_DURATION, then finishUsingItem.
            if ticks + 1 >= 1200 {
                self.stop_spyglass();
                self.using = None;
            } else {
                self.using = Some((slot, ticks + 1));
            }
            return;
        }
        let duration = 32;
        // Consumable.shouldEmitParticlesAndSounds(useItemRemainingTicks), with
        // the remaining ticks before this tick counts down: every 4 ticks once
        // more than 21.875% of the time has passed (none on the last tick).
        let remaining = duration - ticks.min(duration);
        let ticks = ticks + 1;
        let eye = self.eye_position().unwrap_or_default();
        if duration - remaining > (duration as f32 * 0.21875) as u32 && remaining % 4 == 0 {
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.2 + 1.0;
            self.audio.play(
                "minecraft:entity.generic.eat",
                Some(eye),
                0.5 + 0.5 * self.world.random.next_int_bound(2).unwrap_or(0) as f32,
                pitch,
            );
            self.eat_particles(&stack, 5);
        }
        if ticks >= duration {
            self.using = None;
            // Consumable.onConsume: emitParticlesAndSounds(..., 16).
            self.eat_particles(&stack, 16);
            // Consumable.onConsume.
            self.post_entity_event("minecraft:eat", horizon_client::mobs::Who::Player);
            if let Some((nutrition, saturation)) = info.food
                && let Some(p) = self.player.as_mut()
            {
                // FoodData.eat(nutrition, saturation)
                let food = &mut p.health.food;
                food.food = (food.food + nutrition as i32).clamp(0, 20);
                food.saturation = (food.saturation + saturation).clamp(0.0, food.food as f32);
                self.audio.play(
                    "minecraft:entity.player.burp",
                    Some(eye),
                    0.5,
                    self.world.random.next_float() * 0.1 + 0.9,
                );
                self.trigger_item("minecraft:consume_item", &stack);
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
        // GameType.isBlockPlacingRestricted: adventure and spectator.
        if !self.may_build() {
            self.mining = None;
            return Ok(());
        }
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
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                ticks: 0,
            });
        }
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let facts = self.res.facts.get(state);
        if facts.hardness < 0.0 {
            return Ok(());
        }
        // Player.getDestroySpeed / BlockBehaviour.getDestroyProgress.
        let held = self.held();
        let mut speed = self.res.items.mining_speed(held, &facts.name);
        speed = self.modify_destroy_speed(speed);
        let player = self.player.as_ref().ok_or("no player")?;
        if !player.ground.on_ground {
            speed /= 5.0;
        }
        let eye = self.eye_position().unwrap_or_default();
        if self
            .res
            .facts
            .get(self.world.block(
                eye[0].floor() as i32,
                eye[1].floor() as i32,
                eye[2].floor() as i32,
            ))
            .fluid
            == Fluid::Water
        {
            speed *= self.submerged_mining_speed();
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
            let c = [
                f64::from(m.pos[0]) + 0.5,
                f64::from(m.pos[1]) + 0.5,
                f64::from(m.pos[2]) + 0.5,
            ];
            self.audio.play(
                &hit_sound.hit,
                Some(c),
                (hit_sound.volume + 1.0) / 8.0,
                hit_sound.pitch * 0.5,
            );
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
                if floor.full_collision
                    && floor.fluid == Fluid::None
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
                let ok = f.full_collision
                    && f.fluid == Fluid::None
                    && !f.name.contains("leaves")
                    && f.hardness >= 0.0;
                return ok.then_some(y + 1);
            }
            y -= 1;
        }
        None
    }
    fn spawn_player(&mut self) -> Result<()> {
        if self.player.is_some() || self.remote.is_some() {
            return Ok(());
        }
        if let Some((_, target)) = &self.travelling {
            let (cx, cz) = (
                (target.x.floor() as i32) >> 4,
                (target.z.floor() as i32) >> 4,
            );
            self.streaming.center = (cx, cz);
            if (-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                let (player, target) = self.travelling.take().ok_or("travel")?;
                if self.end_travel {
                    self.place_after_end_travel(player, target)?;
                } else {
                    self.place_after_travel(player, target)?;
                }
            }
            return Ok(());
        }
        let saved = self.level.position.map(|p| Vec3::new(p[0], p[1], p[2]));
        if self.spawn.is_none() {
            self.spawn = self
                .level
                .spawn
                .map(|s| Vec3::new(s[0] + 0.5, s[1], s[2] + 0.5));
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
            let y = self
                .land_column(s.x.floor() as i32, s.z.floor() as i32)
                .map_or(s.y, f64::from);
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
        let spectator = self.mode == horizon_client::command::args::GameMode::Spectator;
        player.may_fly = self.creative || spectator;
        player.invulnerable = self.creative || spectator;
        player.spectator = spectator;
        player.pitch = self.pitch;
        // LivingEntity.canGlideUsing: a glider in its slot that will not break next.
        player.glider = self.armor[1].as_ref().is_some_and(|s| {
            self.res
                .items
                .get(&s.item)
                .is_some_and(|i| i.glider && (i.max_damage == 0 || s.damage + 1 < i.max_damage))
        });
        if self.level.health > 0.0 && self.ticks < 2 {
            player.set_health(self.level.health, false)?;
            player.health.food.food = self.level.food;
            player.health.food.saturation = self.level.saturation;
        }
        self.previous_position = feet;
        self.player = Some(player);
        self.sway = Default::default();
        self.apply_equipment();
        self.eye = 1.62;
        self.previous_eye = 1.62;
        self.clock.reset();
        Ok(())
    }
    fn tick(&mut self) -> Result<()> {
        self.tick_debug_keys()?;
        if self.remote.is_some() {
            return self.tick_remote();
        }
        self.ticks += 1;
        // `/tick freeze` stops the world but not the player.
        let running = self.clock.world_runs();
        if running {
            self.world.game_time += 1;
            if self.ui.rules.bool("advance_weather") {
                self.world.weather.tick(&mut self.world.random);
            }
            self.tick_weather_effects();
        }
        if self.ticks.is_multiple_of(6000) {
            self.save_game()?;
        }
        if running {
            self.advance_clock();
        }
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        self.apply_totem();
        self.update_totem_ready();
        self.tick_command_extras();
        let camera_eye = self.camera_eye_height();
        if self.riding.is_some() && self.player.is_some() {
            let playing = self.screen == Screen::Playing;
            let held = |k| playing && self.keys.contains(&k);
            let keys = Keys {
                forward: held(KeyCode::KeyW),
                backward: held(KeyCode::KeyS),
                left: held(KeyCode::KeyA),
                right: held(KeyCode::KeyD),
                jump: held(KeyCode::Space),
                shift: held(KeyCode::ShiftLeft),
                sprint: false,
            };
            self.tick_riding(keys)?;
        }
        let riding = self.riding.is_some();
        let Some(player) = self.player.as_mut() else {
            // A dedicated server has no local player: the world still runs.
            if self.options.dedicated.is_some() {
                return self.tick_world(running);
            }
            return Ok(());
        };
        // Hold position until the surrounding terrain exists.
        let (cx, cz) = (
            (player.position.x.floor() as i32) >> 4,
            (player.position.z.floor() as i32) >> 4,
        );
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
        if !riding {
            self.previous_position = player.position;
        }
        player.yaw = self.yaw;
        let spectator = self.mode == horizon_client::command::args::GameMode::Spectator;
        player.may_fly = self.creative || spectator;
        // LocalPlayer.sendPosition: autoJumpEnabled follows the option.
        player.auto_jump_enabled = self.settings.auto_jump;
        player.invulnerable = self.creative || spectator;
        player.spectator = spectator;
        player.pitch = self.pitch;
        // LivingEntity.canGlideUsing: a glider in its slot that will not break next.
        player.glider = self.armor[1].as_ref().is_some_and(|s| {
            self.res
                .items
                .get(&s.item)
                .is_some_and(|i| i.glider && (i.max_damage == 0 || s.damage + 1 < i.max_damage))
        });
        // LocalPlayer.modifyInput: walking while using an item.
        player.item_use_speed = self
            .using
            .and_then(|(slot, _)| {
                if slot == shield::OFFHAND_SLOT {
                    self.offhand.as_ref()
                } else {
                    self.inventory.get(slot)?.as_ref()
                }
            })
            .and_then(|s| self.res.items.get(&s.item))
            .map(|i| i.use_speed);
        // PowderSnowBlock.canEntityWalkOnPowderSnow and LivingEntity.canFreeze.
        player.leather_boots = self.armor[3]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:leather_boots");
        let immune = self
            .res
            .items
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        player.freeze_immune = self
            .armor
            .iter()
            .flatten()
            .any(|s| immune.is_some_and(|t| t.contains(&*s.item)));
        let grounded = player.ground.on_ground;
        let before = (
            grounded,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        if !riding {
            MotionWorld {
                player,
                scene: &mut self.world,
                assets: &self.res.assets,
                collision_binding: self.collision,
                resources: self.motion,
            }
            .controlled_simulation_tick(keys)?;
            // Player.jumpFromGround: the jump statistic.
            if grounded && keys.jump && !player.flying && player.velocity.y > 0.2 {
                self.stats.custom("jump", 1);
            }
        }
        let events = std::mem::take(&mut player.events);
        let glider_wear = std::mem::take(&mut player.glider_wear);
        if glider_wear > 0
            && !self.creative
            && let Some(elytra) = self.armor[1].as_mut()
        {
            elytra.damage += glider_wear;
        }
        let feet = [player.position.x, player.position.y, player.position.z];
        self.previous_eye = self.eye;
        self.eye += (camera_eye.unwrap_or(player.dimensions.eye_height) - self.eye) * 0.5;
        let crouching = player.pose == horizon_client::pose::Pose::Crouching;
        self.tick_avatar(before, riding);
        self.record_combat(&events);
        for event in events {
            match event {
                MotionEvent::Emission {
                    position, state, ..
                } if !crouching => {
                    // Entity.playStepSound: the block's step sound at 0.15 volume.
                    let sounds = self.res.facts.get(state).sounds.clone();
                    self.audio
                        .play(&sounds.step, Some(feet), sounds.volume * 0.15, sounds.pitch);
                    let _ = position;
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Sound {
                    sound,
                    volume,
                    pitch,
                    ..
                }) => {
                    let name = match sound {
                        horizon_client::landing::SceneSound::BigFall => {
                            "minecraft:entity.player.big_fall"
                        }
                        horizon_client::landing::SceneSound::SmallFall => {
                            "minecraft:entity.player.small_fall"
                        }
                        _ => continue,
                    };
                    self.audio.play(name, Some(feet), volume, pitch);
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Write {
                    position,
                    flags,
                    ..
                }) => {
                    // Landing rules (farmland trampling, turtle eggs) wrote the block raw.
                    self.with_level(|b, level, ticks| {
                        b.notify_written(level, ticks, position, flags as i32)
                    });
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Destroy {
                    position, ..
                }) => {
                    self.with_level(|b, level, ticks| {
                        b.notify_written(level, ticks, position, level_flags::UPDATE_ALL)
                    });
                }
                MotionEvent::Fall(horizon_client::landing::FallEvent::Damage { .. }) => {
                    self.audio
                        .play("minecraft:entity.player.hurt", Some(feet), 1.0, 1.0);
                }
                MotionEvent::Snowflake { position, velocity } => self.snowflake(position, velocity),
                MotionEvent::EnterBlock { state } => self.trigger_enter_block(state),
                _ => {}
            }
        }
        self.powder_snow_fire()?;
        self.apply_totem();
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        if player.position.y < f64::from(MIN_Y - 64) {
            // Out of the world.
            let _ = player.set_health(0.0, false);
        }
        if player.health.dead_or_dying() && self.screen != Screen::Dead {
            // Inventory.dropAll: every stack thrown in a random direction.
            let at = [
                player.position.x,
                player.position.y + 1.32,
                player.position.z,
            ];
            let killer = self.combat_killer_kind();
            self.death_message();
            // ServerPlayer.die: deaths, and the time-since counters restart.
            self.stats.custom("deaths", 1);
            self.score_player_died();
            if let Some(kind) = killer {
                self.stats.add("killed_by", &kind, 1);
                let view = horizon_client::advancements::EntityView {
                    kind,
                    ..Default::default()
                };
                self.trigger_entity("minecraft:entity_killed_player", "entity", &view);
            }
            self.stats.reset("custom", "minecraft:time_since_death");
            self.stats.reset("custom", "minecraft:time_since_rest");
            if !self.creative && !self.ui.rules.bool("keep_inventory") {
                // Inventory.dropAll: slots 0..36, armor feet to head (36..39), off hand (40).
                let mut stacks: Vec<ItemStack> =
                    self.inventory.iter_mut().filter_map(Option::take).collect();
                stacks.extend(self.armor.iter_mut().rev().filter_map(Option::take));
                stacks.extend(self.offhand.take());
                self.apply_equipment();
                for stack in stacks {
                    let r = &mut self.world.random;
                    let strength = f64::from(r.next_float()) * 0.5;
                    let angle = f64::from(r.next_float()) * std::f64::consts::TAU;
                    let v = [-angle.sin() * strength, 0.2, angle.cos() * strength];
                    self.spawn_item(stack, at, v, 40);
                }
            }
            self.audio
                .play("minecraft:entity.player.death", Some(at), 1.0, 1.0);
            self.screen = Screen::Dead;
            self.capture(false);
        }
        self.tick_world(running)
    }
    /// What the tick runs after the local player's own update: effects, the
    /// world's entities and blocks, and the network hub.
    fn tick_world(&mut self, running: bool) -> Result<()> {
        horizon_client::spans::time("tick_effects", || self.tick_effects());
        horizon_client::spans::time("tick_burning", || self.tick_burning());
        horizon_client::spans::time("tick_using", || self.tick_using());
        horizon_client::spans::time("tick_hud", || self.tick_hud());
        // The player's digging comes in with its packets, before the level
        // ticks; ServerLevel.tick then runs the scheduled and random block
        // ticks before any entity, so what they drop or spawn (dispensed
        // items, popped crops) moves on the tick it appears, as in vanilla.
        horizon_client::spans::time("tick_local_hand", || self.tick_local_hand())?;
        horizon_client::spans::time("postprocess_chunks", || self.postprocess_chunks())?;
        if running {
            horizon_client::spans::time("tick_blocks", || self.tick_blocks())?;
        }
        if running {
            horizon_client::spans::time("tick_paintings", || self.tick_paintings());
            horizon_client::spans::time("tick_item_frames", || self.tick_item_frames());
            horizon_client::spans::time("tick_armor_stands", || self.tick_armor_stands());
            horizon_client::spans::time("tick_boats", || self.tick_boats());
            horizon_client::spans::time("tick_carts", || self.tick_carts());
            horizon_client::spans::time("tick_thunder", || self.tick_thunder());
            horizon_client::spans::time("tick_lecterns", || self.tick_lecterns());
            horizon_client::spans::time("tick_maps", || self.tick_maps());
            horizon_client::spans::time("tick_bolts", || self.tick_bolts());
            horizon_client::spans::time("tick_fishing", || self.tick_fishing());
            horizon_client::spans::time("tick_tridents", || self.tick_tridents());
            horizon_client::spans::time("tick_rockets", || self.tick_rockets());
        }
        if running {
            horizon_client::spans::time("tick_items", || self.tick_items());
        }
        horizon_client::spans::time("spawn_loaded_entities", || self.spawn_loaded_entities());
        if running {
            horizon_client::spans::time("tick_mobs", || self.tick_mobs())?;
            horizon_client::spans::time("recheck_golem_openers", || self.recheck_golem_openers());
            horizon_client::spans::time("tick_experience", || self.tick_experience());
        }
        horizon_client::spans::time("tick_lan", || self.tick_lan());
        horizon_client::spans::time("tick_hand", || self.tick_hand());
        horizon_client::spans::time("tick_sleep", || self.tick_sleep());
        horizon_client::spans::time("tick_portal", || self.tick_portal())?;
        if running {
            horizon_client::spans::time("tick_end", || self.tick_end())?;
        }
        if running {
            horizon_client::spans::time("tick_falling", || self.tick_falling())?;
            horizon_client::spans::time("tick_tnt", || self.tick_tnt())?;
        }
        if running {
            horizon_client::spans::time("tick_block_entities", || self.tick_block_entities())?;
            horizon_client::spans::time("tick_sculk_entities", || self.tick_sculk_entities());
            horizon_client::spans::time("tick_sculk_blocks", || self.tick_sculk_blocks())?;
            horizon_client::spans::time("tick_brewing_stands", || self.tick_brewing_stands())?;
        }
        if running {
            // ClientLevel.tickBlockEntities (not while paused or frozen):
            // chest lids, books, shulker boxes, bells, conduits, heads.
            horizon_client::spans::time("tick_block_entity_anims", || {
                self.tick_block_entity_anims()
            });
            horizon_client::spans::time("tick_particles", || self.tick_particles());
        }
        if self.right && self.use_delay == 0 && self.screen == Screen::Playing {
            self.use_with_hand()?;
            self.use_delay = 4;
        }
        Ok(())
    }
    /// Keep the held-key state of toggled sneak and sprint in step with their mapping.
    fn sync_toggle(&mut self, slot: usize) {
        let key = [KeyCode::ShiftLeft, KeyCode::ControlLeft][slot];
        if self.toggles.is_down(slot) {
            self.keys.insert(key);
        } else {
            self.keys.remove(&key);
        }
    }
    /// `KeyMapping.releaseAll` when a screen opens and
    /// `restoreToggleStatesOnScreenClosed` when it closes.
    fn track_toggle_screens(&mut self) {
        let playing = self.screen == Screen::Playing && self.title.is_none();
        if playing == self.was_playing {
            return;
        }
        self.was_playing = playing;
        let options = [
            self.settings.toggle_crouch,
            self.settings.toggle_sprint,
            self.settings.toggle_attack,
            self.settings.toggle_use,
        ];
        if playing {
            self.toggles.screen_closed(options);
        } else {
            self.toggles.screen_opened(options);
        }
        self.sync_toggle(0);
        self.sync_toggle(1);
        self.left = self.toggles.is_down(2);
        self.right = self.toggles.is_down(3);
    }
    /// The options `MouseHandler.turnPlayer` reads.
    fn look_options(&self) -> LookOptions {
        LookOptions {
            sensitivity: f64::from(self.settings.sensitivity),
            smooth_camera: self.smooth_camera,
            scoping: self.scoping(),
            invert_x: self.settings.invert_x,
            invert_y: self.settings.invert_y,
        }
    }
    /// `MouseHandler.handleAccumulatedMovement` and `Entity.turn`, once per frame.
    /// An agent moving the mouse (`mouse-move`) counts as a grabbed pointer in
    /// an active window, as it cannot grab the real one.
    fn apply_mouse_look(&mut self) {
        let look = self.look_options();
        let time = self.started.elapsed().as_secs_f64();
        let pointer = self.captured || self.agent_pointer;
        let grabbed = pointer && self.screen == Screen::Playing && self.player.is_some();
        let active = self.focused || self.agent_pointer;
        if let Some((xo, yo)) = self
            .mouse
            .handle_accumulated_movement(time, active, grabbed, look)
        {
            let mut rotation = Rotation {
                x_rot: self.pitch,
                y_rot: self.yaw,
                x_rot_o: self.pitch,
                y_rot_o: self.yaw,
            };
            rotation.turn(xo, yo);
            self.yaw = rotation.y_rot;
            self.pitch = rotation.x_rot;
        }
    }
    /// `Minecraft.pause`: a single-player game with a pausing screen open.
    fn pauses_game(&self) -> bool {
        self.remote.is_none()
            && self.title.is_none()
            && (matches!(
                self.screen,
                Screen::Paused | Screen::Options | Screen::Stats | Screen::Advancements
            ) || self.book.is_some())
    }
    fn capture(&mut self, capture: bool) {
        // The real pointer takes over from an agent's.
        self.agent_pointer = false;
        let Some(window) = &self.window else { return };
        if capture {
            self.mouse.set_ignore_first_move();
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
    fn draw_item(&mut self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        // `GuiGraphics.renderItem`: the resolved item model (tints, foil and
        // special renderers included), then the decorations.
        self.draw_item_model(batch, stack, x, y);
        self.draw_item_decorations(batch, stack, x, y);
    }
    /// `GuiGraphicsExtractor.itemDecorations`: the count and damage bar.
    fn draw_item_decorations(&mut self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        // GuiGraphicsExtractor.itemDecorations, laid out at the origin and
        // moved to (x, y). Horizon tracks no item cooldowns to shade yet.
        let max_damage = self
            .res
            .items
            .get(&stack.item)
            .map_or(0, |info| info.max_damage as i32);
        let decorations = hud_overlays::ItemDecorations {
            count: stack.count,
            count_text: None,
            bar: hud_overlays::damage_bar(stack.damage as i32, max_damage),
            cooldown: 0.0,
        };
        let mut draws = vec![hud_layout::GuiDraw::Push(vec![
            hud_layout::PoseOp::Translate(x, y),
        ])];
        draws.extend(hud_overlays::item_decorations(
            0,
            0,
            &decorations,
            &self.res.atlas.font,
        ));
        draws.push(hud_layout::GuiDraw::Pop);
        hud_layout::paint(batch, &self.res.atlas, &draws);
    }
    fn hud(&mut self, size: [u32; 2]) -> HudBatch {
        let mut batch = self.hud_layers(size);
        self.frame_graph
            .draw(self.settings.horizon.overlay, &mut batch, &self.res.atlas);
        batch
    }
    fn hud_layers(&mut self, size: [u32; 2]) -> HudBatch {
        let mut batch = HudBatch::new(size);
        let (w, h) = (batch.width, batch.height);
        if self.screen == Screen::Options {
            self.draw_options(&mut batch);
            return batch;
        }
        if let Some(title) = &self.title {
            if horizon_client::ui_theme::horizon() {
                let fade = self.backdrop_fade();
                let mut ui =
                    horizon_client::ui_theme::Ui::new(&self.res.atlas, &mut batch, self.cursor);
                title.draw_horizon(&mut ui, fade);
                ui.finish();
            } else {
                title.draw(&self.res.atlas, &mut batch, self.cursor);
            }
            return batch;
        }
        let atlas_ptr: *const HudAtlas = &self.res.atlas;
        let _ = atlas_ptr;
        if self.remote_overlay(&mut batch) {
            return batch;
        }
        if self.player.is_none() {
            let text = if self.spawn_search > 0 {
                "Finding a place to spawn..."
            } else {
                "Loading terrain..."
            };
            let atlas = &self.res.atlas;
            let tw = HudBatch::text_width(atlas, text);
            batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255));
            batch.text(atlas, text, (w - tw) / 2.0, h / 2.0 - 4.0, u32::MAX, true);
            return batch;
        }
        // Hud.extractRenderState: F1 (keyToggleGui) hides camera overlays, the
        // crosshair, hotbar, effects, bars, sidebar, action bar, titles and chat;
        // the sleep overlay, debug screen and open screens still draw.
        let spectator = self.spectator();
        let (wi, hi) = (w as i32, h as i32);
        // Horizon's pause menu sits over the blurred world alone.
        if !self.ui.hide_gui && !self.horizon_pause() {
            self.draw_spyglass(&mut batch);
            // Hud.extractCrosshair / extractItemHotbar / extractPlayerHealth /
            // extractVehicleHealth and the experience bar, laid out by hud_layout.
            // Horizon has no attack cooldown yet: the strength is always full.
            let indicator = self.settings.attack_indicator;
            if self.camera_type.first_person() && !spectator {
                let attack = (indicator == 1).then_some(1.0);
                let draws = hud_layout::crosshair(wi, hi, attack, false);
                hud_layout::paint(&mut batch, &self.res.atlas, &draws);
            }
            if !spectator {
                let inputs = hud_layout::HotbarInputs {
                    selected: self.selected as i32,
                    right_handed: !self.settings.main_hand_left,
                    offhand: self.offhand.is_some(),
                    slots: std::array::from_fn(|i| self.inventory[i].as_ref().map(|_| 0.0)),
                    offhand_pop: 0.0,
                    attack: (indicator == 2).then_some(1.0),
                };
                let draws = hud_layout::hotbar(wi, hi, &inputs);
                hud_layout::paint(&mut batch, &self.res.atlas, &draws);
                for d in &draws {
                    if let hud_layout::GuiDraw::Item { slot, x, y, .. } = *d {
                        let stack = if slot == 40 {
                            self.offhand.clone()
                        } else {
                            self.inventory[slot as usize].clone()
                        };
                        if let Some(stack) = stack {
                            self.draw_item(&mut batch, &stack, x as f32, y as f32);
                        }
                    }
                }
            }
            self.draw_effect_icons(&mut batch);
            // Hud: the action bar and titles draw over the hotbar (nextStratum).
            self.draw_boss_bars(&mut batch);
        }
        // Hud.extractSleepOverlay, under the sidebar, titles and chat.
        if let Some((_, ticks)) = self.sleeping {
            let draws = hud_overlays::sleep_overlay(wi, hi, ticks as i32);
            hud_layout::paint(&mut batch, &self.res.atlas, &draws);
        }
        if !self.ui.hide_gui {
            self.draw_titles(&mut batch);
            self.draw_scoreboard(&mut batch);
            self.draw_subtitles(&mut batch);
            // The mount's hearts (and the jump bar that replaces the experience bar).
            let (vehicle_hearts, jump_bar) = if spectator {
                (0, false)
            } else {
                self.draw_mount_hud(&mut batch, wi, hi)
            };
            if !self.creative
                && !spectator
                && let Some(p) = &mut self.player
            {
                let atlas = &self.res.atlas;
                let width = |s: &str| HudBatch::text_width(atlas, s) as i32;
                let draws = hud_layout::experience(
                    wi,
                    hi,
                    self.experience.progress,
                    self.experience.level,
                    !jump_bar,
                    &width,
                );
                hud_layout::paint(&mut batch, atlas, &draws);
                let effects = &self.potions.effects;
                let kind = if effects.get("poison").is_some() {
                    hud_layout::HeartKind::Poisoned
                } else if effects.get("wither").is_some() {
                    hud_layout::HeartKind::Withered
                } else if p.is_fully_frozen() {
                    hud_layout::HeartKind::Frozen
                } else {
                    hud_layout::HeartKind::Normal
                };
                let eye = p.position.y + f64::from(p.dimensions.eye_height);
                let under_water = self
                    .res
                    .facts
                    .get(self.world.block(
                        p.position.x.floor() as i32,
                        eye.floor() as i32,
                        p.position.z.floor() as i32,
                    ))
                    .fluid
                    == horizon_client::blocks::Fluid::Water;
                let inputs = hud_layout::HealthInputs {
                    tick: self.ticks as i32,
                    millis: self.started.elapsed().as_millis() as i64,
                    health: p.health.health(),
                    max_health: p.attribute("minecraft:max_health").unwrap_or(20.0) as f32,
                    absorption: p.health.absorption(),
                    armor: p
                        .attribute("minecraft:armor")
                        .or_else(|_| p.attribute("armor"))
                        .unwrap_or(0.0) as i32,
                    food: p.health.food.food,
                    saturation: p.health.food.saturation,
                    kind,
                    regeneration: effects.get("regeneration").is_some(),
                    hunger: effects.get("hunger").is_some(),
                    hardcore: self.level.hardcore,
                    damage_cooldown: p.health.hurt.cooldown > 0,
                    // Horizon does not track the player's air supply yet.
                    air: 300,
                    max_air: 300,
                    under_water,
                    vehicle_hearts,
                };
                let draws = self.health_hud.layout(&inputs, wi, hi);
                hud_layout::paint(&mut batch, atlas, &draws);
            }
            if !spectator {
                self.draw_selected_name(&mut batch, !self.creative);
            }
        }
        let atlas = &self.res.atlas;
        if self.debug {
            let p = self
                .player
                .as_ref()
                .map(|p| p.position)
                .unwrap_or(Vec3::ZERO);
            let block = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
            let biome = self
                .world
                .chunks
                .get(&(block[0] >> 4, block[2] >> 4))
                .map(|c| {
                    self.res
                        .biomes
                        .get(c.biome(block[0], block[1], block[2]))
                        .name
                        .clone()
                })
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
                format!(
                    "C: {} chunks, {} sections drawn / {}",
                    self.world.chunks.len(),
                    self.renderer.as_ref().map_or(0, |r| r.drawn_sections),
                    self.renderer.as_ref().map_or(0, |r| r.section_count())
                ),
                format!("Pending jobs: {}", self.streaming.pending()),
                String::new(),
                format!("XYZ: {:.3} / {:.5} / {:.3}", p.x, p.y, p.z),
                format!("Block: {} {} {}", block[0], block[1], block[2]),
                format!(
                    "Chunk: {} {} {} in {} {} {}",
                    block[0] & 15,
                    block[1] & 15,
                    block[2] & 15,
                    block[0] >> 4,
                    block[1] >> 4,
                    block[2] >> 4
                ),
                format!(
                    "Facing: {facing} ({:.1} / {:.1})",
                    self.yaw.rem_euclid(360.0)
                        - if self.yaw.rem_euclid(360.0) > 180.0 {
                            360.0
                        } else {
                            0.0
                        },
                    self.pitch
                ),
                format!(
                    "Light: {} (sky {}, block {})",
                    (light >> 4).max(light & 15),
                    light >> 4,
                    light & 15
                ),
                format!("Biome: {biome}"),
                format!(
                    "Day {} time {}",
                    self.world.day_time / 24000,
                    self.world.day_time % 24000
                ),
            ];
            for (i, line) in lines.iter().enumerate() {
                if line.is_empty() {
                    continue;
                }
                let tw = HudBatch::text_width(atlas, line);
                batch.fill(
                    atlas,
                    1.0,
                    2.0 + i as f32 * 9.0 - 1.0,
                    tw + 2.0,
                    9.0,
                    rgba(80, 80, 80, 144),
                );
                batch.text(
                    atlas,
                    line,
                    2.0,
                    2.0 + i as f32 * 9.0,
                    rgba(224, 224, 224, 255),
                    false,
                );
            }
        }
        if !self.ui.hide_gui
            && let Some((text, at)) = &self.message
            && at.elapsed() < Duration::from_secs(3)
        {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, h - 59.0, u32::MAX, true);
        }
        self.draw_chat(&mut batch);
        self.draw_tab_list(&mut batch);
        match self.screen {
            Screen::Paused if self.pause_menu => self.draw_pause_menu(&mut batch),
            // PauseScreen(false): only its title, at the top.
            Screen::Paused => {
                let title = self.translate("menu.paused", "Game Paused");
                let tw = HudBatch::text_width(&self.res.atlas, &title);
                batch.text(
                    &self.res.atlas,
                    &title,
                    ((batch.width / 2.0).floor() - (tw / 2.0).floor()).floor(),
                    10.0,
                    u32::MAX,
                    true,
                );
            }
            Screen::Dead => self.draw_death_screen(&mut batch),
            Screen::Stats => self.draw_stats(&mut batch),
            Screen::Advancements => self.draw_advancements(&mut batch),
            Screen::WorldOptions => self.draw_world_options(&mut batch),
            Screen::Inventory => {
                self.draw_inventory(&mut batch);
            }
            Screen::Creative => {
                self.draw_creative(&mut batch);
            }
            Screen::Chat => self.draw_chat_screen(&mut batch),
            Screen::Playing | Screen::Options => {}
        }
        self.draw_toasts(&mut batch);
        self.remote_hud(&mut batch);
        self.draw_sign_editor(&mut batch);
        self.draw_book(&mut batch);
        batch
    }
    fn inventory_origin(&self, batch: &HudBatch) -> (f32, f32) {
        let (x, y) = batch.centered_origin(176.0, self.screen_height());
        (x + self.recipe_book_shift(batch.width), y)
    }
    /// Slot ids: 0..36 player inventory, 100..109 crafting grid, 200 result.
    fn container_kind(&self) -> Option<&BlockEntity> {
        self.container
            .and_then(|k| self.world.block_entities.get(&k))
    }
    /// 300.. are the open container's slots.
    fn slots(&self) -> Vec<usize> {
        if let Some(v) = self.station_slots().or_else(|| self.mount_slots()) {
            return v;
        }
        let mut v: Vec<usize> = (0..36).collect();
        match self.container_kind() {
            Some(BlockEntity::Chest(_)) => v.extend((0..self.container_size()).map(|i| 300 + i)),
            Some(BlockEntity::Furnace(_)) => v.extend(300..303),
            Some(BlockEntity::Brewing(_)) => v.extend(300..305),
            Some(BlockEntity::Crafter(_)) => v.extend(300..310),
            Some(
                BlockEntity::Sign(_)
                | BlockEntity::Jukebox(_)
                | BlockEntity::Beacon(_)
                | BlockEntity::Lectern(_)
                | BlockEntity::Sculk(_)
                | BlockEntity::Banner(_)
                | BlockEntity::CreakingHeart(_),
            ) => {}
            None => {
                v.extend((0..self.craft_size * self.craft_size).map(|i| 100 + i));
                v.push(200);
                if self.craft_size == 2 {
                    v.extend(500..505);
                }
            }
        }
        v
    }
    /// InventoryMenu / CraftingMenu slot layouts.
    fn slot_position(&self, i: usize) -> (f32, f32) {
        if let Some(p) = self
            .station_slot_position(i)
            .or_else(|| self.machine_slot_position(i))
            .or_else(|| self.crafter_slot_position(i))
            .or_else(|| self.mount_slot_position(i))
        {
            return p;
        }
        let table = self.craft_size == 3;
        // ContainerScreen (3 rows) shifts the player inventory down by one pixel.
        // ChestMenu: the inventory sits `18 * (rows - 3) + 1` lower than the
        // 3-row layout of the other menus.
        let shift = match self.chest_rows() {
            Some(rows) => 1.0 + (rows as f32 - 3.0) * 18.0,
            None => 0.0,
        };
        match i {
            0..9 => (8.0 + i as f32 * 18.0, 142.0 + shift),
            9..36 => (
                8.0 + ((i - 9) % 9) as f32 * 18.0,
                84.0 + shift + ((i - 9) / 9) as f32 * 18.0,
            ),
            500..504 => (8.0, 8.0 + (i - 500) as f32 * 18.0),
            // InventoryMenu: the shield slot.
            504 => (77.0, 62.0),
            300..500 => match self.container_kind() {
                Some(BlockEntity::Furnace(_)) => {
                    [(56.0, 17.0), (56.0, 53.0), (116.0, 35.0)][(i - 300).min(2)]
                }
                _ => (
                    8.0 + ((i - 300) % 9) as f32 * 18.0,
                    18.0 + ((i - 300) / 9) as f32 * 18.0,
                ),
            },
            200 => {
                if table {
                    (124.0, 35.0)
                } else {
                    (154.0, 28.0)
                }
            }
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
        if i >= mount_menu::SADDLE {
            return self.mount_slot_mut(i);
        }
        if i >= stations::INPUT {
            return self.station_slot_mut(i);
        }
        if (500..504).contains(&i) {
            return &mut self.armor[i - 500];
        }
        if i == 504 {
            return &mut self.offhand;
        }
        if i >= 300 {
            return self.container_slot_mut(i - 300);
        }
        if (100..109).contains(&i) {
            &mut self.craft_grid[i - 100]
        } else {
            &mut self.inventory[i]
        }
    }
    /// What a slot shows: the crafting result slots are computed.
    fn slot_view(&mut self, i: usize) -> Option<ItemStack> {
        match i {
            200 => self.craft_result(),
            309 => self.crafter_result(),
            _ => self.slot_mut(i).clone(),
        }
    }
    fn craft_result(&self) -> Option<ItemStack> {
        self.craft_recipe().map(|(_, result)| result)
    }
    /// The recipe id and result of the crafting grid.
    /// `RecipeCraftingHolder.awardUsedRecipes`: a crafted recipe becomes known
    /// (special ones never do).
    fn award_crafted(&mut self, recipes: &[String]) {
        let index = &self.res.stations.recipe_index;
        let (_, unlocked) = self.level.recipe_book.add_recipes(index, recipes);
        self.recipe_unlocked(&unlocked);
    }
    fn craft_recipe(&self) -> Option<(String, ItemStack)> {
        let n = self.craft_size;
        let grid: Vec<Option<ItemStack>> = (0..n * n).map(|i| self.craft_grid[i].clone()).collect();
        let scale = |id: i32| self.maps.data.get(&id).map(|m| m.scale);
        self.res
            .recipes
            .find_with(&grid, n, &scale)
            .map(|(name, result)| (name.to_owned(), result))
    }
    fn draw_inventory(&mut self, batch: &mut HudBatch) {
        if self.draw_station(batch) {
            return;
        }
        let (ox, oy) = self.inventory_origin(batch);
        {
            let atlas = &self.res.atlas;
            batch.fill(
                atlas,
                0.0,
                0.0,
                batch.width,
                batch.height,
                rgba(16, 16, 16, 160),
            );
            let part = |batch: &mut HudBatch,
                        name: &str,
                        sx: f32,
                        sy: f32,
                        w: f32,
                        h: f32,
                        dx: f32,
                        dy: f32| {
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
            match self
                .container
                .and_then(|k| self.world.block_entities.get(&k))
            {
                Some(BlockEntity::Chest(_)) if self.draw_machine_screen(batch, ox, oy) => {}
                Some(BlockEntity::Chest(_)) => self.draw_chest_screen(batch, ox, oy),
                Some(BlockEntity::Furnace(f)) => {
                    let (tex, title) = match f.kind {
                        horizon_client::containers::FurnaceKind::Furnace => {
                            ("gui/container/furnace", "Furnace")
                        }
                        horizon_client::containers::FurnaceKind::BlastFurnace => {
                            ("gui/container/blast_furnace", "Blast Furnace")
                        }
                        horizon_client::containers::FurnaceKind::Smoker => {
                            ("gui/container/smoker", "Smoker")
                        }
                    };
                    part(batch, tex, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    let title = &self.container_title(title.to_owned());
                    let tw = HudBatch::text_width(atlas, title);
                    batch.text(atlas, title, ox + (176.0 - tw) / 2.0, oy + 6.0, grey, false);
                    batch.text(atlas, "Inventory", ox + 8.0, oy + 72.0, grey, false);
                    if f.lit_remaining > 0 {
                        // AbstractFurnaceScreen: flame height ceil(lit * 13) + 1.
                        let total = if f.lit_total == 0 { 200 } else { f.lit_total };
                        let h = ((f.lit_remaining as f32 / total as f32) * 13.0).ceil() + 1.0;
                        part(
                            batch,
                            "gui/sprites/container/furnace/lit_progress",
                            0.0,
                            14.0 - h,
                            14.0,
                            h,
                            ox + 56.0,
                            oy + 36.0 + 14.0 - h,
                        );
                    }
                    if f.cooking_timer > 0 && f.cooking_total > 0 {
                        let w = ((f.cooking_timer as f32 / f.cooking_total as f32) * 24.0).ceil();
                        part(
                            batch,
                            "gui/sprites/container/furnace/burn_progress",
                            0.0,
                            0.0,
                            w,
                            16.0,
                            ox + 79.0,
                            oy + 34.0,
                        );
                    }
                }
                Some(BlockEntity::Brewing(_)) => {}
                Some(BlockEntity::Crafter(_)) => self.draw_crafter_screen(batch, ox, oy),
                Some(
                    BlockEntity::Sign(_)
                    | BlockEntity::Jukebox(_)
                    | BlockEntity::Beacon(_)
                    | BlockEntity::Lectern(_)
                    | BlockEntity::Sculk(_)
                    | BlockEntity::Banner(_)
                    | BlockEntity::CreakingHeart(_),
                ) => {}
                None if self.mount_active() => self.draw_mount_background(batch, ox, oy),
                None => {
                    let texture = if self.craft_size == 3 {
                        "gui/container/crafting_table"
                    } else {
                        "gui/container/inventory"
                    };
                    part(batch, texture, 0.0, 0.0, 176.0, 166.0, ox, oy);
                    let (title, tx) = if self.craft_size == 3 {
                        ("Crafting", 29.0)
                    } else {
                        ("Crafting", 97.0)
                    };
                    batch.text(
                        atlas,
                        title,
                        ox + tx,
                        oy + if self.craft_size == 3 { 6.0 } else { 8.0 },
                        grey,
                        false,
                    );
                    if self.craft_size == 3 {
                        batch.text(atlas, "Inventory", ox + 8.0, oy + 72.0, grey, false);
                    }
                }
            }
        }
        if self.container.is_none() && self.craft_size != 3 && !self.mount_active() {
            self.draw_inventory_player(batch, ox, oy);
        }
        for i in self.slots() {
            let (sx, sy) = self.slot_position(i);
            if i == 504 && self.offhand.is_none() {
                batch.sprite(
                    &self.res.atlas,
                    "gui/sprites/container/slot/shield",
                    ox + sx,
                    oy + sy,
                );
            }
            if (500..504).contains(&i) && self.armor[i - 500].is_none() {
                let icon = ["helmet", "chestplate", "leggings", "boots"][i - 500];
                batch.sprite(
                    &self.res.atlas,
                    &format!("gui/sprites/container/slot/{icon}"),
                    ox + sx,
                    oy + sy,
                );
            }
            let stack = self.slot_view(i);
            if let Some(stack) = stack {
                self.draw_item(batch, &stack, ox + sx, oy + sy);
            }
        }
        self.draw_recipe_book(batch);
        if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
            let (sx, sy) = self.slot_position(slot);
            batch.fill(
                &self.res.atlas,
                ox + sx,
                oy + sy,
                16.0,
                16.0,
                rgba(255, 255, 255, 128),
            );
        }
        if let Some(stack) = self.carried.clone() {
            self.draw_item(batch, &stack, self.cursor.0 - 8.0, self.cursor.1 - 8.0);
        }
        self.draw_slot_tooltip(batch);
    }
    pub(super) fn close_container(&mut self) {
        self.close_station();
        self.mount_closed();
        self.recipe_ui.reset_screen();
        if self.remote.is_some() {
            return self.remote_close_container();
        }
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
            self.container_opener_event(key, false);
            self.post_block_game_event(
                "minecraft:container_close",
                [key.0, key.1, key.2],
                Some(horizon_client::mobs::Who::Player),
                None,
            );
            let name = self
                .res
                .facts
                .get(self.world.block(key.0, key.1, key.2))
                .name
                .clone();
            let sound = if storage::is_shulker_box(&name) {
                Some("minecraft:block.shulker_box.close")
            } else if &*name == "minecraft:ender_chest" {
                Some("minecraft:block.ender_chest.close")
            } else if name.contains("chest") {
                Some("minecraft:block.chest.close")
            } else if &*name == "minecraft:barrel" {
                Some("minecraft:block.barrel.close")
            } else {
                None
            };
            if let Some(sound) = sound {
                self.audio.play(
                    sound,
                    Some([
                        f64::from(key.0) + 0.5,
                        f64::from(key.1) + 0.5,
                        f64::from(key.2) + 0.5,
                    ]),
                    0.5,
                    1.0,
                );
            }
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
        self.close_ender_chest();
        self.close_cart_container();
    }
    /// `quickMoveStack` of the open plain menu for a shift-click.
    fn inventory_quick_move(&mut self, slot: usize) -> bool {
        self.menu_click(Some(slot), 0, horizon_client::menu::Input::QuickMove)
    }
    /// `AbstractContainerMenu.clicked` on the open inventory, crafting table,
    /// chest, hopper, dispenser or furnace menu, run by the verified menu
    /// engine (`horizon_client::menu`). `slot` None is outside the window.
    /// False when the open screen is not one the engine models.
    fn menu_click(
        &mut self,
        slot: Option<usize>,
        button: i32,
        input: horizon_client::menu::Input,
    ) -> bool {
        self.menu_ops(&[(slot, button, input)])
    }
    /// Several clicks on one menu instance (a quick craft drag is three).
    fn menu_ops(&mut self, ops: &[(Option<usize>, i32, horizon_client::menu::Input)]) -> bool {
        use horizon_client::menu::Input;
        if let [(Some(slot), button, input)] = ops {
            self.crafter_slot_clicked(*slot, *button, *input);
        }
        let ops = ops.to_vec();
        self.menu_session(move |menu, env, to_menu| {
            let mut clicks = Vec::new();
            for &(slot, button, input) in &ops {
                let menu_slot = match slot {
                    Some(g) => {
                        if g == 200 && input != Input::QuickMove {
                            return None;
                        }
                        match to_menu(g) {
                            Some(i) => i as i32,
                            None => return None,
                        }
                    }
                    None => -999,
                };
                clicks.push((menu_slot, button, input));
            }
            for (menu_slot, button, input) in clicks {
                if menu.click(env, menu_slot, button, input).is_err() {
                    // Vanilla's server throws: nothing of this click is kept.
                    return Some(false);
                }
            }
            Some(true)
        })
    }
    /// The open menu as the verified engine (`horizon_client::menu`): the
    /// game's slots are copied in, `run` acts on it, and the slots, carried
    /// stack, menu data and drops are copied back when it returns Some(true)
    /// (Some(false): handled, nothing kept; None: not handled). False when the
    /// open screen is not one the engine models or `run` did not handle it.
    pub(super) fn menu_session(
        &mut self,
        run: impl FnOnce(
            &mut horizon_client::menu::Menu,
            &horizon_client::menu::Env,
            &dyn Fn(usize) -> Option<usize>,
        ) -> Option<bool>,
    ) -> bool {
        use horizon_client::menu::{Env, Kind, Menu};
        #[derive(Clone, Copy)]
        struct Map {
            container: usize,
            table: bool,
            cells: usize,
            plain: bool,
        }
        impl Map {
            fn main(&self) -> usize {
                match (self.plain, self.table) {
                    (false, _) => self.container,
                    (true, true) => 10,
                    (true, false) => 9,
                }
            }
            fn to_menu(self, g: usize) -> Option<usize> {
                let main = self.main();
                if self.plain {
                    match g {
                        200 => Some(0),
                        100..109 if g - 100 < self.cells => Some(1 + g - 100),
                        500..504 if !self.table => Some(5 + g - 500),
                        504 if !self.table => Some(45),
                        9..36 => Some(main + g - 9),
                        0..9 => Some(main + 27 + g),
                        _ => None,
                    }
                } else {
                    match g {
                        300.. if g < 300 + self.container => Some(g - 300),
                        9..36 => Some(main + g - 9),
                        0..9 => Some(main + 27 + g),
                        _ => None,
                    }
                }
            }
            fn to_game(self, i: usize) -> Option<usize> {
                let main = self.main();
                if self.plain {
                    match i {
                        0 => None,
                        1..=9 if i - 1 < self.cells => Some(100 + i - 1),
                        5..9 if !self.table => Some(500 + i - 5),
                        45 if !self.table => Some(504),
                        _ if i >= main && i < main + 27 => Some(9 + i - main),
                        _ if i >= main + 27 && i < main + 36 => Some(i - main - 27),
                        _ => None,
                    }
                } else if i < self.container {
                    Some(300 + i)
                } else if i < self.container + 27 {
                    Some(9 + i - self.container)
                } else {
                    Some(i - self.container - 27)
                }
            }
        }
        // Menu slot -> game slot, for every menu slot.
        let mut plain_map = None;
        let crafter_open = matches!(self.container_kind(), Some(BlockEntity::Crafter(_)));
        let loom_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Loom);
        let merchant_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Merchant);
        let (kind, game_of): (Kind, Vec<Option<usize>>) = if merchant_open {
            // MerchantMenu: two payment slots, the result, then the inventory.
            let mut v: Vec<Option<usize>> = vec![
                Some(stations::INPUT),
                Some(stations::INPUT + 1),
                Some(stations::RESULT),
            ];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Merchant, v)
        } else if crafter_open {
            // CrafterMenu: the grid, the inventory, then the result.
            let mut v: Vec<Option<usize>> = (300..309).map(Some).collect();
            v.extend((9..36).chain(0..9).map(Some));
            v.push(None);
            (Kind::CrafterBlock, v)
        } else if loom_open {
            // LoomMenu: banner, dye, pattern, result, then the inventory.
            let mut v: Vec<Option<usize>> = (0..3).map(|i| Some(stations::INPUT + i)).collect();
            v.push(Some(stations::RESULT));
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Loom, v)
        } else {
            let (kind, map) = if self.container.is_some() {
                let size = self.container_size();
                let (kind, count) = match self.container_kind() {
                    Some(BlockEntity::Chest(_)) => {
                        let shulker = self.container.is_some_and(|k| {
                            let id = self.world.block(k.0, k.1, k.2);
                            storage::is_shulker_box(&self.res.facts.get(id).name)
                        });
                        let kind = match size {
                            5 => Kind::Hopper,
                            9 => Kind::Dispenser,
                            27 if shulker => Kind::Shulker,
                            n => Kind::Chest((n / 9).clamp(1, 6)),
                        };
                        (kind, size)
                    }
                    Some(BlockEntity::Furnace(f)) => (
                        Kind::Furnace(
                            match f.kind.recipe_type().trim_start_matches("minecraft:") {
                                "blasting" => "blasting",
                                "smoking" => "smoking",
                                _ => "smelting",
                            },
                        ),
                        3,
                    ),
                    _ => return false,
                };
                let map = Map {
                    container: count,
                    table: false,
                    cells: 0,
                    plain: false,
                };
                (kind, map)
            } else if self.station.is_none() && !self.mount_active() && self.remote.is_none() {
                let table = self.craft_size == 3;
                let map = Map {
                    container: 0,
                    table,
                    cells: self.craft_size * self.craft_size,
                    plain: true,
                };
                (
                    if table {
                        Kind::Crafting
                    } else {
                        Kind::Inventory
                    },
                    map,
                )
            } else {
                return false;
            };
            let total = Menu::new(kind).slot_count();
            plain_map = Some(map);
            (kind, (0..total).map(|i| map.to_game(i)).collect())
        };
        // The plain map also takes the crafting result slot (a shift-click on it).
        let to_menu = |g: usize| match plain_map {
            Some(map) => map.to_menu(g),
            None => game_of.iter().position(|x| *x == Some(g)),
        };
        let mut menu = Menu::new(kind);
        let total = menu.slot_count();
        let before: Vec<Option<ItemStack>> = game_of
            .iter()
            .map(|g| g.and_then(|g| self.slot_mut(g).clone()))
            .collect();
        let carried = self.carried.clone();
        let creative = self.creative;
        let station_menu = self.station.as_ref().map(|s| {
            (
                s.loom.selected.map_or(-1, |i| i as i32),
                s.loom.patterns.clone(),
            )
        });
        let crafter_data = match self.container_kind() {
            Some(BlockEntity::Crafter(c)) => Some((c.disabled, c.triggered)),
            _ => None,
        };
        // The merchant's session, lent to the menu for this click.
        let mut session = self
            .station
            .as_ref()
            .and_then(|s| s.merchant.as_ref())
            .map(|m| (m.session.clone(), m.mob));
        if let Some((s, mob)) = session.as_mut() {
            self.merchant_sync_in(s, *mob);
        }
        let next_map = if merchant_open { self.peek_map_id() } else { 0 };
        let tag = |t: &str| self.structure_tag(t);
        let maps = trading::GameMaps {
            terrain: self.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let trade_env = horizon_client::merchant::TradeEnv {
            trades: &self.res.stations.trades,
            enchantments: &self.res.stations.enchantments,
            maps: Some(&maps),
        };
        let env = Env {
            items: &self.res.items,
            recipes: &self.res.recipes,
            brewing: &self.res.stations.brewing,
            banners: &self.res.stations.banners,
            trade: Some(&trade_env),
        };
        menu.creative = creative;
        menu.selected = self.selected;
        for (i, stack) in before.iter().enumerate() {
            menu.set_slot(&env, i, stack.clone());
        }
        // The off hand is part of the inventory even when the menu has no slot for it.
        let offhand_mapped = game_of.contains(&Some(504));
        if !offhand_mapped {
            menu.set_player_item(40, self.offhand.clone());
        }
        menu.settle(&env);
        if kind == Kind::Loom
            && let Some((selected, patterns)) = station_menu
        {
            // The menu's state between clicks: what set_slot recomputed is replaced.
            menu.data[0] = selected;
            menu.loom_patterns = patterns;
            menu.set_slot(&env, 3, before[3].clone());
        }
        if let Some((disabled, triggered)) = crafter_data {
            for (i, d) in disabled.iter().enumerate() {
                menu.data[i] = i32::from(*d);
            }
            menu.data[9] = i32::from(triggered);
        }
        menu.carried = carried;
        menu.drops.clear();
        if kind == Kind::Merchant {
            // Attached after the slots were set: setting them is not a click.
            menu.merchant = session.map(|(s, _)| s);
        }
        let outcome = run(&mut menu, &env, &to_menu);
        let made = maps.made.take();
        match outcome {
            None => return false,
            Some(false) => return true,
            Some(true) => {}
        }
        self.register_exploration_maps(made);
        if let Some(s) = menu.merchant.take() {
            self.merchant_sync_out(s);
        }
        let after: Vec<Option<ItemStack>> = (0..total).map(|i| menu.slot(i).cloned()).collect();
        let carried = menu.carried.take();
        let dropped = std::mem::take(&mut menu.dropped);
        let sounds = std::mem::take(&mut menu.sounds);
        let crafted = std::mem::take(&mut menu.crafted);
        for (i, stack) in after.into_iter().enumerate() {
            if let Some(g) = game_of[i] {
                *self.slot_mut(g) = stack;
            }
        }
        self.carried = carried;
        if !offhand_mapped {
            self.offhand = menu.player_item(40).cloned();
        }
        for stack in dropped {
            self.player_drop(stack);
        }
        self.award_crafted(&crafted);
        if kind == Kind::Loom {
            if let Some(s) = self.station.as_mut() {
                s.loom.selected = usize::try_from(menu.data[0]).ok();
                s.loom.patterns = std::mem::take(&mut menu.loom_patterns);
            }
            for sound in sounds {
                self.loom_sound(sound);
            }
            self.loom_screen_changed();
        }
        if kind == Kind::CrafterBlock
            && let Some(BlockEntity::Crafter(c)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            for (i, d) in c.disabled.iter_mut().enumerate() {
                *d = menu.data[i] == 1;
            }
        }
        if matches!(kind, Kind::Furnace(_))
            && let Some(BlockEntity::Furnace(f)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            // AbstractFurnaceBlockEntity.setItem: only a new input restarts cooking.
            f.input_clicked(&self.res.recipes, before[0].clone());
        }
        self.apply_equipment();
        true
    }
    /// Number keys, F and Q over a slot of the open menu: SWAP with a hotbar
    /// slot or the off hand, THROW one or the whole stack.
    fn inventory_hotkey(&mut self, key: KeyCode) -> bool {
        use horizon_client::menu::Input;
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = HudBatch::new(size);
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return false;
        };
        let digit = [
            KeyCode::Digit1,
            KeyCode::Digit2,
            KeyCode::Digit3,
            KeyCode::Digit4,
            KeyCode::Digit5,
            KeyCode::Digit6,
            KeyCode::Digit7,
            KeyCode::Digit8,
            KeyCode::Digit9,
        ]
        .iter()
        .position(|k| *k == key);
        let done = match (digit, key) {
            (Some(n), _) => self.menu_click(Some(slot), n as i32, Input::Swap),
            (None, KeyCode::KeyF) => self.menu_click(Some(slot), 40, Input::Swap),
            (None, KeyCode::KeyQ) => {
                let all = self.keys.contains(&KeyCode::ControlLeft);
                self.menu_click(Some(slot), i32::from(all), Input::Throw)
            }
            _ => false,
        };
        if done && let Some(key) = self.container {
            self.container_changed(key);
        }
        done
    }
    /// The plain inventory mouse press: starts a quick craft drag when a
    /// stack is held, gathers on a double click, else the normal click.
    fn inventory_press(&mut self, right: bool) {
        use horizon_client::container_screen::Target;
        use horizon_client::menu::Input;
        let (slot, outside) = self.inventory_target();
        let shift = self.has_shift_down();
        let target = Target::of(slot, outside);
        let input = self.inv_clicks.press(
            &target,
            Self::screen_button(right),
            self.carried.is_none(),
            shift,
            false,
        );
        if outside {
            // slotId -999: an empty hand throws nothing; a carried stack
            // starts a drag whose release drops it.
            match input {
                Some(input) => self.throw_click(right, input),
                None if self.inv_clicks.quick_crafting().is_some() => {
                    self.inv_drag = Some((i32::from(right), Vec::new()));
                }
                None => {}
            }
            return;
        }
        if let Some(slot) = slot
            && !shift
            && slot != 200
        {
            let now = std::time::Instant::now();
            let double = !right
                && self.carried.is_some()
                && self.last_inv_click.is_some_and(|(at, last, was_right)| {
                    last == slot && !was_right && now.duration_since(at).as_millis() < 250
                });
            self.last_inv_click = Some((now, slot, right));
            if double && self.menu_click(Some(slot), 0, Input::PickupAll) {
                self.last_inv_click = None;
                // The double click's own release (`mouseReleased`'s PICKUP_ALL).
                self.inv_clicks.skip_release();
                return;
            }
            if self.carried.is_some() {
                self.inv_drag = Some((i32::from(right), vec![slot]));
                return;
            }
        }
        self.click_inventory(right);
    }
    /// Mouse movement while a quick craft drag is running.
    fn inventory_drag_move(&mut self) {
        let Some((_, slots)) = self.inv_drag.as_ref() else {
            return;
        };
        let Some(r) = self.renderer.as_ref() else {
            return;
        };
        let batch = HudBatch::new([r.config.width, r.config.height]);
        if let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1)
            && slot != 200
            && !slots.contains(&slot)
            && let Some((_, slots)) = self.inv_drag.as_mut()
        {
            slots.push(slot);
        }
    }
    /// The hovered slot and whether the cursor is outside the window.
    fn inventory_target(&self) -> (Option<usize>, bool) {
        let Some(r) = self.renderer.as_ref() else {
            return (None, false);
        };
        let batch = HudBatch::new([r.config.width, r.config.height]);
        (
            self.slot_at(&batch, self.cursor.0, self.cursor.1),
            self.clicked_outside(&batch),
        )
    }
    fn screen_button(right: bool) -> horizon_client::container_screen::Button {
        use horizon_client::container_screen::Button;
        if right { Button::Right } else { Button::Left }
    }
    /// `Screen.hasShiftDown` / `InputWithModifiers.hasShiftDown`: the
    /// physical Shift keys, whatever the sneak key is bound or toggled to.
    pub(super) fn has_shift_down(&self) -> bool {
        self.physical_keys.contains(&KeyCode::ShiftLeft)
            || self.physical_keys.contains(&KeyCode::ShiftRight)
    }
    /// `hasClickedOutside` of the open screen at the cursor.
    fn clicked_outside(&self, batch: &HudBatch) -> bool {
        use horizon_client::container_screen::{Book, Image, clicked_outside};
        let (ox, oy) = self.inventory_origin(batch);
        let (shift, width) = match self.station.as_ref().map(|s| s.kind) {
            Some(stations::Kind::Merchant) => (-50.0, 276),
            Some(stations::Kind::Beacon) => (beacon::SHIFT, 230),
            _ => (0.0, 176),
        };
        let image = Image {
            left: (ox + shift) as i32,
            top: oy as i32,
            width,
            height: self.screen_height() as i32,
        };
        let book = self
            .recipe_book_area(batch)
            .map(|(visible, tab_hovered)| Book {
                visible,
                selected_tab_hovered: tab_hovered,
            });
        clicked_outside(
            f64::from(self.cursor.0),
            f64::from(self.cursor.1),
            image,
            book,
        )
    }
    /// A click outside the window (slot id -999): `THROW` does nothing; a
    /// `PICKUP` drops the carried stack (left) or one of it (right), thrown
    /// from the hand (`AbstractContainerMenu.doClick`, `LivingEntity.drop`).
    fn throw_click(&mut self, right: bool, input: horizon_client::menu::Input) {
        use horizon_client::menu::Input;
        let button = i32::from(right);
        if self.remote.is_some() {
            self.remote_throw(right, input);
            return;
        }
        let before = self.carried.clone();
        if !self.menu_click(None, button, input) && input == Input::Pickup {
            // A screen the menu engine doesn't model: the same doClick.
            if let Some(c) = self.carried.as_mut() {
                let thrown = if right {
                    let one = ItemStack {
                        count: 1,
                        ..c.clone()
                    };
                    c.count -= 1;
                    if c.count == 0 {
                        self.carried = None;
                    }
                    one
                } else {
                    self.carried.take().expect("some")
                };
                self.player_drop(thrown);
            }
        }
        if self.carried != before {
            self.swing_arm();
        }
        if let Some(key) = self.container {
            self.container_changed(key);
        }
    }
    /// Mouse release over an inventory screen: `mouseReleased` ends or
    /// cancels the drag, drops a stack released outside, or clicks.
    fn inventory_mouse_release(&mut self, right: bool) {
        use horizon_client::container_screen::{Release, Target};
        let (slot, outside) = self.inventory_target();
        let dragged = self.inv_drag.as_ref().is_some_and(|(_, s)| !s.is_empty());
        let release = self.inv_clicks.release(
            &Target::of(slot, outside),
            Self::screen_button(right),
            self.carried.is_none(),
            dragged,
            self.has_shift_down(),
            false,
        );
        match (release, Target::of(slot, outside)) {
            (Release::QuickCraft, _) => self.inventory_release(),
            (Release::Click(input), Target::Outside) => {
                self.inv_drag = None;
                self.throw_click(right, input);
            }
            (Release::Click(input), Target::Slot(slot)) if self.inv_drag.is_some() => {
                // The drag never reached a slot: a plain click where it ends.
                self.inv_drag = None;
                if !self.menu_click(Some(slot), i32::from(right), input) {
                    self.click_inventory(right);
                }
                if let Some(key) = self.container {
                    self.container_changed(key);
                }
            }
            _ => self.inv_drag = None,
        }
    }
    /// Mouse release: finishes a quick craft drag (one slot is a plain click).
    fn inventory_release(&mut self) {
        use horizon_client::menu::Input;
        let Some((ty, slots)) = self.inv_drag.take() else {
            return;
        };
        if slots.len() == 1 {
            if !self.menu_click(Some(slots[0]), ty, Input::Pickup) {
                self.click_inventory(ty == 1);
            }
        } else {
            let mask = |header: i32| header | (ty << 2);
            let mut ops = vec![(None, mask(0), Input::QuickCraft)];
            ops.extend(
                slots
                    .into_iter()
                    .map(|slot| (Some(slot), mask(1), Input::QuickCraft)),
            );
            ops.push((None, mask(2), Input::QuickCraft));
            if !self.menu_ops(&ops) {
                // Not a menu the engine models: the drag is a plain click.
                self.click_inventory(ty == 1);
            }
        }
        if let Some(key) = self.container {
            self.container_changed(key);
        }
    }
    fn click_inventory(&mut self, right: bool) {
        if self.mount_click(right) {
            return;
        }
        if self.recipe_book_click(right) {
            return;
        }
        if self.station_click(right) {
            return;
        }
        if self.remote.is_some() {
            return self.remote_click(right);
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let batch = HudBatch::new(size);
        let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1) else {
            return;
        };
        let held_shift = self.has_shift_down();
        if self.crafter_click(slot, held_shift, right) {
            return;
        }
        if held_shift && self.container.is_some() && self.container_quick_move(slot) {
            if let Some(key) = self.container {
                self.world.modified.insert((key.0 >> 4, key.2 >> 4));
            }
            return;
        }
        if let Some(c) = &self.carried
            && self.refuses(slot, c)
        {
            return;
        }
        let max = |items: &Items, s: &ItemStack| items.get(&s.item).map_or(64, |i| i.max_stack);
        let shift = held_shift;
        if slot >= 300
            && let Some(key) = self.container
        {
            self.world.modified.insert((key.0 >> 4, key.2 >> 4));
        }
        let is_furnace = matches!(self.container_kind(), Some(BlockEntity::Furnace(_)));
        if is_furnace && slot == 302 {
            // FurnaceResultSlot: take only.
            let Some(result) = self.slot_mut(302).take() else {
                return;
            };
            match &mut self.carried {
                None => self.carried = Some(result),
                Some(c)
                    if c.item == result.item
                        && c.count + result.count <= max(&self.res.items, c) =>
                {
                    c.count += result.count
                }
                _ => *self.slot_mut(302) = Some(result),
            }
            return;
        }
        // Changing the input restarts cooking (AbstractFurnaceBlockEntity.setItem).
        let furnace_input = (is_furnace && slot == 300).then(|| self.slot_mut(300).clone());
        if slot == 200 {
            // ResultSlot: take the crafted stack and consume one of each ingredient.
            let mut crafted = 0;
            while let Some((recipe, result)) = self.craft_recipe() {
                let made = (result.item.clone(), result.count);
                let cells = self.craft_size * self.craft_size;
                let remainders = self.res.recipes.remainders(
                    &recipe,
                    &self.craft_grid[..cells],
                    self.craft_size,
                );
                if shift {
                    if self.give(result).is_some() {
                        break;
                    }
                } else {
                    match &mut self.carried {
                        None => self.carried = Some(result),
                        Some(c)
                            if c.item == result.item
                                && c.count + result.count <= max(&self.res.items, c) =>
                        {
                            c.count += result.count
                        }
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
                for (i, rest) in remainders.into_iter().enumerate() {
                    let Some(rest) = rest else { continue };
                    match &mut self.craft_grid[i] {
                        cell @ None => *cell = Some(rest),
                        Some(c) if c.same_item_same_components(&rest) => c.count += rest.count,
                        _ => self.give_or_drop(rest),
                    }
                }
                crafted += 1;
                self.stats.add("crafted", &made.0, made.1 as i32);
                self.award_crafted(&[format!("minecraft:{recipe}")]);
                if !shift || crafted >= 64 {
                    break;
                }
            }
            return;
        }
        if shift
            && (slot < 36 || (500..504).contains(&slot) || (100..109).contains(&slot))
            && self.inventory_quick_move(slot)
        {
            return;
        }
        if (500..504).contains(&slot) {
            let wanted = ["head", "chest", "legs", "feet"][slot - 500];
            let fits = self.carried.as_ref().is_none_or(|c| {
                self.res
                    .items
                    .get(&c.item)
                    .and_then(|i| i.equip_slot.as_deref())
                    == Some(wanted)
            });
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
                let taken = ItemStack {
                    count: half,
                    ..s.clone()
                };
                ((s.count > 0).then_some(s), Some(taken))
            }
            (None, s) => (None, s),
            (Some(mut c), None) if right => {
                let one = ItemStack {
                    count: 1,
                    ..c.clone()
                };
                c.count -= 1;
                (Some(one), (c.count > 0).then_some(c))
            }
            (Some(c), None) => (Some(c), None),
            (Some(mut c), Some(mut s)) if c.same_item_same_components(&s) => {
                let room = max(items, &s).saturating_sub(s.count);
                let moved = if right {
                    1.min(room).min(c.count)
                } else {
                    room.min(c.count)
                };
                s.count += moved;
                c.count -= moved;
                (Some(s), (c.count > 0).then_some(c))
            }
            (Some(c), Some(s)) => (Some(c), Some(s)),
        };
        self.carried = new_carried;
        *self.slot_mut(slot) = new_slot;
        if let Some(before) = furnace_input
            && let Some(BlockEntity::Furnace(f)) = self
                .container
                .and_then(|k| self.world.block_entities.get_mut(&k))
        {
            f.input_clicked(&self.res.recipes, before);
        }
    }
    /// ItemEntityRenderer: items bob and spin; blocks render as small cubes.
    fn entity_geometry(&mut self, camera: glam::DVec3, alpha: f64) -> Vec<Vertex> {
        let mut out = Vec::new();
        // `ItemEntityRenderer`.
        self.dropped_item_geometry(camera, alpha, &mut out);
        // Mobs and their projectiles.
        self.mob_geometry(camera, alpha, &mut out);
        self.player_model_geometry(camera, alpha, &mut out);
        self.guest_model_geometry(camera, alpha, &mut out);
        self.fangs_geometry(camera, alpha, &mut out);
        self.name_tag_geometry(camera, alpha, &mut out);
        // Arrows: two crossed quads along the flight direction.
        if let Some(r) = self
            .res
            .atlas
            .sprites
            .get("entity/projectiles/arrow")
            .copied()
        {
            for a in &self.arrows {
                let p = glam::DVec3::new(
                    a.previous.x + (a.position.x - a.previous.x) * alpha,
                    a.previous.y + (a.position.y - a.previous.y) * alpha,
                    a.previous.z + (a.position.z - a.previous.z) * alpha,
                ) - camera;
                let dir = glam::Vec3::new(
                    a.velocity.x as f32,
                    a.velocity.y as f32,
                    a.velocity.z as f32,
                )
                .normalize_or(glam::Vec3::X);
                let side = dir.cross(glam::Vec3::Y).normalize_or(glam::Vec3::Z) * 0.08;
                let up = dir.cross(side).normalize_or(glam::Vec3::Y) * 0.08;
                let c = p.as_vec3();
                let (u1, v1) = (
                    r.u0 + (r.u1 - r.u0) * 0.5,
                    r.v0 + (r.v1 - r.v0) * (5.0 / 32.0),
                );
                for off in [side, up] {
                    let q = [
                        c - dir * 0.25 - off,
                        c + dir * 0.25 - off,
                        c + dir * 0.25 + off,
                        c - dir * 0.25 + off,
                    ];
                    for (v, uv) in q
                        .iter()
                        .zip([[r.u0, v1], [u1, v1], [u1, r.v0], [r.u0, r.v0]])
                    {
                        out.push(Vertex {
                            normal: [0; 4],
                            pos: v.to_array(),
                            uv,
                            layer: 0,
                            color: 0xFFFF_FFFF,
                            light: 240 | 240 << 8,
                        });
                    }
                }
            }
        }
        // Entity shadows (EntityRenderer.extractShadow).
        if let Some(sprite) = self.res.atlas.sprites.get("misc/shadow").copied() {
            let ambient = if self.dim == Dim::Nether { 0.1 } else { 0.0 };
            let darken = self.sky_darken();
            let a = alpha;
            let lerp = |p: Vec3, q: Vec3| {
                glam::DVec3::new(
                    p.x + (q.x - p.x) * a,
                    p.y + (q.y - p.y) * a,
                    p.z + (q.z - p.z) * a,
                )
            };
            let mut casters: Vec<(glam::DVec3, &str)> = self
                .mobs
                .iter()
                .map(|m| (lerp(m.previous, m.position), m.kind.id.as_str()))
                .collect();
            casters.extend(
                self.dropped
                    .iter()
                    .map(|i| (lerp(i.previous, i.position), "minecraft:item")),
            );
            casters.extend(
                self.orbs
                    .iter()
                    .map(|o| (lerp(o.previous, o.position), "minecraft:experience_orb")),
            );
            for (pos, id) in casters {
                let (radius, strength) = horizon_client::shadows::shadow_params(id);
                horizon_client::shadows::shadow(
                    &self.world,
                    &self.res.facts,
                    sprite,
                    camera,
                    pos,
                    radius,
                    strength,
                    ambient,
                    darken,
                    &mut out,
                );
            }
        }
        // Block entities (BlockEntityRenderers): chests, shulker boxes, banners, skulls...
        if let Some(r) = &self.renderer {
            let lid = |p: [i32; 3]| self.chest_lid(p, alpha as f32);
            let anim = |p: [i32; 3]| self.block_anim(p, alpha as f32);
            let (view_yaw, view_pitch) =
                if self.camera_type == perspective::CameraType::ThirdPersonFront {
                    (self.yaw + 180.0, -self.pitch)
                } else {
                    (self.yaw, self.pitch)
                };
            let scene = horizon_client::block_entity_render::Scene {
                camera,
                // Camera.setRotation.
                orientation: glam::Quat::from_euler(
                    glam::EulerRot::YXZ,
                    std::f32::consts::PI - view_yaw.to_radians(),
                    -view_pitch.to_radians(),
                    0.0,
                ),
                anim: &anim,
                texts: None,
                partial: alpha as f32,
                game_time: self.world.game_time,
                max_y: self.dim.max_y(),
                world: &self.world,
                sprites: &self.res.atlas.sprites,
                nether: self.dim == Dim::Nether,
                lid: &lid,
                sign: &|p| match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                    Some(BlockEntity::Sign(sign)) => Some(sign),
                    _ => None,
                },
                banner: &|p| self.banner_layers_at(p),
                glyph: &|c| self.res.atlas.glyph_cell(c),
            };
            // Block entities outside the view frustum are skipped (with a
            // block of margin for models larger than their block).
            let view = self.view_frustum;
            // Beams and structure outlines reach far outside their block
            // (BlockEntityRenderer.shouldRenderOffScreen): never culled.
            let off_screen = |state: u32| {
                let name = &*self.res.facts.get(BlockStateId(state)).name;
                matches!(
                    name,
                    "minecraft:beacon"
                        | "minecraft:end_gateway"
                        | "minecraft:structure_block"
                        | "minecraft:test_instance_block"
                )
            };
            let visible = r.block_entities().filter(|(p, state)| {
                off_screen(*state)
                    || view.is_none_or(|v| {
                        let o =
                            (glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
                                - camera)
                                .as_vec3();
                        v.intersects(o - glam::Vec3::ONE, o + glam::Vec3::splat(2.0))
                    })
            });
            self.res
                .block_entity_render
                .render(&scene, visible, &mut out);
        }
        // Experience orbs (ExperienceOrbRenderer): pulsing billboards from a 4x4 icon sheet.
        if let Some(r) = self
            .res
            .atlas
            .sprites
            .get("entity/experience/experience_orb")
            .copied()
        {
            let (yaw, pitch) = (self.yaw.to_radians(), self.pitch.to_radians());
            let forward = glam::Vec3::new(
                -yaw.sin() * pitch.cos(),
                -pitch.sin(),
                yaw.cos() * pitch.cos(),
            );
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
                let light = self.world.light(
                    o.position.x.floor() as i32,
                    o.position.y.floor() as i32,
                    o.position.z.floor() as i32,
                );
                let packed = (u32::from(light >> 4).max(10) * 16)
                    | ((u32::from(light & 15).max(10) * 16) << 8);
                let c = p.as_vec3();
                let s = 0.15;
                for (k, uv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)]
                    .iter()
                    .zip([[u0, v0 + dv], [u0 + du, v0 + dv], [u0 + du, v0], [u0, v0]])
                {
                    let v = c + right * (k.0 * s) + up * (k.1 * s);
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: v.to_array(),
                        uv,
                        layer: 0,
                        color,
                        light: packed,
                    });
                }
            }
        }
        // Falling blocks: a unit cube with the block's texture.
        for f in self.falling.clone() {
            let Some((sprite, tint)) = self.particle_sprite(f.state) else {
                continue;
            };
            let Some(r) = self.res.atlas.sprites.get(&sprite).copied() else {
                continue;
            };
            let p = glam::DVec3::new(
                f.previous.x + (f.position.x - f.previous.x) * alpha - 0.5,
                f.previous.y + (f.position.y - f.previous.y) * alpha,
                f.previous.z + (f.position.z - f.previous.z) * alpha - 0.5,
            ) - camera;
            let o = p.as_vec3();
            let light = self.world.light(
                f.position.x.floor() as i32,
                f.position.y.floor() as i32 + 1,
                f.position.z.floor() as i32,
            );
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let v = |x: f32, y: f32, z: f32| o + glam::Vec3::new(x, y, z);
            let faces = [
                (
                    [v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.), v(0., 1., 0.)],
                    1.0,
                ),
                (
                    [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
                    0.5,
                ),
                (
                    [v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
                    0.8,
                ),
                (
                    [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
                    0.8,
                ),
                (
                    [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
                    0.6,
                ),
                (
                    [v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)],
                    0.6,
                ),
            ];
            for (corners, shade) in faces {
                let c = |sft: u32| (((tint >> sft) & 255) as f32 * shade) as u32;
                let color = 0xFF00_0000 | c(0) << 16 | c(8) << 8 | c(16);
                for (p, uv) in
                    corners
                        .iter()
                        .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
                {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: p.to_array(),
                        uv,
                        layer: 0,
                        color,
                        light: packed,
                    });
                }
            }
        }
        self.tnt_geometry(camera, alpha, &mut out);
        self.painting_geometry(camera, &mut out);
        self.frame_geometry(camera, &mut out);
        self.armor_stand_geometry(camera, alpha, &mut out);
        self.boat_geometry(camera, alpha, &mut out);
        self.cart_geometry(camera, alpha, &mut out);
        self.beacon_geometry(camera, alpha, &mut out);
        self.fishing_geometry(camera, alpha, &mut out);
        self.leash_geometry(camera, alpha, &mut out);
        self.trident_geometry(camera, alpha, &mut out);
        self.rocket_geometry(camera, alpha, &mut out);
        // Particles (ParticleEngine.extract): camera-facing quads.
        let world = &self.world;
        self.particles.render(
            camera,
            self.yaw,
            self.pitch,
            alpha as f32,
            &|b| world.light(b[0], b[1], b[2]),
            &mut out,
        );
        let atlas = &self.res.atlas;
        // Breaking progress overlay (destroy_stage_0..9).
        if let Some(m) = &self.mining
            && m.progress > 0.0
        {
            let stage = ((m.progress * 10.0) as u32).min(9);
            if atlas
                .sprites
                .contains_key(&format!("block/destroy_stage_{stage}"))
            {
                let o = (glam::DVec3::new(
                    f64::from(m.pos[0]),
                    f64::from(m.pos[1]),
                    f64::from(m.pos[2]),
                ) - camera)
                    .as_vec3();
                let state = self.world.block(m.pos[0], m.pos[1], m.pos[2]);
                if self.res.facts.get(state).render_shape
                    == horizon_client::blocks::RenderShape::Model
                    && let Ok(models) = self
                        .res
                        .assets
                        .model_catalog
                        .collect(state, BlockPos::new(m.pos[0], m.pos[1], m.pos[2]))
                {
                    let offset = self
                        .res
                        .context
                        .tables
                        .offset(state, m.pos[0], m.pos[2])
                        .unwrap_or([0.0; 3]);
                    let origin = o + glam::DVec3::from(offset).as_vec3();
                    let translucent = models
                        .iter()
                        .any(|(_, model)| model.faces.iter().any(|f| f.translucent));
                    for (_, model) in models {
                        horizon_client::breaking::geometry(model, origin, translucent, &mut out);
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
        if self.dim == Dim::End {
            let sky = self
                .res
                .end_environment
                .sample(self.world.day_time, None, None);
            return (15.0 - 15.0 * sky.sky_light_level).round() as i32;
        }
        let (rain, thunder) = self.world.weather.levels(1.0);
        // Sampled once per (time, weather): every mob asks each tick.
        let key = (self.world.day_time, rain.to_bits(), thunder.to_bits());
        if let Some((k, darken)) = self.sky_darken_memo.get()
            && k == key
        {
            return darken;
        }
        let mut sky = self.res.environment.sample(self.world.day_time, None, None);
        horizon_client::environment::apply_weather(&mut sky, rain, thunder);
        // Level.updateSkyBrightness: darkening from the sky light level.
        let darken = (15.0 - 15.0 * sky.sky_light_level).round() as i32;
        self.sky_darken_memo.set(Some((key, darken)));
        darken
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
        if self.spectator() {
            return false;
        }
        // A painting nearer than the targeted block and mob breaks instead.
        if let Some((i, d)) = self.painting_hit()
            && self.mob_hit().is_none_or(|m| d < m.1)
            && self.pick().is_none_or(|h| {
                let eye = self.eye_position().unwrap_or_default();
                (0..3)
                    .map(|a| (h.point[a] - eye[a]).powi(2))
                    .sum::<f64>()
                    .sqrt()
                    > d
            })
        {
            self.break_painting(i, true);
            return true;
        }
        let hand_damage = 1.0
            + self
                .held()
                .and_then(|h| self.res.items.get(&h.item))
                .map_or(0.0, |i| i.attack_damage);
        if self.attack_frame()
            || self.attack_armor_stand()
            || self.attack_boat(hand_damage)
            || self.attack_cart(hand_damage)
            || self.attack_dragon()
        {
            return true;
        }
        let Some((index, distance)) = self.mob_hit() else {
            return false;
        };
        if let Some(hit) = self.pick() {
            let eye = self.eye_position().unwrap_or_default();
            let d = ((hit.point[0] - eye[0]).powi(2)
                + (hit.point[1] - eye[1]).powi(2)
                + (hit.point[2] - eye[2]).powi(2))
            .sqrt();
            if d < distance {
                return false;
            }
        }
        let held = self.held().cloned();
        let modifiers = held
            .as_ref()
            .and_then(|h| self.res.items.get(&h.item))
            .map(|i| (i.attack_damage, i.attack_speed))
            .unwrap_or((0.0, 0.0));
        let damage = 1.0 + modifiers.0 + self.effect_attack_bonus();
        let speed = (4.0 + modifiers.1).max(0.1);
        let delay = 20.0 / speed;
        let scale = ((self.attack_ticks as f32 + 0.5) / delay).clamp(0.0, 1.0);
        let mut amount = damage * (0.2 + scale * scale * 0.8);
        let player = self.player.as_ref();
        let falling =
            player.is_some_and(|p| !p.ground.on_ground && p.velocity.y < 0.0 && !p.sprinting);
        let crit = scale > 0.9 && falling;
        if crit {
            amount *= 1.5;
        }
        // Enchantment damage (sharpness, smite, bane) scaled by the attack strength.
        let target_kind = self.mobs[index].kind.id.clone();
        amount += self.enchant_damage_bonus(&format!(
            "minecraft:{}",
            target_kind.trim_start_matches("minecraft:")
        )) * scale;
        self.attack_ticks = 0;
        if let Some(p) = self.player.as_mut()
            && !self.creative
        {
            p.health.food.exhaustion = (p.health.food.exhaustion + 0.1).min(40.0);
        }
        let player = self.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        self.swing_arm();
        // LivingEntity.setLastHurtMob (tamed wolves join in).
        self.combat.last_hurt_mob = Some((self.mobs[index].id, self.world.game_time));
        let mob = &mut self.mobs[index];
        let center = [
            mob.position.x,
            mob.position.y + mob.kind.height / 2.0,
            mob.position.z,
        ];
        let dealt = (amount.min(mob.health.max(0.0)) * 10.0).round() as i32;
        if mob.hurt(amount, from, &mut self.world.random) {
            self.stats.custom("damage_dealt", dealt);
            // Player.attack: crit emitter and damage indicator hearts.
            let (feet, width, height) = (
                [mob.position.x, mob.position.y, mob.position.z],
                mob.kind.width,
                mob.kind.height,
            );
            if crit {
                self.particles.tracking_emitter(
                    horizon_client::particles::ParticleOptions::simple(
                        horizon_client::particles::ParticleType::Crit,
                    ),
                    feet,
                    width,
                    height,
                );
            }
            if amount > 2.0 {
                let indicator = horizon_client::particles::ParticleOptions::simple(
                    horizon_client::particles::ParticleType::DamageIndicator,
                );
                self.particles.send_particles(
                    &indicator,
                    [feet[0], feet[1] + height * 0.5, feet[2]],
                    [0.1, 0.0, 0.1],
                    0.2,
                    (amount * 0.5) as u32,
                );
            }
            self.last_hurt_by_player.insert(mob.id);
            let sound = if crit {
                "minecraft:entity.player.attack.crit"
            } else if scale > 0.9 {
                "minecraft:entity.player.attack.strong"
            } else {
                "minecraft:entity.player.attack.weak"
            };
            self.audio.play(sound, Some(center), 1.0, 1.0);
            // The hurt or death sound the mob raised (getHurtSound / getDeathSound,
            // its voice pitch from its own random).
            let queued = std::mem::take(&mut self.mobs[index].outbox);
            for event in queued {
                if let horizon_client::mobs::MobEvent::Sound(s, _, volume, pitch) = event {
                    self.audio
                        .play(&format!("minecraft:{s}"), Some(center), volume, pitch);
                }
            }
            self.post_attack(index);
            if let Some(stack) = self.inventory[self.selected].clone()
                && let Some(max) = self
                    .res
                    .items
                    .get(&stack.item)
                    .filter(|i| i.max_damage > 0)
                    .map(|i| i.max_damage)
            {
                let loss = self.durability_loss(&stack, 1);
                if let Some(stack) = self.inventory[self.selected].as_mut() {
                    stack.damage += loss;
                    if stack.damage >= max {
                        self.inventory[self.selected] = None;
                    }
                }
            }
        }
        true
    }
    /// ServerExplosion: rays from a 16^3 grid lose (resistance + 0.3) * 0.3 per step.
    /// A mob explosion (`Level.ExplosionInteraction.MOB`: drop decay with the
    /// default mobGriefing rule).
    fn explode(&mut self, center: [f64; 3], radius: f32) -> Result<()> {
        self.server_explode(
            center,
            radius,
            explosion::BlockInteraction::DestroyWithDecay,
            false,
        )
    }
    fn tick_block_entities(&mut self) -> Result<()> {
        self.tick_hoppers()?;
        self.tick_crafters();
        self.tick_mount_menu();
        self.tick_jukeboxes();
        self.tick_beacons();
        self.tick_creaking_hearts()?;
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
    fn with_entities(
        &self,
        chunk: &Arc<horizon_client::chunk::Chunk>,
    ) -> Arc<horizon_client::chunk::Chunk> {
        let mut c = (**chunk).clone();
        c.block_entities = self
            .world
            .block_entities
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, e)| (*p, e.clone()))
            .collect();
        c.block_names = self
            .world
            .block_names
            .iter()
            .filter(|(p, _)| p.0 >> 4 == c.x && p.2 >> 4 == c.z)
            .map(|(p, n)| (*p, n.clone()))
            .collect();
        self.attach_save_data(&mut c);
        Arc::new(c)
    }
    fn tick_mobs(&mut self) -> Result<()> {
        self.tick_creatures()?;
        self.position_rider();
        self.tick_stats();
        self.tick_scoreboard();
        self.load_bossbars();
        self.tick_functions();
        self.tick_worldborder();
        self.tick_subtitles();
        self.spawn_phantoms();
        self.tick_advancements();
        self.tick_player_model();
        Ok(())
    }
    /// SkyRenderer.renderSunMoonAndStars: bodies on a frame rotated Y -90 then X by angle.
    fn celestial_geometry(&self, sky: &horizon_client::environment::Sky) -> Vec<Vertex> {
        let mut out = Vec::new();
        let atlas = &self.res.atlas;
        let frame = |angle: f32| {
            glam::Mat4::from_rotation_y((-90.0f32).to_radians())
                * glam::Mat4::from_rotation_x(angle.to_radians())
        };
        let mut body = |name: &str, angle: f32, size: f32, color: u32| {
            let Some(r) = atlas.sprites.get(name).copied() else {
                return;
            };
            let m = frame(angle);
            let corners = [(-size, -size), (size, -size), (size, size), (-size, size)];
            let uv = [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]];
            for ((x, z), uv) in corners.iter().zip(uv) {
                let p = m.transform_point3(glam::Vec3::new(*x, 100.0, *z));
                out.push(Vertex {
                    normal: [0; 4],
                    pos: p.to_array(),
                    uv,
                    layer: 2,
                    color,
                    light: 0,
                });
            }
        };
        body(
            "environment/celestial/sun",
            sky.sun_angle,
            30.0,
            0xFFFF_FFFF,
        );
        let phases = [
            "full_moon",
            "waning_gibbous",
            "third_quarter",
            "waning_crescent",
            "new_moon",
            "waxing_crescent",
            "first_quarter",
            "waxing_gibbous",
        ];
        let phase = phases[(self.world.day_time / 24000).rem_euclid(8) as usize];
        body(
            &format!("environment/celestial/moon/{phase}"),
            sky.moon_angle,
            20.0,
            0xFFFF_FFFF,
        );
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
                        out.push(Vertex {
                            normal: [0; 4],
                            pos: p.to_array(),
                            uv: [cu, cv],
                            layer: 2,
                            color,
                            light: 0,
                        });
                    }
                }
            }
        }
        out
    }
    /// CloudRenderer (fancy): 12x4x12 cells from clouds.png drifting east.
    fn cloud_geometry(
        &self,
        sky: &horizon_client::environment::Sky,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let (w, h, cells) = &self.res.clouds;
        let (w, h) = (i64::from(*w), i64::from(*h));
        let Some(white) = self.res.atlas.sprites.get("horizon/white").copied() else {
            return;
        };
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
        // The Cloud Distance option, not the render distance.
        let radius = horizon_client::clouds::radius_cells(self.settings.cloud_range);
        let c = sky.cloud_color;
        let color = |shade: f32| {
            let ch = |v: f32| ((v * shade).clamp(0.0, 1.0) * 255.0) as u32;
            ch(c[3].clamp(0.0, 1.0)) << 24 | ch(c[2]) << 16 | ch(c[1]) << 8 | ch(c[0])
        };
        let light = 240 | 240 << 8;
        for (dx, dz) in horizon_client::clouds::cells(radius) {
            let (dx, dz) = (i64::from(dx), i64::from(dz));
            let (cx, cz) = (cell_x + dx, cell_z + dz);
            if !filled(cx, cz) {
                continue;
            }
            let x0 = dx as f32 * 12.0 - x_in;
            let z0 = dz as f32 * 12.0 - z_in;
            let (x1, z1, y0, y1) = (x0 + 12.0, z0 + 12.0, bottom, bottom + 4.0);
            let mut quad = |p: [[f32; 3]; 4], shade: f32| {
                for v in p {
                    // Overlay 4: clouds (unlit, faded by the cloud fog end).
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: v,
                        uv,
                        layer: 4,
                        color: color(shade),
                        light,
                    });
                }
            };
            let faces = [
                (
                    [[x1, y0, z0], [x1, y0, z1], [x0, y0, z1], [x0, y0, z0]],
                    0.7,
                ),
                (
                    [[x0, y1, z0], [x0, y1, z1], [x1, y1, z1], [x1, y1, z0]],
                    1.0,
                ),
                (
                    [[x0, y0, z0], [x0, y1, z0], [x1, y1, z0], [x1, y0, z0]],
                    0.8,
                ),
                (
                    [[x1, y0, z1], [x1, y1, z1], [x0, y1, z1], [x0, y0, z1]],
                    0.8,
                ),
                (
                    [[x0, y0, z1], [x0, y1, z1], [x0, y1, z0], [x0, y0, z0]],
                    0.9,
                ),
                (
                    [[x1, y0, z0], [x1, y1, z0], [x1, y1, z1], [x1, y0, z1]],
                    0.9,
                ),
            ];
            let mask = horizon_client::clouds::face_mask(
                dx as i32,
                dz as i32,
                bottom,
                [
                    !filled(cx, cz - 1),
                    !filled(cx, cz + 1),
                    !filled(cx - 1, cz),
                    !filled(cx + 1, cz),
                ],
            );
            for (direction, inside) in horizon_client::clouds::faces(mask) {
                let (p, shade) = faces[usize::from(direction)];
                quad(if inside { [p[3], p[2], p[1], p[0]] } else { p }, shade);
            }
        }
    }
    /// Load (or create) a world folder under saves/ and start streaming it.
    fn open_world(
        &mut self,
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
    ) -> Result<()> {
        let save_root = std::env::current_dir()?.join("saves").join(folder);
        horizon_client::save::upgrade_older_world(&save_root)?;
        horizon_client::save::migrate_legacy(&save_root)?;
        self.session_lock = None;
        self.session_lock = Some(horizon_client::save::SessionLock::acquire(&save_root)?);
        self.persist = Default::default();
        self.score = Default::default();
        self.fns = Default::default();
        self.bosses = Default::default();
        // RandomSequences: the world's data/minecraft/random_sequences.dat.
        self.rand_sequences =
            horizon_client::save::read_dat(&save_root.join("data/minecraft/random_sequences.dat"))
                .and_then(|t| t.get("data").cloned())
                .map(|d| horizon_client::random_sequences::RandomSequences::from_nbt(&d))
                .unwrap_or_default();
        self.border = Default::default();
        let level = Level::read(&save_root).unwrap_or_else(|| Level {
            name: name.unwrap_or(folder).to_owned(),
            // WorldOptions.randomSeed when no seed was given.
            seed: seed.unwrap_or_else(|| {
                let t = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_or(0, |d| d.as_nanos() as u64);
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
        horizon_client::crash::set_detail("World", format!("{} (saves/{folder})", level.name));
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
        self.set_mode_id(level.mode());
        let mut inventory = vec![None; 36];
        self.armor = [None, None, None, None];
        self.offhand = None;
        self.ender_items = if level.ender_items.len() == 27 {
            level.ender_items.clone()
        } else {
            vec![None; 27]
        };
        for (slot, id, count, damage) in &level.inventory {
            let stack = ItemStack {
                item: Arc::from(id.as_str()),
                count: *count,
                damage: *damage,
                extra: level.inventory_components.get(slot).cloned(),
            };
            if (100..104).contains(slot) {
                self.armor[usize::from(103 - slot)] = Some(stack);
            } else if *slot == 150 {
                self.offhand = Some(stack);
            } else if let Some(cell) = inventory.get_mut(usize::from(*slot)) {
                *cell = Some(stack);
            }
        }
        if self.creative && level.inventory.is_empty() && level.position.is_none() {
            for (i, name) in [
                "minecraft:grass_block",
                "minecraft:stone",
                "minecraft:oak_planks",
                "minecraft:glass",
                "minecraft:torch",
                "minecraft:oak_log",
                "minecraft:chest",
                "minecraft:furnace",
                "minecraft:diamond_pickaxe",
            ]
            .iter()
            .enumerate()
            {
                inventory[i] = Some(ItemStack {
                    item: Arc::from(*name),
                    count: 1,
                    damage: 0,
                    extra: None,
                });
            }
        }
        let (yaw, pitch) = self
            .options
            .look
            .unwrap_or((level.rotation[0], level.rotation[1]));
        self.yaw = yaw;
        self.pitch = pitch;
        self.selected = level.selected_slot.clamp(0, 8) as usize;
        self.streaming = streaming;
        self.world = world;
        self.inventory = inventory;
        self.save = save;
        self.level = level;
        // ServerRecipeBook.loadUntrusted: recipes the manager does not have are dropped.
        let index = &self.res.stations.recipe_index;
        let book = &mut self.level.recipe_book;
        book.recipes.retain(|r| index.contains(r));
        book.to_be_displayed.retain(|r| index.contains(r));
        self.dim = dim;
        self.world_root = save_root.clone();
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.install_live_features();
        self.tick_chunks.clear();
        self.falling.clear();
        self.tnts.clear();
        self.travelling = None;
        self.player = None;
        self.spawn = None;
        self.spawn_search = 0;
        self.mobs.clear();
        self.creatures.reset_trader();
        self.arrows.clear();
        self.dropped.clear();
        self.particles.clear();
        self.orbs.clear();
        self.experience = Experience {
            level: self.level.xp_level,
            progress: self.level.xp_progress,
            total: self.level.xp_total,
        };
        self.restore_effects();
        self.mining = None;
        self.container = None;
        let kinds = std::mem::take(&mut self.be_anims.kinds);
        self.be_anims = horizon_client::block_entity_anim::BlockEntityAnims {
            kinds,
            ..Default::default()
        };
        self.carried = None;
        self.using = None;
        self.screen = Screen::Playing;
        self.title = None;
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.apply_settings();
        self.streaming.sky_light = dim != Dim::Nether;
        self.streaming.nether_shading = dim == Dim::Nether;
        // New worlds get their level.dat right away.
        if !save_root.join("level.dat").exists() {
            self.level.write(&save_root)?;
        }
        self.load_world_state();
        Ok(())
    }
    /// Streaming and region storage for one dimension of a world.
    fn make_streaming(
        &self,
        world_root: &std::path::Path,
        dim: Dim,
        seed: i64,
    ) -> Result<(Streaming, Arc<SaveDir>)> {
        let save = Arc::new(SaveDir::new(horizon_client::save::dimension_dir(
            world_root,
            dim.id(),
        )));
        let res = &*self.res;
        let flat = (dim == Dim::Overworld)
            .then(|| horizon_client::save::world_gen_settings(world_root))
            .flatten()
            .and_then(|s| {
                horizon_client::flatgen::FlatTerrain::from_settings(
                    &s,
                    res.assets.registry,
                    &res.biomes,
                )
            });
        let mut streaming = if let Some(inner) = flat {
            Streaming::new(
                Arc::new(Persistent {
                    inner: Arc::new(inner),
                    save: save.clone(),
                    registry: res.assets.clone(),
                    biomes: res.biomes.clone(),
                }),
                res.context.clone(),
                res.threads,
            )
        } else if self.options.test_terrain && dim == Dim::Overworld {
            let inner = Arc::new(TestTerrain::new(res.assets.registry, &res.biomes)?);
            Streaming::new(
                Arc::new(Persistent {
                    inner,
                    save: save.clone(),
                    registry: res.assets.clone(),
                    biomes: res.biomes.clone(),
                }),
                res.context.clone(),
                res.threads,
            )
        } else {
            let inner = Arc::new(VanillaTerrain::for_dimension(&self.options.resources.join("worldgen"), res.assets.registry, &res.biomes, seed, dim.id()).map_err(|e| {
                format!("{e}\nPrepare the worldgen bundle with the launcher, or: horizon-prepare --prepare")
            })?);
            self.end.set_locator(Some(inner.clone()));
            Streaming::new(
                Arc::new(Persistent {
                    inner,
                    save: save.clone(),
                    registry: res.assets.clone(),
                    biomes: res.biomes.clone(),
                }),
                res.context.clone(),
                res.threads,
            )
        };
        streaming.radius = self.settings.render_distance;
        streaming.biome_blend = self.settings.biome_blend;
        streaming.smooth_lighting = self.settings.smooth_lighting;
        streaming.cutout_leaves = self.settings.cutout_leaves;
        streaming.biome_zoom = BiomeZoom::new(obfuscate_seed(seed));
        streaming.sky_light = dim != Dim::Nether;
        streaming.nether_shading = dim == Dim::Nether;
        Ok((streaming, save))
    }
    /// Change dimension through a portal (Entity.changeDimension with
    /// PortalForcer placement once the destination has loaded).
    fn enter_dimension(&mut self, target: Dim) -> Result<()> {
        let Some(player) = self.player.take() else {
            return Ok(());
        };
        // Save the departing dimension's changed chunks.
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks: Vec<_> = keys
            .iter()
            .filter_map(|k| self.world.chunks.get(k))
            .map(|c| self.with_entities(c))
            .collect();
        self.save
            .save(&chunks, self.res.assets.registry, &self.res.biomes)?;
        self.save_dimension_entities()?;
        // Coordinates scale by 8 between the Overworld and the Nether.
        let scale = match (self.dim, target) {
            (Dim::Overworld, Dim::Nether) => 1.0 / 8.0,
            (Dim::Nether, Dim::Overworld) => 8.0,
            (Dim::End, Dim::Nether) => 1.0 / 8.0,
            _ => 1.0,
        };
        let destination = self.end_destination(target).unwrap_or(Vec3::new(
            player.position.x * scale,
            player.position.y,
            player.position.z * scale,
        ));
        let from_end = self.dim == Dim::End;
        let (streaming, save) =
            self.make_streaming(&self.world_root.clone(), target, self.level.seed)?;
        let mut world = World::new(self.res.facts.air, self.level.seed);
        world.game_time = self.world.game_time;
        world.day_time = self.world.day_time;
        world.weather = self.world.weather;
        self.streaming = streaming;
        self.world = world;
        self.save = save;
        let from = self.dim;
        self.dim = target;
        self.trigger_dimension(from, target);
        self.level.dimension = target.id().to_owned();
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.falling.clear();
        self.tnts.clear();
        self.mobs.clear();
        self.arrows.clear();
        self.dropped.clear();
        self.particles.clear();
        self.orbs.clear();
        self.mining = None;
        self.travelling = Some((player, destination));
        self.portal_cooldown = 300;
        self.end_travel = (target == Dim::End || from_end) && self.end_portal_travel;
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.apply_settings();
        Ok(())
    }
    /// PortalShape: an obsidian frame around 2..=21 by 3..=21 empty blocks.
    fn portal_shape(&self, start: [i32; 3], x_axis: bool) -> Option<([i32; 3], i32, i32)> {
        let name = |p: [i32; 3]| {
            self.res
                .facts
                .get(self.world.block(p[0], p[1], p[2]))
                .name
                .clone()
        };
        let empty = |p: [i32; 3]| {
            let n = name(p);
            matches!(
                &*n,
                "minecraft:air"
                    | "minecraft:cave_air"
                    | "minecraft:fire"
                    | "minecraft:soul_fire"
                    | "minecraft:nether_portal"
            )
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
        if !(2..=21).contains(&width)
            || !frame([left[0] + d[0] * width, left[1], left[2] + d[2] * width])
        {
            return None;
        }
        let mut height = 0;
        'rows: while height <= 21 {
            let y = left[1] + height;
            if !frame([left[0] - d[0], y, left[2] - d[2]])
                || !frame([left[0] + d[0] * width, y, left[2] + d[2] * width])
            {
                break;
            }
            for i in 0..width {
                if !empty([left[0] + d[0] * i, y, left[2] + d[2] * i]) {
                    break 'rows;
                }
            }
            height += 1;
        }
        let top_ok =
            (0..width).all(|i| frame([left[0] + d[0] * i, left[1] + height, left[2] + d[2] * i]));
        ((3..=21).contains(&height) && top_ok).then_some((left, width, height))
    }
    /// BaseFireBlock.onPlace: lighting fire inside a frame creates a portal.
    fn try_light_portal(&mut self, at: [i32; 3]) -> Result<bool> {
        for x_axis in [true, false] {
            if let Some((origin, width, height)) = self.portal_shape(at, x_axis) {
                let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
                let registry = &self.res.assets.registry;
                let Some(default) = registry.default_state("minecraft:nether_portal") else {
                    return Ok(false);
                };
                let mut portal = registry.state(default).cloned().ok_or("portal state")?;
                portal
                    .properties
                    .insert("axis".into(), if x_axis { "x" } else { "z" }.into());
                let id = registry.resolve(&portal).unwrap_or(default);
                for h in 0..height {
                    for w in 0..width {
                        self.world.set(
                            origin[0] + d[0] * w,
                            origin[1] + h,
                            origin[2] + d[2] * w,
                            id,
                        )?;
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
        let (min_y, max_y) = if self.dim == Dim::Nether {
            (1, 126)
        } else {
            (MIN_Y + 1, MIN_Y + HEIGHT - 3)
        };
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
                        let d = i64::from(x - tx).pow(2)
                            + i64::from(z - tz).pow(2)
                            + i64::from(y - target.y as i32).pow(2);
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
        player.position = Vec3::new(
            f64::from(spot[0]) + 0.5,
            f64::from(spot[1]),
            f64::from(spot[2]) + 0.5,
        );
        player.velocity = Vec3::ZERO;
        self.previous_position = player.position;
        self.player = Some(player);
        self.sway = Default::default();
        self.apply_equipment();
        let at = [f64::from(spot[0]), f64::from(spot[1]), f64::from(spot[2])];
        self.audio
            .play("minecraft:block.portal.travel", Some(at), 0.25, 1.0);
        Ok(())
    }
    fn tick_portal(&mut self) -> Result<()> {
        let Some(p) = self.player.as_ref() else {
            return Ok(());
        };
        let w = f64::from(p.dimensions.width) / 2.0;
        let (lo, hi) = (
            [p.position.x - w, p.position.y, p.position.z - w],
            [
                p.position.x + w,
                p.position.y + f64::from(p.dimensions.height),
                p.position.z + w,
            ],
        );
        let mut inside = false;
        for x in lo[0].floor() as i32..=hi[0].floor() as i32 {
            for y in lo[1].floor() as i32..=hi[1].floor() as i32 {
                for z in lo[2].floor() as i32..=hi[2].floor() as i32 {
                    if &*self.res.facts.get(self.world.block(x, y, z)).name
                        == "minecraft:nether_portal"
                    {
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
            self.audio.play(
                "minecraft:block.portal.trigger",
                at,
                0.25,
                self.world.random.next_float() * 0.4 + 0.8,
            );
        }
        self.portal_ticks += 1;
        // Player.getPortalWaitTime: 80 ticks, or 1 when invulnerable (creative).
        let wait = if self.creative { 1 } else { 80 };
        if self.portal_ticks >= wait {
            self.portal_ticks = 0;
            // NetherPortalBlock: the Nether leads to the Overworld, anywhere else to the Nether.
            let target = if self.dim == Dim::Nether {
                Dim::Overworld
            } else {
                Dim::Nether
            };
            self.end_portal_travel = false;
            self.enter_dimension(target)?;
        }
        Ok(())
    }
    /// Leaving the game with a world open (window closed, Quit, `quit`):
    /// menu items go back (Player.closeContainer), then the world is saved,
    /// once.
    pub(super) fn save_for_exit(&mut self) -> Result<()> {
        if self.title.is_some() || self.saved_for_exit {
            return Ok(());
        }
        self.saved_for_exit = true;
        self.close_container();
        if self.remote.is_none() {
            self.stats.custom("leave_game", 1);
        }
        self.save_game()
    }
    fn close_world(&mut self) -> Result<()> {
        self.ambience.clear(&mut self.audio);
        // updateLevelInEngines(null): every sound stops; the music manager
        // lets go of its track.
        self.music.stop_playing(None, &mut self.audio);
        self.audio.stop_all();
        if self.remote.is_some() {
            return self.leave_server(None);
        }
        self.stats.custom("leave_game", 1);
        self.save_game()?;
        horizon_client::crash::clear_detail("World");
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
            (
                "0",
                [
                    glam::Vec3::new(-d, d, -d),
                    glam::Vec3::new(d, d, -d),
                    glam::Vec3::new(d, -d, -d),
                    glam::Vec3::new(-d, -d, -d),
                ],
            ),
            (
                "1",
                [
                    glam::Vec3::new(d, d, -d),
                    glam::Vec3::new(d, d, d),
                    glam::Vec3::new(d, -d, d),
                    glam::Vec3::new(d, -d, -d),
                ],
            ),
            (
                "2",
                [
                    glam::Vec3::new(d, d, d),
                    glam::Vec3::new(-d, d, d),
                    glam::Vec3::new(-d, -d, d),
                    glam::Vec3::new(d, -d, d),
                ],
            ),
            (
                "3",
                [
                    glam::Vec3::new(-d, d, d),
                    glam::Vec3::new(-d, d, -d),
                    glam::Vec3::new(-d, -d, -d),
                    glam::Vec3::new(-d, -d, d),
                ],
            ),
            (
                "4",
                [
                    glam::Vec3::new(-d, d, d),
                    glam::Vec3::new(d, d, d),
                    glam::Vec3::new(d, d, -d),
                    glam::Vec3::new(-d, d, -d),
                ],
            ),
            (
                "5",
                [
                    glam::Vec3::new(-d, -d, -d),
                    glam::Vec3::new(d, -d, -d),
                    glam::Vec3::new(d, -d, d),
                    glam::Vec3::new(-d, -d, d),
                ],
            ),
        ];
        for (index, corners) in faces {
            let Some(r) = self
                .res
                .atlas
                .sprites
                .get(&format!("gui/title/background/panorama_{index}"))
                .copied()
            else {
                continue;
            };
            let uv = [[r.u0, r.v0], [r.u1, r.v0], [r.u1, r.v1], [r.u0, r.v1]];
            for (p, uv) in corners.iter().zip(uv) {
                out.push(Vertex {
                    normal: [0; 4],
                    pos: p.to_array(),
                    uv,
                    layer: 2,
                    color: 0xFFFF_FFFF,
                    light: 0,
                });
            }
        }
        out
    }
    /// Equipment attribute modifiers (LivingEntity.collectEquipmentChanges).
    fn apply_equipment(&mut self) {
        let Some(player) = self.player.as_mut() else {
            return;
        };
        for (attribute, id) in std::mem::take(&mut self.applied_modifiers) {
            let _ = player.set_modifier(&attribute, &id, 0.0, "add_value", false);
        }
        let slots = ["head", "chest", "legs", "feet"];
        let mut equipped: Vec<(Option<&ItemStack>, &str)> = self
            .armor
            .iter()
            .zip(slots)
            .map(|(s, n)| (s.as_ref(), n))
            .collect();
        equipped.push((self.inventory[self.selected].as_ref(), "mainhand"));
        for (stack, slot) in equipped {
            let Some(info) = stack.and_then(|s| self.res.items.get(&s.item)) else {
                continue;
            };
            for (attribute, id, amount, op, group) in &info.modifiers {
                let matches = group == slot
                    || group == "any"
                    || (group == "armor" && slot != "mainhand")
                    || (group == "hand" && slot == "mainhand");
                // Main-hand attack modifiers are read by the attack code directly.
                if !matches || slot == "mainhand" {
                    continue;
                }
                if player
                    .set_modifier(attribute, id, *amount, op, true)
                    .is_ok()
                {
                    self.applied_modifiers.push((attribute.clone(), id.clone()));
                }
            }
        }
        self.apply_enchantment_attributes();
    }
    /// Inventory.hurtArmor: each piece loses max(1, damage / 4) durability.
    fn hurt_armor(&mut self, damage: f32) {
        let amount = ((damage / 4.0) as u32).max(1);
        let mut broke = false;
        let losses: Vec<u32> = (0..4)
            .map(|i| {
                self.armor[i]
                    .clone()
                    .map_or(amount, |s| self.durability_loss(&s, amount))
            })
            .collect();
        for (slot, amount) in self.armor.iter_mut().zip(losses) {
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
                self.audio.play(
                    "minecraft:entity.item.break",
                    Some(eye),
                    0.8,
                    0.8 + self.world.random.next_float() * 0.4,
                );
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
            let p = [
                origin[0] + dir[0] * t,
                origin[1] + dir[1] * t,
                origin[2] + dir[2] * t,
            ];
            let b = p.map(|v| v.floor() as i32);
            if b != last {
                let state = self.world.block(b[0], b[1], b[2]);
                let f = self.res.facts.get(state);
                let face = [last[0] - b[0], last[1] - b[1], last[2] - b[2]];
                if f.fluid != Fluid::None
                    && f.fluid_level == 8
                    && !f.fluid_falling
                    && !f.waterlogged
                    && f.render_shape != horizon_client::blocks::RenderShape::Model
                {
                    return Some(Hit {
                        pos: b,
                        face,
                        point: p,
                    });
                }
                if !self.res.facts.is_air(state) && f.fluid == Fluid::None {
                    return Some(Hit {
                        pos: b,
                        face,
                        point: p,
                    });
                }
                last = b;
            }
            t += 0.05;
        }
        None
    }
    /// `CauldronInteractions` for the held item aimed at a cauldron.
    fn use_on_cauldron(&mut self, pos: [i32; 3]) -> Result<bool> {
        use horizon_client::cauldron::{self, Outcome};
        struct Hand<'g, 'a>(&'g mut Game<'a>);
        impl cauldron::Player for Hand<'_, '_> {
            fn creative(&self) -> bool {
                self.0.creative
            }
            fn hand(&mut self) -> &mut Option<ItemStack> {
                let selected = self.0.selected;
                &mut self.0.inventory[selected]
            }
            fn add(&mut self, stack: &mut ItemStack) -> bool {
                let before = stack.count;
                match self.0.give(stack.clone()) {
                    None => stack.count = 0,
                    Some(rest) => stack.count = rest.count,
                }
                stack.count < before
            }
            fn contains(&self, stack: &ItemStack) -> bool {
                self.0
                    .inventory
                    .iter()
                    .flatten()
                    .any(|have| have.same_item_same_components(stack))
            }
            fn drop_stack(&mut self, stack: ItemStack) {
                if let Some(eye) = self.0.eye_position() {
                    self.0.spawn_item(stack, eye, [0.0, 0.1, 0.0], 40);
                }
            }
        }
        let registry = self.res.assets.registry;
        let id = self.world.block(pos[0], pos[1], pos[2]);
        let Some(state) = registry
            .state(id)
            .filter(|s| s.name.ends_with("cauldron"))
            .cloned()
        else {
            return Ok(false);
        };
        let Some(held) = self.inventory[self.selected].as_ref() else {
            return Ok(false);
        };
        let removes_dye = cauldron::removes_dye(&self.res.items, &held.item);
        let above = self.world.block(pos[0], pos[1] + 1, pos[2]);
        let under_water = registry
            .state(above)
            .is_some_and(|s| s.name == "minecraft:water");
        let result = cauldron::interact(&state, under_water, removes_dye, &mut Hand(self));
        if result.outcome == Outcome::Pass {
            return Ok(false);
        }
        if let Some(block) = result.block
            && let Ok(new) = registry.resolve(&block)
        {
            self.level_set(pos, new)?;
        }
        if let Some(sound) = result.sound {
            self.audio.play(
                sound,
                Some([
                    f64::from(pos[0]) + 0.5,
                    f64::from(pos[1]) + 0.5,
                    f64::from(pos[2]) + 0.5,
                ]),
                1.0,
                1.0,
            );
        }
        Ok(true)
    }
    /// `BlockTransformer.transformBlock` for a held axe, hoe or shovel.
    fn use_block_transformer(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
    ) -> Result<bool> {
        use horizon_client::block_transform::Around;
        struct Column<'w> {
            world: &'w World,
            registry: &'w horizon_world::registry::BlockRegistry,
            pos: [i32; 3],
        }
        impl Around for Column<'_> {
            fn state(&self, o: [i32; 3]) -> horizon_world::chunk::BlockState {
                let id =
                    self.world
                        .block(self.pos[0] + o[0], self.pos[1] + o[1], self.pos[2] + o[2]);
                self.registry.state(id).cloned().unwrap_or_else(|| {
                    horizon_world::chunk::BlockState {
                        name: "minecraft:air".into(),
                        properties: Default::default(),
                    }
                })
            }
        }
        let Some(id) = self
            .res
            .items
            .get(&stack.item)
            .and_then(|i| i.block_transformer.clone())
        else {
            return Ok(false);
        };
        let face_name = match face {
            [0, -1, 0] => "down",
            [0, 1, 0] => "up",
            [0, 0, -1] => "north",
            [0, 0, 1] => "south",
            [-1, 0, 0] => "west",
            _ => "east",
        };
        let registry = self.res.assets.registry;
        let (state, sound, loot, damage, particle) = {
            let around = Column {
                world: &self.world,
                registry,
                pos,
            };
            let Some((_, t, state)) =
                self.res
                    .transformers
                    .apply(&self.res.items, registry, &id, face_name, &around)
            else {
                return Ok(false);
            };
            (
                state,
                t.sound.clone(),
                t.loot.clone(),
                t.damage,
                t.particle.clone(),
            )
        };
        let Ok(new) = registry.resolve(&state) else {
            return Ok(false);
        };
        self.level_set(pos, new)?;
        // BlockTransformer.TransformParticle.send.
        match particle.as_str() {
            "scrape" => self.level_event_at(3005, pos, 0),
            "wax_on" => self.level_event_at(3003, pos, 0),
            "wax_off" => self.level_event_at(3004, pos, 0),
            _ => {}
        }
        if loot.as_deref() == Some("minecraft:till/rooted_dirt") {
            self.pop_resource(
                [pos[0] + face[0], pos[1] + face[1], pos[2] + face[2]],
                ItemStack::new("minecraft:hanging_roots", 1),
            );
        }
        self.audio.play(
            &sound,
            Some([
                f64::from(pos[0]) + 0.5,
                f64::from(pos[1]) + 0.5,
                f64::from(pos[2]) + 0.5,
            ]),
            1.0,
            1.0,
        );
        self.damage_held(damage);
        Ok(true)
    }
    fn set_state_named(&mut self, pos: [i32; 3], name: &str, keep: &[&str]) -> Result<bool> {
        let registry = &self.res.assets.registry;
        let old = self.world.block(pos[0], pos[1], pos[2]);
        let Some(default) = registry.default_state(name) else {
            return Ok(false);
        };
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
        let amount = match self.inventory[self.selected].clone() {
            Some(held) => self.durability_loss(&held, amount),
            None => amount,
        };
        if let Some(stack) = self.inventory[self.selected].as_mut()
            && let Some(info) = self.res.items.get(&stack.item)
            && info.max_damage > 0
        {
            stack.damage += amount;
            if stack.damage >= info.max_damage {
                let broken = stack.item.clone();
                self.stats.add("broken", &broken, 1);
                self.inventory[self.selected] = None;
                if let Some(eye) = self.eye_position() {
                    self.audio.play(
                        "minecraft:entity.item.break",
                        Some(eye),
                        0.8,
                        0.8 + self.world.random.next_float() * 0.4,
                    );
                }
            }
        }
    }
    /// Item use on blocks for tools, buckets and fire starters; returns true if handled.
    fn use_tool(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = stack.item.trim_start_matches("minecraft:").to_owned();
        let at = |p: [i32; 3]| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]
        };
        if let Some(hit) = self.pick()
            && self.use_on_cauldron(hit.pos)?
        {
            return Ok(true);
        }
        if item == "bucket" {
            // getPlayerPOVHitResult(SOURCE_ONLY): powder snow's outline before
            // a fluid source behind it.
            if let Some(hit) = self.pick()
                && self.is_powder_snow(hit.pos)
            {
                let eye = self.eye_position().unwrap_or(hit.point);
                let d = |p: [f64; 3]| {
                    (p[0] - eye[0]).powi(2) + (p[1] - eye[1]).powi(2) + (p[2] - eye[2]).powi(2)
                };
                if self
                    .pick_fluid()
                    .is_none_or(|fluid| d(fluid.point) >= d(hit.point))
                {
                    self.pick_up_powder_snow(hit.pos)?;
                    return Ok(true);
                }
            }
            let Some(hit) = self.pick_fluid() else {
                return Ok(false);
            };
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let f = self.res.facts.get(state).clone();
            if f.fluid == Fluid::None || f.fluid_level != 8 || f.fluid_falling {
                return Ok(false);
            }
            let (filled, sound) = match f.fluid {
                Fluid::Water => ("minecraft:water_bucket", "minecraft:item.bucket.fill"),
                _ => ("minecraft:lava_bucket", "minecraft:item.bucket.fill_lava"),
            };
            self.world
                .set(hit.pos[0], hit.pos[1], hit.pos[2], self.world.air)?;
            self.audio.play(sound, Some(at(hit.pos)), 1.0, 1.0);
            self.trigger_item("minecraft:filled_bucket", &ItemStack::new(filled, 1));
            if !self.creative {
                self.consume_held();
                let filled = ItemStack {
                    item: Arc::from(filled),
                    count: 1,
                    damage: 0,
                    extra: None,
                };
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
        let Some(hit) = self.pick() else {
            return Ok(false);
        };
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if item == "water_bucket" || item == "lava_bucket" {
            let target = if self.res.facts.get(clicked).replaceable {
                hit.pos
            } else {
                [
                    hit.pos[0] + hit.face[0],
                    hit.pos[1] + hit.face[1],
                    hit.pos[2] + hit.face[2],
                ]
            };
            let existing = self.world.block(target[0], target[1], target[2]);
            if !self.res.facts.get(existing).replaceable && !self.res.facts.is_air(existing) {
                return Ok(true);
            }
            let fluid = if item == "water_bucket" {
                "minecraft:water"
            } else {
                "minecraft:lava"
            };
            self.set_state_named(target, fluid, &[])?;
            let sound = if item == "water_bucket" {
                "minecraft:item.bucket.empty"
            } else {
                "minecraft:item.bucket.empty_lava"
            };
            self.audio.play(sound, Some(at(target)), 1.0, 1.0);
            if !self.creative {
                self.inventory[self.selected] = Some(ItemStack {
                    item: Arc::from("minecraft:bucket"),
                    count: 1,
                    damage: 0,
                    extra: None,
                });
            }
            return Ok(true);
        }
        if self.use_block_transformer(stack, hit.pos, hit.face)? {
            return Ok(true);
        }
        if item == "honeycomb" {
            let registry = self.res.assets.registry;
            let old = registry.state(clicked).cloned();
            if let Some(waxed) = old.and_then(|old| self.res.transformers.waxed(registry, &old))
                && let Ok(state) = registry.resolve(&waxed)
            {
                self.level_set(hit.pos, state)?;
                self.level_event_at(3003, hit.pos, 0);
                self.audio.play(
                    "minecraft:item.honeycomb.wax_on",
                    Some(at(hit.pos)),
                    1.0,
                    1.0,
                );
                if !self.creative {
                    self.consume_held();
                }
                return Ok(true);
            }
        }
        if item == "flint_and_steel" || item == "fire_charge" {
            let target = [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ];
            if self
                .res
                .facts
                .is_air(self.world.block(target[0], target[1], target[2]))
            {
                if !self.try_light_portal(target)? {
                    self.set_state_named(target, "minecraft:fire", &[])?;
                }
                let sound = horizon_client::items::fire_starter_sound(item == "fire_charge");
                let pitch = horizon_client::items::fire_starter_pitch(
                    item == "fire_charge",
                    &mut self.world.random,
                );
                self.audio.play(sound, Some(at(target)), 1.0, pitch);
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(true);
            }
        }
        // Equippable items swap into their armor slot (Equippable.swapWithEquipmentSlot);
        // block items aimed at a block are placed instead.
        if let Some(slot) = self
            .res
            .items
            .get(&stack.item)
            .filter(|i| i.block.is_none())
            .and_then(|i| i.equip_slot.clone())
        {
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
                self.audio
                    .play("minecraft:item.armor.equip_generic", Some(eye), 1.0, 1.0);
            }
            self.apply_equipment();
            return Ok(true);
        }
        Ok(false)
    }
    /// `ExperienceOrb.award`: orbs of the largest sizes; each draws the id
    /// `tryMergeToExisting` compares from the level random, then moves with
    /// its own random.
    fn spawn_orbs(&mut self, at: [f64; 3], mut value: u32) {
        while value > 0 {
            let v = ExperienceOrb::split(value);
            value -= v;
            let _merge_id = self.world.random.next_int_bound(40);
            self.add_orb(at, v);
        }
    }
    /// `new ExperienceOrb(level, pos, Vec3.ZERO, value)`: yaw and motion from
    /// the orb's own random.
    fn add_orb(&mut self, at: [f64; 3], value: u32) {
        let mut r = dispenser::entity_random();
        let _yaw = r.next_float();
        let p = Vec3::new(at[0], at[1], at[2]);
        self.orbs.push(ExperienceOrb {
            position: p,
            previous: p,
            velocity: Vec3::new(
                (r.next_double() * 0.2 - 0.1) * 2.0,
                r.next_double() * 0.2 * 2.0,
                (r.next_double() * 0.2 - 0.1) * 2.0,
            ),
            value,
            age: 0,
            on_ground: false,
        });
    }
    /// DropExperienceBlock / RedStoneOreBlock / SculkBlock experience ranges.
    fn block_experience(&mut self, name: &str) -> u32 {
        let (min, max) = match name.trim_start_matches("minecraft:") {
            "coal_ore" | "deepslate_coal_ore" => (0, 2),
            "nether_gold_ore" => (0, 1),
            "lapis_ore" | "deepslate_lapis_ore" | "nether_quartz_ore" => (2, 5),
            "diamond_ore" | "deepslate_diamond_ore" | "emerald_ore" | "deepslate_emerald_ore" => {
                (3, 7)
            }
            "redstone_ore" | "deepslate_redstone_ore" => (1, 5),
            "sculk" => (1, 1),
            "sculk_catalyst" | "sculk_sensor" | "calibrated_sculk_sensor" | "sculk_shrieker" => {
                (5, 5)
            }
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
        let spectator = self.spectator();
        let player = self
            .player
            .as_ref()
            .filter(|p| !p.health.dead_or_dying() && !spectator);
        let target = player.map(|p| {
            [
                p.position.x,
                p.position.y + f64::from(p.dimensions.eye_height) / 2.0,
                p.position.z,
            ]
        });
        let bounds = player.map(|p| {
            let w = f64::from(p.dimensions.width) / 2.0;
            (
                [p.position.x - w, p.position.y, p.position.z - w],
                [
                    p.position.x + w,
                    p.position.y + f64::from(p.dimensions.height),
                    p.position.z + w,
                ],
            )
        });
        let mut orbs = std::mem::take(&mut self.orbs);
        orbs.retain_mut(|o| o.tick(&self.world, &self.res.facts, self.collision, target));
        let mut gained = 0;
        if let Some((min, max)) = bounds {
            orbs.retain(|o| {
                let touching = o.position.x + 0.25 > min[0]
                    && o.position.x - 0.25 < max[0]
                    && o.position.y + 0.5 > min[1]
                    && o.position.y < max[1]
                    && o.position.z + 0.25 > min[2]
                    && o.position.z - 0.25 < max[2];
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
            gained = self.mending(gained);
        }
        if gained > 0 {
            let at = self.eye_position();
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.35 + 0.9;
            self.audio
                .play("minecraft:entity.experience_orb.pickup", at, 0.1, pitch);
            if self.experience.give(gained as i32)
                && self.experience.level % 5 == 0
                && self.ticks > self.last_level_up + 100
            {
                self.last_level_up = self.ticks;
                let volume = if self.experience.level > 30 {
                    1.0
                } else {
                    self.experience.level as f32 / 30.0
                };
                self.audio
                    .play("minecraft:entity.player.levelup", at, volume * 0.75, 1.0);
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
        let arrow_slot = self
            .inventory
            .iter()
            .position(|s| s.as_ref().is_some_and(|s| &*s.item == "minecraft:arrow"));
        if arrow_slot.is_none() && !self.creative {
            return;
        }
        let Some(eye) = self.eye_position() else {
            return;
        };
        // BowItem.shootProjectile: shootFromRotation(power * 3, inaccuracy 1).
        let p = Vec3::new(eye[0], eye[1] - 0.1, eye[2]);
        let (power_bonus, infinity) = self.bow_enchantments();
        let kind = horizon_client::projectiles::ProjectileKind::Arrow {
            base_damage: 2.0 + power_bonus,
            critical: power >= 1.0,
            effect: None,
        };
        let mut arrow = horizon_client::projectiles::Projectile::new(
            kind,
            p,
            Some(horizon_client::mobs::Who::Player),
        );
        let (velocity, on_ground) = self
            .player
            .as_ref()
            .map_or((Vec3::ZERO, true), |pl| (pl.velocity, pl.ground.on_ground));
        arrow.shoot_from_rotation(
            &mut self.world.random,
            self.pitch,
            self.yaw,
            0.0,
            power * 3.0,
            1.0,
            velocity,
            on_ground,
        );
        self.creatures.projectiles.push(arrow);
        self.player_projectile_shot();
        let pitch = 1.0 / (self.world.random.next_float() * 0.4 + 1.2) + power * 0.5;
        self.audio
            .play("minecraft:entity.arrow.shoot", Some(eye), 1.0, pitch);
        if !self.creative
            && !infinity
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
    /// FirstPersonHandsAndItemsRenderer: the right arm or held item in view
    /// space, converted to camera-relative world space.
    fn hand_geometry(&mut self, alpha: f64) -> Vec<Vertex> {
        let mut out = Vec::new();
        // GameRenderer.renderItemInHand: F1 hides the hand with the HUD.
        if self.spectator()
            || self.ui.hide_gui
            || !self.camera_type.first_person()
            || self
                .player
                .as_ref()
                .is_none_or(|p| p.health.dead_or_dying())
            || self.screen == Screen::Dead
        {
            return out;
        }
        let a = alpha as f32;
        // FirstPersonHandsAndItemsRenderer: the swing of the hand that
        // swings, and its inverse arm height (the swap scale is 1 for every
        // item but spears).
        let attack = if self.swing.attack_hand() == horizon_client::first_person::Hand::Main {
            self.swing.animation(a)
        } else {
            0.0
        };
        let inverse_height = 1.0 - self.heights.interpolated(a)[0];
        let view = horizon_client::camera::view_matrix(self.yaw, self.pitch);
        // The hand sway: both hands turn by a tenth of the bobs' lag.
        let [sway_x, sway_y] = self.sway.rotation(a, self.pitch, self.yaw);
        let to_world = view.inverse()
            * glam::Mat4::from_rotation_x(sway_x.to_radians())
            * glam::Mat4::from_rotation_y(sway_y.to_radians());
        let eye = self.eye_position().unwrap_or_default();
        let light = self.world.light(
            eye[0].floor() as i32,
            eye[1].floor() as i32,
            eye[2].floor() as i32,
        );
        let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
        let deg = |d: f32| d.to_radians();
        let sqrt_attack = attack.sqrt();
        if self.held_map_geometry(to_world, attack, inverse_height, packed, &mut out) {
            return out;
        }
        let [main_item, _] = self.visible_hand_items();
        match main_item {
            None => {
                let Some(arm) = self.res.player_arm.clone() else {
                    return out;
                };
                let Some(r) = self
                    .res
                    .atlas
                    .sprites
                    .get("entity/player/wide/steve")
                    .copied()
                else {
                    return out;
                };
                // renderPlayerArm (right arm)
                let x_swing = -0.3 * (sqrt_attack * std::f32::consts::PI).sin();
                let y_swing = 0.4 * (sqrt_attack * std::f32::consts::TAU).sin();
                let z_swing = -0.4 * (attack * std::f32::consts::PI).sin();
                let z_rot = (attack * attack * std::f32::consts::PI).sin();
                let y_rot = (sqrt_attack * std::f32::consts::PI).sin();
                let m = to_world
                    * glam::Mat4::from_translation(glam::Vec3::new(
                        x_swing + 0.64,
                        y_swing - 0.6 + inverse_height * -0.6,
                        z_swing - 0.72,
                    ))
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
                horizon_client::mobs::render_model(
                    &arm,
                    m,
                    &pose,
                    [r.u0, r.v0, r.u1, r.v1],
                    packed,
                    0,
                    &mut out,
                );
            }
            Some(stack) => {
                // applyItemArmTransform + swingArm (applyItemArmAttackTransform)
                let x = -0.4 * (sqrt_attack * std::f32::consts::PI).sin();
                let y = 0.2 * (sqrt_attack * std::f32::consts::TAU).sin();
                let z = -0.2 * (attack * std::f32::consts::PI).sin();
                let y_swing_rot = (attack * attack * std::f32::consts::PI).sin();
                let xz_swing_rot = (sqrt_attack * std::f32::consts::PI).sin();
                let m = to_world
                    * glam::Mat4::from_translation(glam::Vec3::new(
                        0.56,
                        -0.52 + inverse_height * -0.6,
                        -0.72,
                    ))
                    * glam::Mat4::from_translation(glam::Vec3::new(x, y, z))
                    * glam::Mat4::from_rotation_y(deg(45.0 + y_swing_rot * -20.0))
                    * glam::Mat4::from_rotation_z(deg(xz_swing_rot * -20.0))
                    * glam::Mat4::from_rotation_x(deg(xz_swing_rot * -80.0))
                    * glam::Mat4::from_rotation_y(deg(-45.0));
                // `ItemInHandRenderer.renderItem`: the player as owner, seeded by
                // its id plus the display context ordinal.
                let owner = self.player_owner(Some(&stack));
                let context = horizon_assets::item_model::DisplayContext::FirstPersonRightHand;
                let state = self.resolve_item(&stack, context, Some(&owner), 4);
                let light = self.world.light(
                    eye[0].floor() as i32,
                    eye[1].floor() as i32,
                    eye[2].floor() as i32,
                );
                self.submit_item_world(
                    &state,
                    m,
                    horizon_client::block_entity_render::vertex_light(light),
                    &mut out,
                );
            }
        }
        out
    }
    /// The renderer for the window, with the current block sprites, shared
    /// atlas and world border texture.
    fn new_renderer(&self, window: Arc<Window>) -> Result<Renderer> {
        let assets = &self.res.assets;
        pollster::block_on(Renderer::new(
            window,
            self.options.offscreen,
            &assets.pixels,
            assets.sprite_size,
            assets.texture_layers,
            &assets.animations,
            (
                &self.res.atlas.pixels,
                self.res.atlas.width,
                self.res.atlas.height,
            ),
            (&self.res.forcefield.2, self.res.forcefield.0),
        ))
    }
    fn apply_settings(&mut self) {
        self.streaming.radius = self.settings.render_distance;
        self.streaming.biome_blend = self.settings.biome_blend;
        self.streaming.smooth_lighting = self.settings.smooth_lighting;
        self.streaming.cutout_leaves = self.settings.cutout_leaves;
        self.audio.master = self.settings.master_volume;
        horizon_client::hud::GUI_SCALE.store(
            self.settings.gui_scale,
            std::sync::atomic::Ordering::Relaxed,
        );
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
        let t = if y > 80 {
            b.temperature - (y - 80) as f32 * 0.05 / 40.0
        } else {
            b.temperature
        };
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
        let (rain, _) = self.world.weather.levels(1.0);
        let Some(eye) = self.eye_position() else {
            return;
        };
        // WeatherEffectRenderer.tickRainParticles: occasional rain sounds.
        if rain > 0.0 && self.world.random.next_int_bound(3).unwrap_or(1) < (rain * 3.0) as i32 {
            let r = &mut self.world.random;
            let (dx, dz) = (
                r.next_int_bound(21).unwrap_or(10) - 10,
                r.next_int_bound(21).unwrap_or(10) - 10,
            );
            let (x, z) = (eye[0].floor() as i32 + dx, eye[2].floor() as i32 + dz);
            let top = self.rain_height(x, z, eye[1] as i32 + 10);
            if self.precipitation(x, top, z) == Some(false)
                && (top - eye[1] as i32).abs() < 10
                && self.world.random.next_int_bound(5).unwrap_or(1) == 0
            {
                let above = f64::from(top) > eye[1] + 1.0;
                let (name, volume, pitch) = if above {
                    ("minecraft:weather.rain.above", 0.1, 0.5)
                } else {
                    ("minecraft:weather.rain", 0.2, 1.0)
                };
                self.audio.play(
                    name,
                    Some([f64::from(x) + 0.5, f64::from(top), f64::from(z) + 0.5]),
                    volume,
                    pitch,
                );
            }
        }
    }
    /// WeatherEffectRenderer: rain and snow columns within the weather radius.
    fn weather_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        use horizon_client::weather;
        let (intensity, _) = self.world.weather.levels(alpha as f32);
        if intensity <= 0.0 {
            return;
        }
        let atlas = &self.res.atlas;
        let (Some(rain_tex), Some(snow_tex)) = (
            atlas.sprites.get("environment/rain").copied(),
            atlas.sprites.get("environment/snow").copied(),
        ) else {
            return;
        };
        let (cx, cy, cz) = (
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        );
        let radius = self.settings.weather_radius;
        let ticks = self.world.game_time;
        let partial = alpha as f32;
        let eye = [camera.x, camera.y, camera.z];
        // Vanilla draws every rain column before the snow ones.
        let mut columns = (Vec::new(), Vec::new());
        for z in cz - radius..=cz + radius {
            for x in cx - radius..=cx + radius {
                let top = self.rain_height(x, z, cy + radius + 10);
                let y0 = (cy - radius).max(top);
                let y1 = (cy + radius).max(top);
                if y1 == y0 {
                    continue;
                }
                let Some(snow) = self.precipitation(x, cy, z) else {
                    continue;
                };
                let mut random = weather::column_random(x, z);
                let light = self.world.light(x, top.max(cy), z);
                let coords = i32::from(light & 15) << 4 | i32::from(light >> 4) << 20;
                if snow {
                    columns.1.push(weather::snow_column(
                        &mut random,
                        ticks,
                        x,
                        y0,
                        y1,
                        z,
                        coords,
                        partial,
                    ));
                } else {
                    columns.0.push(weather::rain_column(
                        &mut random,
                        ticks,
                        x,
                        y0,
                        y1,
                        z,
                        coords,
                        partial,
                    ));
                }
            }
        }
        let rain = columns.0.iter().map(|c| (c, 1.0, rain_tex));
        let snow = columns.1.iter().map(|c| (c, 0.8, snow_tex));
        for (column, max_alpha, r) in rain.chain(snow) {
            let quad = weather::column_quad(column, eye, max_alpha, radius, intensity);
            let color = quad[0].color;
            let lc = quad[0].light_coords;
            let light = ((lc >> 20 & 15) * 16) as u32 | (((lc >> 4 & 15) * 16) as u32) << 8;
            weather::tile(&quad, |pos, uvs| {
                for (p, [u, v]) in pos.into_iter().zip(uvs) {
                    out.push(Vertex {
                        normal: [0; 4],
                        pos: p,
                        uv: [r.u0 + (r.u1 - r.u0) * u, r.v0 + (r.v1 - r.v0) * v],
                        layer: 0,
                        color,
                        light,
                    });
                }
            });
        }
    }
    /// Player.startSleepInBed: night or thunder, no monsters within 8 blocks;
    /// the bed becomes the respawn point either way.
    fn try_sleep(&mut self, pos: [i32; 3]) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let time = self.world.day_time.rem_euclid(24000);
        let night = (12542..23460).contains(&time) || self.world.weather.thundering;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let props = self
            .res
            .assets
            .registry
            .state(state)
            .map(|s| s.properties.clone())
            .unwrap_or_default();
        let head = if props.get("part").map(String::as_str) == Some("foot") {
            let d = facing_offset(props.get("facing").map_or("north", String::as_str));
            [pos[0] + d[0], pos[1], pos[2] + d[2]]
        } else {
            pos
        };
        self.spawn = Some(Vec3::new(
            f64::from(head[0]) + 0.5,
            f64::from(head[1]) + 0.6,
            f64::from(head[2]) + 0.5,
        ));
        self.level.spawn = Some([
            f64::from(head[0]),
            f64::from(head[1]) + 1.0,
            f64::from(head[2]),
        ]);
        let dist = ((p.position.x - f64::from(head[0]) - 0.5).powi(2)
            + (p.position.z - f64::from(head[2]) - 0.5).powi(2))
        .sqrt();
        if !night {
            self.message = Some((
                "You can sleep only at night or during thunderstorms".into(),
                Instant::now(),
            ));
            return;
        }
        if dist > 3.0 {
            self.message = Some((
                "You may not rest now; the bed is too far away".into(),
                Instant::now(),
            ));
            return;
        }
        let monsters = self.mobs.iter().any(|m| {
            m.hostile()
                && m.alive()
                && (m.position.x - p.position.x).abs() < 8.0
                && (m.position.y - p.position.y).abs() < 5.0
                && (m.position.z - p.position.z).abs() < 8.0
        });
        if monsters {
            self.message = Some((
                "You may not rest now; there are monsters nearby".into(),
                Instant::now(),
            ));
            return;
        }
        self.message = Some(("Respawn point set".into(), Instant::now()));
        self.sleeping = Some((head, 0));
        self.stats.custom("sleep_in_bed", 1);
        self.trigger_player("minecraft:slept_in_bed");
        self.stats.reset("custom", "minecraft:time_since_rest");
    }
    fn tick_sleep(&mut self) {
        let Some((head, ticks)) = self.sleeping else {
            return;
        };
        if self.keys.contains(&KeyCode::ShiftLeft)
            || self
                .player
                .as_ref()
                .is_none_or(|p| p.health.dead_or_dying())
        {
            self.sleeping = None;
            return;
        }
        if let Some(p) = self.player.as_mut() {
            p.position = Vec3::new(
                f64::from(head[0]) + 0.5,
                f64::from(head[1]) + 0.5625,
                f64::from(head[2]) + 0.5,
            );
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
        batch.centered_origin(Creative::WIDTH, Creative::HEIGHT)
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
            format!(
                "gui/sprites/container/creative_inventory/tab_{}_{}_{}",
                if t.top { "top" } else { "bottom" },
                if on { "selected" } else { "unselected" },
                t.column + 1
            )
        };
        batch.fill(
            &self.res.atlas,
            0.0,
            0.0,
            batch.width,
            batch.height,
            rgba(16, 16, 16, 160),
        );
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
            let stack = ItemStack {
                item: Arc::from(t.icon.as_str()),
                count: 1,
                damage: 0,
                extra: None,
            };
            self.draw_item(
                batch,
                &stack,
                ox + tx + 5.0,
                oy + ty + if t.top { 9.0 } else { 7.0 },
            );
        }
        let atlas = &self.res.atlas;
        batch.text(
            atlas,
            &tab.title,
            ox + 8.0,
            oy + 6.0,
            rgba(64, 64, 64, 255),
            false,
        );
        if tab.search {
            let shown = format!("{}_", self.creative_screen.search);
            batch.text(atlas, &shown, ox + 82.0, oy + 6.0, u32::MAX, true);
        }
        let items = self.creative_screen.items(&self.res.names);
        let first = self.creative_screen.first_row(&self.res.names);
        let mut hovered: Option<String> = None;
        for r in 0..5 {
            for c in 0..9 {
                let Some(stack) = items.get((first + r) * 9 + c) else {
                    continue;
                };
                let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 18.0 + r as f32 * 18.0);
                self.draw_item(batch, stack, x, y);
                if self.cursor.0 >= x
                    && self.cursor.0 < x + 16.0
                    && self.cursor.1 >= y
                    && self.cursor.1 < y + 16.0
                {
                    batch.fill(&self.res.atlas, x, y, 16.0, 16.0, rgba(255, 255, 255, 128));
                    hovered = Some(stack.item.to_string());
                }
            }
        }
        for c in 0..9 {
            let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 112.0);
            if let Some(stack) = self.inventory[c].clone() {
                self.draw_item(batch, &stack, x, y);
                if self.cursor.0 >= x
                    && self.cursor.0 < x + 16.0
                    && self.cursor.1 >= y
                    && self.cursor.1 < y + 16.0
                {
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
        batch.sprite(
            &self.res.atlas,
            scroller,
            ox + Creative::SCROLL_X,
            oy + Creative::SCROLL_Y + self.creative_screen.scroller_offset(),
        );
        for t in &tabs {
            let (tx, ty) = Creative::tab_position(t);
            if self.cursor.0 >= ox + tx
                && self.cursor.0 < ox + tx + 26.0
                && self.cursor.1 >= oy + ty
                && self.cursor.1 < oy + ty + 32.0
            {
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
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
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
                self.creative_screen.select(i);
                return;
            }
        }
        // Holding the scrollbar scrolls as the mouse moves (mouseDragged).
        if !right
            && self
                .creative_screen
                .press_scrollbar(cx - ox, cy - oy, &self.res.names)
        {
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
                let Some(stack) = items.get((first + r) * 9 + c) else {
                    return;
                };
                let max = self.res.items.get(&stack.item).map_or(64, |i| i.max_stack);
                let stack = ItemStack {
                    count: if right { 1 } else { max },
                    ..stack.clone()
                };
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
    /// Copy the live player and world state into the `Level` that gets saved.
    pub(super) fn sync_level(&mut self) {
        // Level.write saves the game rules (game_rules.dat) with the rest.
        self.level.game_rules = self.ui.rules.by_name();
        self.level.selected_slot = self.selected as i32;
        let mode = self.mode_id();
        let level = &mut self.level;
        level.creative = self.creative;
        level.game_mode = mode;
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
            .filter_map(|(i, s)| {
                s.as_ref()
                    .map(|s| (i as u8, s.item.to_string(), s.count, s.damage))
            })
            .collect();
        level.inventory_components = self
            .inventory
            .iter()
            .enumerate()
            .filter_map(|(i, s)| Some((i as u8, s.as_ref()?.extra.clone()?)))
            .collect();
        if let Some(s) = &self.offhand {
            level
                .inventory
                .push((150, s.item.to_string(), s.count, s.damage));
            if let Some(extra) = s.extra.clone() {
                level.inventory_components.insert(150, extra);
            }
        }
        // Vanilla stores armor in slots 100 (feet) to 103 (head).
        for (i, s) in self.armor.iter().enumerate() {
            if let Some(s) = s {
                level
                    .inventory
                    .push((103 - i as u8, s.item.to_string(), s.count, s.damage));
                if let Some(extra) = s.extra.clone() {
                    level.inventory_components.insert(103 - i as u8, extra);
                }
            }
        }
        level.dimension = self.dim.id().to_owned();
        // An open ender chest holds the live copy.
        if let Some(key) = self.ender_open
            && let Some(BlockEntity::Chest(items)) = self.world.block_entities.get(&key)
        {
            self.ender_items = items.clone();
        }
        level.ender_items = self.ender_items.clone();
        level.active_effects = self
            .potions
            .effects
            .list
            .iter()
            .map(|e| {
                (
                    format!("minecraft:{}", e.id),
                    e.duration,
                    e.amplifier,
                    e.ambient,
                    e.visible,
                )
            })
            .collect();
    }
    fn save_game(&mut self) -> Result<()> {
        if self.remote.is_some() {
            return Ok(());
        }
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks: Vec<_> = keys
            .iter()
            .filter_map(|k| self.world.chunks.get(k))
            .map(|c| self.with_entities(c))
            .collect();
        self.save
            .save(&chunks, self.res.assets.registry, &self.res.biomes)?;
        self.save_entities()?;
        self.save_maps();
        self.save_random_sequences();
        self.save_raids();
        self.save_fight();
        self.save_wandering_trader();
        self.save_stats();
        self.save_advancements();
        self.save_scoreboard();
        self.save_bossbars();
        self.save_schedule();
        self.save_warden_tracker();
        self.sync_level();
        self.level.write(&self.world_root)?;
        self.save_world_state();
        self.message = Some(("Saved the game".into(), Instant::now()));
        Ok(())
    }
    fn render(&mut self) -> Result<bool> {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return Ok(false);
        };
        let alpha = self.clock.fraction();
        let live = self.backdrop_live();
        let view = if live {
            self.backdrop_view()
        } else {
            self.camera_view(alpha as f32)
        };
        let (camera, underwater, view_yaw, view_pitch) = match view {
            Some((c, yaw, pitch)) => {
                let s =
                    self.world
                        .block(c.x.floor() as i32, c.y.floor() as i32, c.z.floor() as i32);
                (c, self.res.facts.get(s).fluid == Fluid::Water, yaw, pitch)
            }
            None => (
                glam::DVec3::new(0.5, 100.0, 0.5),
                false,
                self.yaw,
                self.pitch,
            ),
        };
        self.audio
            .set_listener([camera.x, camera.y, camera.z], view_yaw);
        let block = [
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        ];
        let biome = self
            .world
            .chunks
            .get(&(block[0] >> 4, block[2] >> 4))
            .map(|c| c.biome(block[0], block[1], block[2]))
            .unwrap_or(0);
        let info = self.res.biomes.get(biome).clone();
        let environment = self.dim_environment();
        let mut sky = environment.sample(self.world.day_time, info.sky, info.fog);
        let (rain, thunder) = if self.dim == Dim::Overworld {
            self.world.weather.levels(alpha as f32)
        } else {
            (0.0, 0.0)
        };
        horizon_client::environment::apply_weather(&mut sky, rain, thunder);
        self.end_flash_sky(&mut sky, alpha);
        if self.flash > 0 {
            // Lightning flash brightens the sky (LevelRenderer skyFlashTime).
            sky.sky_color = [0.9, 0.9, 1.0];
            sky.sky_light_factor = 1.0;
        }
        // DebugRenderer: chunk borders and hitboxes (F3+G, F3+B).
        let (debug_lines, debug_lines_on_top) = if self.show_hitboxes || self.show_chunk_borders {
            let g = self.debug_gizmos(alpha as f32);
            let view = horizon_client::camera::view_matrix(view_yaw, view_pitch);
            let at = camera.to_array();
            (
                horizon_client::gizmos::camera_relative(&g.lines, at, view),
                horizon_client::gizmos::camera_relative(&g.on_top, at, view),
            )
        } else {
            (Vec::new(), Vec::new())
        };
        // LevelRenderer.renderHitOutline: the outline shape's edges.
        let outline = if self.screen == Screen::Playing && !self.spectator() {
            self.pick()
                .map(|h| {
                    let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
                    let f = self.res.facts.get(state);
                    let o = if f.outline_offset {
                        self.res
                            .context
                            .tables
                            .offset(state, h.pos[0], h.pos[2])
                            .unwrap_or([0.0; 3])
                    } else {
                        [0.0; 3]
                    };
                    let p = h.pos.map(f64::from);
                    f.outline_edges
                        .iter()
                        .map(|e| std::array::from_fn(|i| p[i % 3] + o[i % 3] + e[i]))
                        .collect()
                })
                .unwrap_or_default()
        } else {
            Vec::new()
        };
        let mut hud = horizon_client::spans::time("hud", || self.hud(size));
        // Map textures changed since the last drawn frame (game/maps.rs).
        hud.patches.append(&mut self.map_patches);
        let title = self.title.is_some();
        let (yaw, pitch) = if live {
            (view_yaw, view_pitch)
        } else if title {
            // PanoramaRenderer: the title cube spins at 0.1 degrees per tick, pitched 10 degrees.
            (-(self.started.elapsed().as_secs_f32() * 2.0) % 360.0, 10.0)
        } else {
            (view_yaw, view_pitch)
        };
        let fov_degrees = if live {
            72.0
        } else if title {
            85.0
        } else {
            self.fov_degrees(alpha as f32, camera.to_array())
        };
        let bob = if title {
            glam::Mat4::IDENTITY
        } else {
            self.bob_matrix(alpha as f32)
        };
        let distance = (self.streaming.radius * 16) as f32;
        let depth_far =
            horizon_client::camera::depth_far(self.streaming.radius, self.settings.cloud_range);
        self.view_frustum = self
            .renderer
            .as_ref()
            .map(|r| r.camera_frustum(fov_degrees, yaw, pitch, bob, depth_far));
        let mut entities = if title {
            Vec::new()
        } else {
            horizon_client::spans::time("entity_geometry", || self.entity_geometry(camera, alpha))
        };
        let mut additive = Vec::new();
        if !title {
            self.bolt_geometry(camera, &mut additive);
            if self.dim == Dim::End {
                let mut dragon = Vec::new();
                self.dragon_geometry(camera, alpha, &mut dragon, &mut additive);
                self.dragon_fireball_geometry(camera, alpha, &mut dragon);
                entities.extend(dragon);
            }
        }
        let celestial = if title && !live {
            self.panorama_geometry()
        } else if self.dim == Dim::Overworld
            && self.potions.effects.get("darkness").is_none()
            && self.potions.effects.get("blindness").is_none()
        {
            // Camera.doesMobEffectBlockSky: no sun, moon or stars.
            self.celestial_geometry(&sky)
        } else if self.dim == Dim::End {
            self.end_sky_geometry()
        } else {
            Vec::new()
        };
        let hand = if title || self.screen == Screen::Inventory || self.scoping() {
            Vec::new()
        } else {
            self.hand_geometry(alpha)
        };
        let screen = if title {
            Vec::new()
        } else {
            self.screen_quads(alpha as f32, size)
        };
        if !title {
            self.end_geometry(camera, &mut entities);
        }
        if (!title || live) && self.dim == Dim::Overworld {
            horizon_client::spans::time("clouds", || {
                self.cloud_geometry(&sky, camera, alpha, &mut entities)
            });
            if self.settings.horizon.weather {
                horizon_client::spans::time("weather", || {
                    self.weather_geometry(camera, alpha, &mut entities)
                });
            }
        }
        let water_fog = {
            let c = info.water_fog;
            [
                (c >> 16 & 255) as f32 / 255.0,
                (c >> 8 & 255) as f32 / 255.0,
                (c & 255) as f32 / 255.0,
            ]
        };
        let border = (!title)
            .then(|| self.border_draw(camera.to_array(), alpha as f32))
            .flatten();
        let frame = Frame {
            nether_lighting: self.dim == Dim::Nether,
            camera,
            yaw,
            pitch,
            fov_degrees,
            bob,
            sky,
            distance,
            depth_far,
            underwater,
            title: title && !live,
            skybox: self.dim == Dim::Overworld && self.settings.horizon.sky,
            end_sky: self.dim == Dim::End && self.settings.horizon.sky,
            environment_fog: (self.dim == Dim::Nether).then_some((sky.fog_start, sky.fog_end)),
            // The camera inside an opaque block sees nothing through it: disable.
            cave_culling: self.settings.horizon.cave_culling
                && !self
                    .res
                    .facts
                    .get(self.world.block(block[0], block[1], block[2]))
                    .solid_render,
            water_fog,
            fog: Some({
                let mut fog = self.frame_fog(camera, &sky, biome);
                // Horizon's fog setting: no distance fog (fluids keep theirs).
                if !self.settings.horizon.fog && !underwater && fog.environmental_end >= distance {
                    (fog.render_distance_start, fog.render_distance_end) = (1e9, 2e9);
                    (fog.environmental_start, fog.environmental_end) = (1e9, 2e9);
                }
                fog
            }),
            dark_disc: self.dark_disc(camera),
            outline,
            debug_lines: &debug_lines,
            debug_lines_on_top: &debug_lines_on_top,
            hud: &hud,
            entities: &entities,
            additive: &additive,
            celestial: if self.settings.horizon.sky || title {
                &celestial
            } else {
                &[]
            },
            hand: &hand,
            hand_fov_degrees: self.hud_fov_degrees(alpha as f32, camera.to_array()),
            screen: &screen,
            border,
            game_time: self.world.game_time as f32,
            shader_game_time: ((self.world.game_time % 24000) as f32 + alpha as f32) / 24000.0,
            crumbling_sprite: self
                .mining
                .as_ref()
                .and_then(|m| {
                    self.res.atlas.sprites.get(&format!(
                        "block/destroy_stage_{}",
                        ((m.progress * 10.0) as u32).min(9)
                    ))
                })
                .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0]),
            end_portal_sprites: ["environment/end_sky", "entity/end_portal/end_portal"].map(|n| {
                self.res
                    .atlas
                    .sprites
                    .get(n)
                    .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0])
            }),
            ticks: self.ticks,
            darkness: self.darkness_lighting(alpha as f32),
            night_vision: self.night_vision_intensity(),
            block_flicker: self.res.fog.state.block_light_flicker,
            post: if live {
                Some(horizon_client::post::PostGrade::FROST)
            } else if self.horizon_pause() {
                Some(horizon_client::post::PostGrade::PAUSE)
            } else {
                None
            },
        };
        let renderer = self.renderer.as_mut().ok_or("renderer")?;
        horizon_client::spans::time("renderer", || renderer.render(&frame))
    }
    /// `LevelExtractor` / `LevelRenderer`: the world border wall this frame.
    fn border_draw(
        &mut self,
        camera: [f64; 3],
        partial: f32,
    ) -> Option<horizon_client::gfx::BorderDraw> {
        use horizon_client::border_render::{self, BorderRenderer};
        let distance = f64::from(self.streaming.radius * 16);
        border_render::extract(
            &self.border,
            partial,
            camera,
            distance,
            &mut self.border_state,
        );
        // Camera.depthFar, as the renderer's far plane.
        let depth_far =
            horizon_client::camera::depth_far(self.streaming.radius, self.settings.cloud_range);
        if !self
            .border_renderer
            .prepare(&self.border_state, camera, distance, depth_far)
        {
            return None;
        }
        let millis = self.started.elapsed().as_millis() as i64;
        Some(horizon_client::gfx::BorderDraw {
            vertices: self.border_renderer.vertices,
            rebuilt: self.border_renderer.take_rebuilt(),
            transforms: self
                .border_renderer
                .transforms(&self.border_state, camera, millis),
            sides: BorderRenderer::sides(&self.border_state, camera, distance),
        })
    }
    /// A key press while a world is open (keyboard or control socket).
    fn key_pressed(&mut self, key: KeyCode) -> Result<()> {
        if self.remote.is_some() && self.remote_key(key)? {
            return Ok(());
        }
        match key {
            KeyCode::Escape => match self.screen {
                Screen::Options => self.options_key(KeyCode::Escape, None)?,
                Screen::Playing => {
                    self.screen = Screen::Paused;
                    self.pause_menu = true;
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
                    self.pause_menu = true;
                    self.capture(true);
                }
                Screen::Dead => {}
                Screen::Stats => self.screen = Screen::Paused,
                Screen::Advancements => self.close_advancements(),
                Screen::WorldOptions => self.close_world_options(),
                Screen::Chat => self.close_chat(),
            },
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9
            | KeyCode::KeyF
            | KeyCode::KeyQ
                if self.screen == Screen::Inventory && self.inventory_hotkey(key) => {}
            KeyCode::KeyT if self.screen == Screen::Inventory => self.recipe_focus_search(),
            KeyCode::KeyT if self.screen == Screen::Playing => self.open_chat(""),
            // key.advancements.
            KeyCode::KeyL if self.screen == Screen::Playing => self.open_advancements(false),
            KeyCode::KeyL if self.screen == Screen::Advancements => self.close_advancements(),
            KeyCode::Slash if self.screen == Screen::Playing => self.open_chat("/"),
            KeyCode::F1 => self.toggle_hide_gui(),
            KeyCode::F11 => self.toggle_fullscreen(),
            KeyCode::F2 => self.take_screenshot(),
            KeyCode::F5 if self.screen == Screen::Playing => self.cycle_camera(),
            KeyCode::KeyE
                if self.screen == Screen::Creative && self.creative_screen.tab().search => {}
            KeyCode::KeyE => match self.screen {
                Screen::Playing if self.spectator() => {}
                Screen::Playing => {
                    // key.inventory while riding a tamed mount opens its inventory.
                    if !self.open_ridden_mount_menu() {
                        self.screen = if self.creative {
                            Screen::Creative
                        } else {
                            Screen::Inventory
                        };
                        self.capture(false);
                    }
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
            // key.swapOffhand: ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND.
            KeyCode::KeyF if self.screen == Screen::Playing => {
                std::mem::swap(&mut self.offhand, &mut self.inventory[self.selected]);
                self.using = None;
            }
            KeyCode::KeyQ if self.screen == Screen::Playing => {
                let all = self.keys.contains(&KeyCode::ControlLeft);
                self.drop_held(all);
            }
            KeyCode::Digit1
            | KeyCode::Digit2
            | KeyCode::Digit3
            | KeyCode::Digit4
            | KeyCode::Digit5
            | KeyCode::Digit6
            | KeyCode::Digit7
            | KeyCode::Digit8
            | KeyCode::Digit9 => {
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
        if self.remote.is_some() {
            return self.remote_mouse(button, pressed);
        }
        match button {
            MouseButton::Left => {
                // KeyMapping.click: Minecraft.tick starts the attack.
                if pressed {
                    self.attack_clicks += 1;
                }
                self.toggles
                    .button(2, pressed, false, self.settings.toggle_attack);
                self.left = self.toggles.is_down(2);
            }
            MouseButton::Right => {
                self.toggles
                    .button(3, pressed, false, self.settings.toggle_use);
                self.right = self.toggles.is_down(3);
                if pressed {
                    self.use_with_hand()?;
                    self.use_delay = 4;
                }
            }
            MouseButton::Middle if pressed && self.creative => {
                if let Some(hit) = self.pick() {
                    let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
                    let item = self.res.facts.get(state).item.clone();
                    if &*item != "minecraft:air" {
                        self.inventory[self.selected] = Some(ItemStack {
                            item,
                            count: 1,
                            damage: 0,
                            extra: None,
                        });
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    fn update_world(&mut self) {
        if let Err(e) = self.poll_remote() {
            log::warn!("Horizon: {e}");
        }
        if let Some(p) = &self.player {
            self.streaming.center = (
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            );
        }
        let (uploads, unloaded) =
            horizon_client::spans::time("streaming", || self.streaming.update(&mut self.world));
        let dirty: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        let modified: Vec<_> = horizon_client::spans::time("with_entities", || {
            dirty.iter().map(|c| self.with_entities(c)).collect()
        });
        horizon_client::spans::time("unload_entities", || self.unload_entities(&unloaded));
        if !unloaded.is_empty() {
            let gone: horizon_client::fast_hash::FxHashSet<(i32, i32)> =
                unloaded.iter().map(|c| (c.x, c.z)).collect();
            self.world
                .block_entities
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
            self.world
                .block_names
                .retain(|p, _| !gone.contains(&(p.0 >> 4, p.2 >> 4)));
        }
        if !modified.is_empty()
            && let Err(e) = horizon_client::spans::time("save_chunks", || {
                self.save
                    .save_later(&modified, self.res.assets.registry, &self.res.biomes)
            })
        {
            log::warn!("Horizon: failed to save unloaded chunks: {e}");
        }
        if let Some(renderer) = self.renderer.as_mut() {
            let g = &self.settings.horizon;
            let budgeted = g.chunk_updates > 0 || g.upload_budget_kib > 0;
            if !budgeted && self.pending_uploads.is_empty() {
                horizon_client::spans::time("upload", || {
                    for (key, mesh) in uploads {
                        renderer.upload(key, &mesh);
                    }
                });
            } else {
                // Horizon's chunk-update and upload budgets: the nearest
                // sections go first, the rest wait for later frames; a newer
                // mesh of a waiting section replaces it.
                let pending = &mut self.pending_uploads;
                let fresh: horizon_client::fast_hash::FxHashSet<(i32, i32, i32)> =
                    uploads.iter().map(|(k, _)| *k).collect();
                let gone: horizon_client::fast_hash::FxHashSet<(i32, i32)> =
                    unloaded.iter().map(|c| (c.x, c.z)).collect();
                pending.retain(|(k, _)| !fresh.contains(k) && !gone.contains(&(k.0, k.2)));
                pending.extend(uploads);
                let centre = self.streaming.center;
                let far = |k: &(i32, i32, i32)| (k.0 - centre.0).pow(2) + (k.2 - centre.1).pow(2);
                pending.sort_unstable_by_key(|(k, _)| std::cmp::Reverse(far(k)));
                let (limit, budget) = if budgeted {
                    (
                        g.chunk_updates as usize,
                        u64::from(g.upload_budget_kib) * 1024,
                    )
                } else {
                    (0, 0)
                };
                let (mut count, mut bytes) = (0, 0u64);
                while let Some((_, mesh)) = pending.last() {
                    let size = ((mesh.solid.len() + mesh.translucent.len()) * 32) as u64;
                    if (limit > 0 && count >= limit)
                        || (budget > 0 && bytes > 0 && bytes + size > budget)
                    {
                        break;
                    }
                    let (key, mesh) = pending.pop().expect("checked above");
                    renderer.upload(key, &mesh);
                    count += 1;
                    bytes += size;
                }
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
        // The Window constructor: created fullscreen when options.txt says so.
        let fullscreen = self.settings.fullscreen;
        self.fullscreen = horizon_client::window::FullscreenState::new(fullscreen);
        let window = match event_loop.create_window(
            Window::default_attributes()
                .with_title("Horizon")
                .with_inner_size(winit::dpi::LogicalSize::new(1280.0, 720.0))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None))),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
                return;
            }
        };
        match self.new_renderer(window.clone()) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                self.error = Some(e);
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
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && let PhysicalKey::Code(key) = event.physical_key
        {
            if event.state == ElementState::Pressed {
                self.physical_keys.insert(key);
            } else {
                self.physical_keys.remove(&key);
            }
        }
        if let WindowEvent::Focused(false) = &event {
            self.physical_keys.clear();
        }
        if let WindowEvent::ModifiersChanged(m) = &event {
            self.modifiers = m.state();
        }
        // KeyboardHandler.keyPress: the debug keys come before the game's.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && let PhysicalKey::Code(key) = key_event.physical_key
            && self.title.is_none()
        {
            use horizon_client::debug_keys::KeyAction;
            let action = match (key_event.state, key_event.repeat) {
                (ElementState::Released, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            if self.debug_key(key, action) {
                return;
            }
        }
        if matches!(
            event,
            WindowEvent::KeyboardInput { .. }
                | WindowEvent::MouseInput { .. }
                | WindowEvent::MouseWheel { .. }
                | WindowEvent::CursorMoved { .. }
        ) {
            self.last_input = Instant::now();
        }
        if let WindowEvent::MouseInput {
            state,
            button: MouseButton::Left,
            ..
        } = &event
        {
            // The press state of Horizon-style buttons.
            horizon_client::ui_theme::set_pressed(*state == ElementState::Pressed);
        }
        let result: Result<()> = (|| {
            match event {
                WindowEvent::CloseRequested => {
                    self.save_for_exit()?;
                    event_loop.exit();
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                    self.observe_fullscreen();
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    if !focused && self.control.is_none() {
                        self.keys.clear();
                        self.left = false;
                        self.right = false;
                        self.capture(false);
                        // Screens (books, signs) stay open instead of pausing.
                        if self.screen == Screen::Playing
                            && self.player.is_some()
                            && !self.options.no_pause
                            && self.settings.file.bool("pauseOnLostFocus")
                            && self.book.is_none()
                            && self.sign_edit.is_none()
                        {
                            self.screen = Screen::Paused;
                        }
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    if let Some(r) = &self.renderer {
                        let b = HudBatch::new([r.config.width, r.config.height]);
                        self.cursor = (position.x as f32 / b.scale, position.y as f32 / b.scale);
                        if self.screen == Screen::Advancements {
                            self.advancements_drag();
                        }
                        if self.screen == Screen::Creative {
                            let (_, oy) = Self::creative_origin(&b);
                            self.creative_screen.drag_scrollbar(self.cursor.1 - oy);
                        }
                        if self.screen == Screen::Inventory {
                            self.loom_drag();
                            self.inventory_drag_move();
                        }
                        if self.dragging && self.screen == Screen::Options {
                            let _ = b;
                            self.options_drag()?;
                        }
                    }
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let (raw_x, raw_y) = match delta {
                        MouseScrollDelta::LineDelta(x, y) => (f64::from(x), f64::from(y)),
                        MouseScrollDelta::PixelDelta(p) => (p.x / 30.0, p.y / 30.0),
                    };
                    // MouseHandler.onScroll: discrete scrolling and the wheel sensitivity.
                    let discrete = self.settings.discrete_scroll;
                    let wheel_sensitivity = f64::from(self.settings.mouse_wheel_sensitivity);
                    let scaled = |v: f64| {
                        (if discrete {
                            horizon_client::mouse::java_signum(v)
                        } else {
                            v
                        }) * wheel_sensitivity
                    };
                    let d = scaled(raw_y) as f32;
                    if self.screen == Screen::Playing
                        && self.player.is_some()
                        && let Some((wheel, _)) =
                            self.mouse
                                .on_scroll(raw_x, raw_y, discrete, wheel_sensitivity)
                    {
                        self.selected = horizon_client::mouse::ScrollWheelHandler::next_selection(
                            f64::from(wheel),
                            self.selected as i32,
                            9,
                        ) as usize;
                    }
                    if self.screen == Screen::Chat && d != 0.0 {
                        self.chat_scroll(d);
                    }
                    if self.screen == Screen::Options && d != 0.0 {
                        self.options_scroll(d);
                    }
                    if self.screen == Screen::Creative && d != 0.0 {
                        self.creative_screen.wheel(f64::from(d), &self.res.names);
                    }
                    if self.screen == Screen::Inventory && d != 0.0 {
                        self.station_scroll(d);
                    }
                    if self.screen == Screen::Stats && d != 0.0 {
                        self.stats_scroll(d);
                    }
                    if self.screen == Screen::Advancements {
                        self.advancements_scroll(scaled(raw_x) as f32, d);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.screen == Screen::Options => {
                    if self.options_waiting_for_key() && state == ElementState::Pressed {
                        self.options_bind_mouse(button)?;
                    } else if matches!(button, MouseButton::Left | MouseButton::Right) {
                        self.options_mouse(
                            state == ElementState::Pressed,
                            button == MouseButton::Right,
                        )?;
                    }
                }
                WindowEvent::MouseInput {
                    state,
                    button: MouseButton::Left,
                    ..
                } if self.title.is_some() && state == ElementState::Pressed => {
                    let size = self
                        .renderer
                        .as_ref()
                        .map(|r| [r.config.width, r.config.height])
                        .unwrap_or([1, 1]);
                    let mut b = HudBatch::new(size);
                    let atlas = &self.res.atlas;
                    let action = self.title.as_mut().map_or(Action::None, |t| {
                        if horizon_client::ui_theme::horizon() {
                            t.click_horizon(atlas, &mut b, self.cursor.0, self.cursor.1)
                        } else {
                            t.click(self.cursor.0, self.cursor.1, (b.width, b.height))
                        }
                    });
                    match action {
                        Action::Quit => event_loop.exit(),
                        Action::Options => self.open_options(true),
                        Action::Play { folder } => self.open_world(&folder, None, None, false)?,
                        Action::Create {
                            folder,
                            name,
                            seed,
                            creative,
                        } => self.open_world(&folder, Some(&name), seed, creative)?,
                        Action::Connect { address } => {
                            self.connect_server(&address, &self.options.player_name.clone())?
                        }
                        Action::None => {}
                    }
                }
                WindowEvent::MouseInput { .. } if self.title.is_some() => {}
                WindowEvent::MouseInput { state, button, .. } if self.book.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        let size = self
                            .renderer
                            .as_ref()
                            .map(|r| [r.config.width, r.config.height])
                            .unwrap_or([1, 1]);
                        self.book_click(size);
                    }
                }
                WindowEvent::MouseInput { state, button, .. } if self.sign_edit.is_some() => {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        let size = self
                            .renderer
                            .as_ref()
                            .map(|r| [r.config.width, r.config.height])
                            .unwrap_or([1, 1]);
                        self.sign_edit_click(size);
                    }
                }
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
                            self.inventory_press(button == MouseButton::Right);
                            if let Some(key) = self.container {
                                self.container_changed(key);
                            }
                        }
                        Screen::Inventory => {
                            self.inventory_mouse_release(button == MouseButton::Right);
                            self.loom_release();
                        }
                        Screen::Chat if pressed && button == MouseButton::Left => self.chat_click(),
                        Screen::Creative if pressed => {
                            self.click_creative(button == MouseButton::Right);
                        }
                        Screen::Creative if button == MouseButton::Left => {
                            self.creative_screen.release();
                        }
                        Screen::Paused if pressed && self.pause_menu => self.pause_click()?,
                        Screen::WorldOptions if pressed && button == MouseButton::Left => {
                            self.world_options_click()
                        }
                        Screen::Dead if pressed => self.death_click()?,
                        Screen::Advancements if button == MouseButton::Left => {
                            let size = self
                                .renderer
                                .as_ref()
                                .map_or([1, 1], |r| [r.config.width, r.config.height]);
                            self.advancements_mouse(pressed, size);
                        }
                        Screen::Stats if pressed && button == MouseButton::Left => {
                            let size = self
                                .renderer
                                .as_ref()
                                .map_or([1, 1], |r| [r.config.width, r.config.height]);
                            self.stats_click(size);
                        }
                        _ => {}
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Chat && self.title.is_none() =>
                {
                    if let PhysicalKey::Code(key) = event.physical_key {
                        if event.state == ElementState::Pressed {
                            self.keys.insert(key);
                            self.chat_key(&event);
                        } else {
                            self.keys.remove(&key);
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.sign_edit.is_some() => {
                    self.sign_edit_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.book.is_some() => {
                    self.book_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Creative
                        && self.creative_screen.tab().search
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    if let Some(shortcut) = self.edit_shortcut(&event) {
                        self.creative_search_shortcut(shortcut);
                    } else {
                        let backspace = event.physical_key == PhysicalKey::Code(KeyCode::Backspace);
                        self.creative_search_edit(event.text.as_deref(), backspace);
                    }
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.recipe_search_focused()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    // The recipe book's search box takes every key but Escape.
                    self.recipe_key(&event);
                }
                WindowEvent::KeyboardInput { event, .. }
                    if self.station_typing()
                        && event.state == ElementState::Pressed
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    // The anvil rename box takes every key but Escape (AnvilScreen.keyPressed).
                    if let Some(shortcut) = self.edit_shortcut(&event) {
                        self.station_shortcut(shortcut);
                    } else {
                        let backspace = event.physical_key == PhysicalKey::Code(KeyCode::Backspace);
                        self.station_text(event.text.as_deref(), backspace);
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.remote_chat_open() => {
                    self.remote_chat_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::WorldOptions => {
                    self.world_options_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::Options => {
                    if let Some(shortcut) = self.edit_shortcut(&event)
                        && self.options_shortcut(shortcut)
                    {
                        return Ok(());
                    }
                    if event.state == ElementState::Pressed
                        && let PhysicalKey::Code(key) = event.physical_key
                    {
                        self.options_key(key, event.text.as_deref())?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.title.is_some() => {
                    if event.state == ElementState::Pressed
                        && self.title.as_ref().is_some_and(|t| !t.skip_opening())
                    {
                        if let Some(shortcut) = self.edit_shortcut(&event) {
                            self.title_shortcut(shortcut);
                            return Ok(());
                        }
                        match event.physical_key {
                            PhysicalKey::Code(KeyCode::Backspace) => self.title_edit(None, true),
                            PhysicalKey::Code(KeyCode::Tab) => {
                                if let Some(t) = self.title.as_mut() {
                                    t.tab();
                                }
                            }
                            PhysicalKey::Code(KeyCode::Escape) => {
                                if let Some(t) = self.title.as_mut() {
                                    t.escape();
                                }
                            }
                            _ => self.title_edit(event.text.as_deref(), false),
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    let PhysicalKey::Code(key) = event.physical_key else {
                        return Ok(());
                    };
                    // key.smoothCamera toggles the cinematic camera.
                    if self.screen == Screen::Playing
                        && event.state == ElementState::Pressed
                        && !event.repeat
                        && horizon_client::keybinds::key_name(key)
                            .is_some_and(|n| self.settings.keys.get("key.smoothCamera") == n)
                    {
                        self.smooth_camera = !self.smooth_camera;
                    }
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
                    // Sneak and sprint are ToggleKeyMappings: a press (or a key
                    // repeat) flips them when toggling; only releases reach them
                    // behind a screen (KeyboardHandler.keyPress).
                    let toggle = match key {
                        KeyCode::ShiftLeft => Some((0, self.settings.toggle_crouch)),
                        KeyCode::ControlLeft => Some((1, self.settings.toggle_sprint)),
                        _ => None,
                    };
                    if let Some((slot, option)) = toggle {
                        let action = match (event.state, event.repeat) {
                            (ElementState::Released, _) => KeyAction::Release,
                            (_, false) => KeyAction::Press,
                            (_, true) => KeyAction::Repeat,
                        };
                        let screen_open = self.screen != Screen::Playing || self.title.is_some();
                        self.toggles.key(slot, action, screen_open, option);
                        self.sync_toggle(slot);
                        if event.state != ElementState::Pressed {
                            return Ok(());
                        }
                    } else if event.state != ElementState::Pressed {
                        self.keys.remove(&key);
                        return Ok(());
                    } else {
                        self.keys.insert(key);
                    }
                    if event.repeat {
                        return Ok(());
                    }
                    self.key_pressed(key)?;
                }
                WindowEvent::RedrawRequested => {
                    self.observe_fullscreen();
                    let now = Instant::now();
                    let elapsed = now.duration_since(self.previous);
                    self.frame_ticks = elapsed.as_secs_f32() * 20.0;
                    self.previous = now;
                    // Arrow keys turn the camera (accessibility / scripted testing).
                    let turn = |k| f32::from(u8::from(self.keys.contains(&k)));
                    let secs = elapsed.as_secs_f32().min(0.1);
                    self.yaw +=
                        (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0 * secs;
                    self.pitch = (self.pitch
                        + (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0 * secs)
                        .clamp(-90.0, 90.0);
                    self.update_backdrop()?;
                    horizon_client::ui_theme::set_backdrop(self.backdrop_fade());
                    if self.screen != Screen::Paused {
                        self.pause_since = None;
                    }
                    if self.title.is_none() {
                        horizon_client::spans::time("update_world", || self.update_world());
                        horizon_client::spans::time("spawn_player", || self.spawn_player())?;
                    }
                    let updated = Instant::now();
                    self.track_toggle_screens();
                    let steps = self.clock.advance(elapsed);
                    for _ in 0..steps {
                        let started = Instant::now();
                        if self.title.is_none() && !self.pause {
                            horizon_client::spans::time("tick", || self.tick())?;
                            self.tick_ambience();
                            self.tick_fov();
                        } else if self.title.is_some() {
                            self.tick_backdrop();
                        }
                        self.tick_music();
                        self.tick_sound_engine();
                        self.clock.record(started.elapsed());
                        if let Some(report) = self.clock.take_sprint_report() {
                            self.sprint_report(&report);
                        }
                    }
                    self.apply_mouse_look();
                    self.advance_flight();
                    let ticked = Instant::now();
                    let rendered = self.render()?;
                    if let Some(threshold) = self.options.spikes
                        && let Some(r) = self.renderer.as_ref()
                    {
                        let cpu = now.elapsed().saturating_sub(r.last_wait).as_secs_f32() * 1000.0;
                        let spans = horizon_client::spans::take();
                        if cpu > threshold {
                            let list: Vec<String> = spans
                                .iter()
                                .filter(|(_, ms)| *ms >= 0.5)
                                .map(|(n, ms)| format!("{n} {ms:.1}"))
                                .collect();
                            log::info!(
                                "Horizon: slow frame {cpu:.1} ms ({} ticks): {}",
                                steps,
                                list.join(", ")
                            );
                        }
                    }
                    if let Some(r) = self.renderer.as_mut() {
                        let cpu = now.elapsed().saturating_sub(r.last_wait);
                        if rendered {
                            let ms = |d: Duration| d.as_secs_f32() * 1000.0;
                            self.frame_graph.record(
                                ms(elapsed),
                                ms(cpu),
                                r.gpu_times.last().copied(),
                            );
                            self.frame_stats.record_cpu(ms(cpu));
                            let render = r.last_encode;
                            self.frame_stats.record_phases([
                                ms(updated - now),
                                ms(ticked - updated),
                                ms(ticked.elapsed().saturating_sub(r.last_wait + render)),
                                ms(render),
                            ]);
                        }
                        self.frame_stats.record_gpu(&r.gpu_times);
                        r.gpu_times.clear();
                    }
                    self.limit_frame();
                    self.pause = self.pauses_game();
                    self.clock.end_frame(self.pause);
                    let finished = self
                        .renderer
                        .as_mut()
                        .and_then(|r| r.screenshot_result.take());
                    if let Some(result) = &finished
                        && self.ui.screenshot_message.is_some()
                    {
                        self.finish_screenshot(result);
                    }
                    if let Some(result) = finished
                        && let Some(reply) = self.pending_screenshot.take()
                    {
                        let _ = reply.send(
                            match result {
                                Ok([w, h]) => {
                                    serde_json::json!({ "ok": true, "width": w, "height": h })
                                }
                                Err(e) => serde_json::json!({ "error": e }),
                            }
                            .to_string(),
                        );
                    }
                    if rendered {
                        self.frame_stats.record(elapsed.as_secs_f32() * 1000.0);
                        self.frames += 1;
                        self.fps_frames += 1;
                        horizon_client::crash::debug_crash_tick();
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
            self.error = Some(e);
            event_loop.exit();
        }
    }
    fn device_event(&mut self, _: &ActiveEventLoop, _: DeviceId, event: DeviceEvent) {
        if self.captured
            && self.focused
            && self.screen == Screen::Playing
            && let DeviceEvent::MouseMotion { delta: (x, y) } = event
        {
            // MouseHandler.onMove with the pointer grabbed; applied once per frame.
            self.mouse.on_move(0.0, 0.0, x, y, true, true);
            self.last_input = Instant::now();
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.poll_control(event_loop);
        if self.options.frames.is_some() && self.started.elapsed() > self.options.timeout {
            self.error = Some("frame-limited run timed out".into());
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window {
            w.request_redraw();
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
    /// The application is ending. On macOS, Quit (Cmd+Q) ends it through
    /// applicationWillTerminate without a CloseRequested; vanilla's GLFW
    /// window turns it into a window close, so Minecraft.stop saves the world.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        if let Err(e) = self.save_for_exit() {
            log::warn!("Horizon: failed to save the world on exit: {e}");
        }
    }
}
/// clouds.png as filled cells (`CloudRenderer.prepare`: alpha 10 or more).
fn load_clouds(root: &std::path::Path) -> (u32, u32, Vec<bool>) {
    horizon_client::hud::decode(&root.join("assets/minecraft/textures/environment/clouds.png"))
        .map(|(w, h, px)| {
            (
                w,
                h,
                px.as_chunks::<4>().0.iter().map(|p| p[3] >= 10).collect(),
            )
        })
        .unwrap_or((1, 1, vec![false]))
}
/// misc/forcefield.png for the world border wall (square, power of two).
fn load_forcefield(root: &std::path::Path) -> (u32, u32, Vec<u8>) {
    horizon_client::hud::decode(&root.join("assets/minecraft/textures/misc/forcefield.png"))
        .ok()
        .filter(|(w, h, _)| w == h && w.is_power_of_two())
        .unwrap_or((1, 1, vec![0; 4]))
}
pub fn run() -> Result<()> {
    let Some(mut options) = Options::read()? else {
        return Ok(());
    };
    if options.dedicated.is_some() && !dedicated::preflight(&mut options)? {
        return Ok(());
    }
    let started = Instant::now();
    horizon_client::spans::enable(options.spikes.is_some());
    let vanilla_root = options.resources.clone();
    let early_settings = Settings::load(&std::env::current_dir()?.join("options.txt"));
    options.resources = packs::startup_root(&vanilla_root, &early_settings.resource_packs);
    let assets = match Assets::load(&options.resources) {
        Ok(assets) => assets,
        // Minecraft.rollbackResourcePacks: the packs break loading, start
        // with the game's own resources.
        Err(e) if options.resources != vanilla_root => {
            log::warn!("Horizon: resource packs failed to load ({e}); using the default resources");
            options.resources = vanilla_root.clone();
            Assets::load(&vanilla_root)?
        }
        Err(e) => {
            return Err(format!(
                "{e}\nPrepare resources with the launcher, or: horizon-prepare --prepare"
            )
            .into());
        }
    };
    let root = options.resources.clone();
    let assets = Arc::new(assets);
    log::info!("load assets: {:.2}s", started.elapsed().as_secs_f32());
    let facts = BlockFacts::load(&root, assets.registry)?;
    let mob_data = horizon_client::mobs::MobData::load(&root, &facts);
    log::info!("load facts: {:.2}s", started.elapsed().as_secs_f32());
    let items = Items::load(&root, assets.registry, &facts)?;
    log::info!("load items: {:.2}s", started.elapsed().as_secs_f32());
    let recipes = Recipes::load(&root, &items)?;
    let transformers = horizon_client::block_transform::Transformers::load(&root, assets.registry)?;
    let biomes = Arc::new(Biomes::load(&root)?);
    let environment = Environment::load(&root)?;
    log::info!("load biomes+env: {:.2}s", started.elapsed().as_secs_f32());
    let visibility = models::load_visibility(&root, assets.registry)?;
    log::info!("load visibility: {:.2}s", started.elapsed().as_secs_f32());
    let tables = Arc::new(MeshTables::new(&assets, &facts, visibility)?);
    let light = Arc::new(LightTables::load(&root, assets.registry, &facts)?);
    log::info!("load tables: {:.2}s", started.elapsed().as_secs_f32());
    let atlas = HudAtlas::build(&root)?;
    horizon_client::ui::load_metadata(&root);
    horizon_client::lang::set(horizon_client::lang::Language::load(
        &[&root],
        &early_settings.language,
    ));
    log::info!("load atlas: {:.2}s", started.elapsed().as_secs_f32());
    let layer_names = assets
        .sprite_layers
        .iter()
        .map(|(name, layer)| (*layer, name.trim_start_matches("minecraft:").to_owned()))
        .collect();
    let registry_owner = assets.clone();
    let collision = collisions::load(&root, registry_owner.registry)?;
    let motion = MotionResources::load(&root, &registry_owner)?;
    log::info!(
        "load collision+motion: {:.2}s",
        started.elapsed().as_secs_f32()
    );
    let context = Arc::new(WorkerContext {
        assets: assets.clone(),
        tables,
        biomes: biomes.clone(),
        light,
    });
    let threads = early_settings.horizon.worker_threads();
    log::info!(
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
    let save = Arc::new(SaveDir::new(
        std::env::current_dir()?
            .join("saves")
            .join(".horizon-title"),
    ));
    let audio = if options.dedicated.is_some() {
        Audio::silent(&root)
    } else {
        Audio::new(&root)
    };
    let world = World::new(facts.air, 0);
    let creative = options.creative;
    let stations = horizon_client::stations::StationData::load(&root, &items);
    let animate =
        horizon_client::animate_tick::AnimateTables::load(&root, registry_owner.registry, &biomes)?;
    let fog = horizon_client::fog::FogRenderer::load(&root, &biomes);
    let mut resources = Resources {
        stations,
        assets,
        facts,
        items,
        recipes,
        transformers,
        biomes,
        environment,
        atlas,
        item_renderer: Arc::new(horizon_client::item_render::ItemRenderer::load(&root)?),
        layer_names,
        providers: Providers::load(&root),
        block_entity_render: horizon_client::block_entity_render::BlockEntityRenderer::load(
            &root,
            registry_owner.registry,
        )?,
        animate,
        fog,
        music: horizon_client::music::MusicTables::load(&root),
        ambient: horizon_client::ambience::AmbientTables::load(&root),
        screen_fx: Default::default(),
        nether_environment: Environment::load_dimension(&root, "the_nether", false)?,
        end_environment: Environment::load_dimension(&root, "the_end", false)?,
        behaviours: Behaviours::load(registry_owner.registry, &root)?,
        names: Names::load(&root),
        clouds: load_clouds(&root),
        forcefield: load_forcefield(&root),
        mob_types: MobTypes::load(&root)?
            .types
            .into_iter()
            .map(|(k, v)| (k, Arc::new(v)))
            .collect(),
        mob_data,
        context,
        threads,
        player_type: perspective::player_type(&root),
        player_arm: horizon_client::mobs::load_model(&root, "minecraft:player#main")
            .and_then(|m| m.children.into_iter().find(|c| c.name == "right_arm")),
    };
    let inventory = vec![None; 36];
    let now = Instant::now();
    let game_root = options.resources.clone();
    let mut settings = Settings::load(&std::env::current_dir()?.join("options.txt"));
    horizon_client::ui_theme::load_preferences(&std::env::current_dir()?);
    // Scripted and frame-limited runs (the parity harnesses) see vanilla's
    // screens unless HORIZON_MENU_STYLE asks otherwise.
    if (options.frames.is_some() || options.control.is_some())
        && std::env::var_os("HORIZON_MENU_STYLE").is_none()
    {
        horizon_client::ui_theme::set_style(horizon_client::ui_theme::MenuStyle::Classic);
    }
    if let Some(d) = options.distance_override {
        settings.render_distance = d;
    }
    let look = options.look;
    let particles = horizon_client::particles::ParticleEngine::new(&root, &resources.atlas.sprites);
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
        inv_drag: None,
        inv_clicks: Default::default(),
        last_inv_click: None,
        container: None,
        station: None,
        potions: Default::default(),
        scratch_slot: None,
        mining: None,
        destroy_delay: 0,
        use_delay: 0,
        creative,
        mode: if creative {
            horizon_client::command::args::GameMode::Creative
        } else {
            horizon_client::command::args::GameMode::Survival
        },
        keys: HashSet::new(),
        physical_keys: HashSet::new(),
        left: false,
        right: false,
        cursor: (0.0, 0.0),
        captured: false,
        focused: true,
        modifiers: winit::keyboard::ModifiersState::empty(),
        fullscreen: horizon_client::window::FullscreenState::new(false),
        mouse: MouseLook::default(),
        agent_pointer: false,
        smooth_camera: false,
        toggles: Default::default(),
        was_playing: false,
        pause: false,
        debug: false,
        screen: Screen::Playing,
        window: None,
        renderer: None,
        health_hud: hud_layout::HealthHud::default(),
        started: now,
        previous: now,
        last_input: now,
        frames: 0,
        fps: 0.0,
        fps_frames: 0,
        fps_at: now,
        ticks: 0,
        error: None,
        message: None,
        save,
        level,
        session_lock: None,
        audio,
        music: Default::default(),
        ambience: Default::default(),
        sound_paused: false,
        dropped: Vec::new(),
        next_entity: 0,
        particles,
        sign_edit: None,
        book: None,
        lectern_pulses: Vec::new(),
        spawn_search: 0,
        mobs: Vec::new(),
        arrows: Vec::new(),
        creatures: Default::default(),
        sculk: Default::default(),
        raiding: Default::default(),
        attack_ticks: 100,
        last_hurt_by_player: HashSet::new(),
        died: HashSet::new(),
        be_anims: {
            let mut anims = horizon_client::block_entity_anim::BlockEntityAnims::default();
            anims.classify(registry_owner.registry);
            anims
        },
        using: None,
        view: Default::default(),
        limiter: horizon_client::window::FramerateLimiter::new(0),
        frame_clock: Default::default(),
        scope_scale: 0.5,
        frame_ticks: 0.0,
        title: None,
        armor: [None, None, None, None],
        offhand: None,
        shield_cooldown: 0,
        command_source: None,
        titles: Default::default(),
        tool_highlight: Default::default(),
        applied_modifiers: Vec::new(),
        experience: Experience::default(),
        orbs: Vec::new(),
        xp_delay: 0,
        last_level_up: 0,
        swing: Default::default(),
        swings_started: 0,
        heights: Default::default(),
        sway: Default::default(),
        visible_hands: [(0, None), (0, None)],
        clicks: Default::default(),
        attack_clicks: 0,
        hand_strength: Default::default(),
        settings,
        options_from_title: false,
        backdrop: Default::default(),
        pause_since: None,
        dragging: false,
        avatar: Default::default(),
        flash: 0,
        sleeping: None,
        dim: Dim::Overworld,
        world_root: PathBuf::new(),
        travelling: None,
        portal_ticks: 0,
        portal_cooldown: 0,
        end_travel: false,
        end_portal_travel: false,
        end: Default::default(),
        block_ticks: Ticks::new(0),
        tick_chunks: HashSet::new(),
        level_events: Vec::new(),
        falling: Vec::new(),
        paintings: Vec::new(),
        painting_variants: paintings::PaintingVariants::load(&root),
        item_frames: Vec::new(),
        frame_model: frames::bake_frame(&root, "item_frame"),
        glow_frame_model: frames::bake_frame(&root, "glow_item_frame"),
        map_frame_model: frames::bake_frame(&root, "item_frame_map"),
        glow_map_frame_model: frames::bake_frame(&root, "glow_item_frame_map"),
        armor_stands: Vec::new(),
        others: Vec::new(),
        armor_stand_assets: armor_stands::ArmorStandAssets::load(&root),
        boats: Vec::new(),
        boat_models: boats::BoatModels::load(&root),
        carts: Vec::new(),
        jukebox_sounds: HashMap::new(),
        bolts: Vec::new(),
        maps: maps::Maps::default(),
        fight: dragon::Fight::default(),
        dragon: None,
        crystals: Vec::new(),
        dragon_fireballs: Vec::new(),
        breath_clouds: Vec::new(),
        dragon_model: horizon_client::mobs::load_model(&root, "minecraft:ender_dragon#main"),
        map_patches: Vec::new(),
        skin_downloads: horizon_client::skins::Downloader::new(
            std::env::current_dir()
                .unwrap_or_default()
                .join("assets/skins"),
        ),
        skin_slots: Vec::new(),
        cart_models: minecarts::CartModels::load(&root),
        riding: None,
        ender_items: vec![None; 27],
        ender_open: None,
        pending: Default::default(),
        spill: Default::default(),
        recipe_ui: Default::default(),
        mount_menu: Default::default(),
        hook: None,
        tridents: Vec::new(),
        rockets: Vec::new(),
        creaking: Default::default(),
        trident_model: horizon_client::mobs::load_model(&root, "minecraft:trident#main"),
        tnts: Vec::new(),
        hoppers: HashMap::new(),
        creative_screen: Creative::load(&game_root)?,
        control: None,
        frame_stats: Default::default(),
        pending_screenshot: None,
        pending_bench: None,
        flight: None,
        agent: agent::Agent::default(),
        view_frustum: None,
        pending_uploads: Vec::new(),
        frame_graph: Default::default(),
        sky_darken_memo: Default::default(),
        persist: Default::default(),
        remote: None,
        lan: None,
        world_options: None,
        debug_keys: Default::default(),
        show_hitboxes: false,
        show_chunk_borders: false,
        pause_menu: true,
        previous_mode: None,
        clipboard: Default::default(),
        text_selection: Default::default(),
        saved_for_exit: false,
        forced_hit: None,
        lan_events: Vec::new(),
        lan_acting: None,
        chat_capture: None,
        cmds: Default::default(),
        cheats: true,
        ui: Default::default(),
        combat: Default::default(),
        mount: Default::default(),
        stats: Default::default(),
        score: Default::default(),
        fns: Default::default(),
        bosses: Default::default(),
        exec: Default::default(),
        rand_sequences: Default::default(),
        border: Default::default(),
        border_renderer: Default::default(),
        border_state: Default::default(),
        tab_list: Default::default(),
        subtitles: Default::default(),
        adv: Default::default(),
        stats_ui: Default::default(),
        adv_ui: Default::default(),
        camera_type: Default::default(),
        fangs: Default::default(),
        player_model: None,
    };
    game.ui.vanilla_root = vanilla_root;
    game.res.atlas.font.uniform = game.settings.force_unicode;
    game.res.atlas.font.japanese = game.settings.japanese_glyphs;
    game.ui.chat.options = game.settings.chat_options();
    if let Some(path) = &game.options.control {
        let server = horizon_client::control::Server::start(path)
            .map_err(|e| format!("control socket {}: {e}", path.display()))?;
        log::info!("Horizon: control socket {}", server.path().display());
        game.control = Some(server);
    }
    if let Ok(spec) = std::env::var("HORIZON_DISPENSE_FIXTURE") {
        return game.dispense_fixture(&spec);
    }
    if let Ok(spec) = std::env::var("HORIZON_SCULK_FIXTURE") {
        return game.sculk_fixture(&spec);
    }
    if game.options.dedicated.is_some() {
        return game.run_dedicated();
    }
    match game.options.world.clone() {
        _ if game.options.server.is_some() => {
            let (address, name) = (
                game.options.server.clone().unwrap_or_default(),
                game.options.player_name.clone(),
            );
            game.connect_server(&address, &name)?;
            if let Some(report) = std::env::var_os("HORIZON_NET_REPORT") {
                return game.run_net_harness(report.as_ref());
            }
        }
        Some(folder) => {
            let (seed, creative) = (game.options.seed, game.options.creative);
            game.open_world(&folder, None, Some(seed), creative)?;
        }
        None => {
            let title = Title::new(std::env::current_dir()?.join("saves"));
            // The launcher's opening, once per session and only in Horizon's style.
            if horizon_client::ui_theme::horizon() && game.options.frames.is_none() {
                title
                    .opening
                    .set(Some(horizon_client::front::Opening::new()));
            }
            game.title = Some(title);
        }
    }
    if let (Some(script), Some(output)) = (
        std::env::var_os("HORIZON_COMMAND_SCRIPT"),
        std::env::var_os("HORIZON_COMMAND_OUTPUT"),
    ) {
        return game.run_command_harness(script.as_ref(), output.as_ref());
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
            "error": game.error.as_ref().map(|e| e.to_string()),
        });
        std::fs::write(path, serde_json::to_vec_pretty(&report)?)?;
    }
    if let Some(e) = game.error {
        return Err(e);
    }
    Ok(())
}
