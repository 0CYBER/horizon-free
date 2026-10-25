//! The playable game loop: streaming world, player simulation at 20 ticks/s,
//! block interaction, inventory and HUD.
mod advancements;
mod advancements_screen;
mod agent;
mod agent_settings;
mod allays;
mod armor_stands;
mod banners;
mod beacon;
mod beehives;
mod block_entity_anims;
mod block_entity_ticks;
mod block_items;
mod blockctl;
mod boat_passengers;
mod boats;
mod books;
mod bossbars;
mod breezes;
mod cartography;
mod chat;
mod clipboard;
mod combat;
mod command_blocks;
mod command_harness;
mod commands;
mod composter;
mod consume;
mod control;
mod copper_golems;
mod creaking;
mod creatures;
mod crossbow;
mod debug_keys;
mod decorated_pot;
mod dedicated;
mod deflect;
mod discord;
mod dispense_fixture;
mod dispenser;
mod dragon;
mod dynamic_lights;
mod enchant_effects;
mod end;
mod entity_culling;
mod events;
mod execution_role;
mod explosion;
mod extras;
mod falling_damage;
mod fangs;
mod fireworks;
mod fishing;
mod fov;
mod frames;
mod friends;
mod functions;
mod game_mode;
mod game_rules_screen;
mod gamemode_switcher;
mod ghasts;
mod golems;
mod growth;
mod hand;
mod initial_spawn;
mod integrated;
mod item_draw;
mod item_particles;
mod jukebox;
mod lan;
mod lan_actions;
mod lan_attributes;
mod lan_chat;
mod lan_crafting;
mod lan_entities;
mod lan_mobs;
mod lan_mount_menu;
mod lan_particles;
mod lan_pvp;
mod lan_screen;
mod lan_stations;
mod lan_trading;
mod lan_vehicles;
mod lan_waypoints;
mod leads;
mod level_particles;
mod levels;
mod lightning;
mod llamas;
mod locator_bar;
mod loom;
mod mace;
mod machines;
mod maps;
mod minecarts;
mod mob_blocks;
mod mob_gear;
mod mods;
mod mount_menu;
mod mounts;
mod moving_pistons;
mod music;
mod names;
mod natural_spawning;
mod nautili;
mod net_harness;
mod nether_portal;
mod ominous_item_spawner;
mod options;
mod packs;
mod pad;
mod pad_menu;
mod paintings;
mod passengers;
mod persist;
mod perspective;
mod pickups;
mod piglins;
mod player_equip;
mod postprocess;
mod potions;
mod powder_snow;
mod pumpkin;
mod raids;
mod recipe_book;
mod remote;
mod remote_mob_data;
mod remote_mob_use;
mod remote_objects;
mod remote_projectiles;
mod remote_stations;
mod remote_vehicles;
mod render_control;
mod render_tick;
mod respawn_anchor;
mod saves_control;
mod saving;
mod scoreboard;
mod screens;
mod sculk;
mod sculk_fixture;
mod server_packs;
mod server_travel;
mod shader_packs;
mod shield;
mod sign_edit;
mod sniffers;
mod spawner;
mod spill;
mod startup_menu;
mod station_control;
mod stations;
mod stats;
mod stats_screen;
mod storage;
mod structures;
mod subtitle_hud;
mod sulfur_cubes;
mod text_displays;
mod titles;
mod tnt;
mod totem;
mod trading;
mod trial_spawner;
mod trial_visuals;
mod trident;
mod tweaks;
mod ui_control;
mod vault;
mod villagers;
mod wandering_trader;
mod win;
mod wind;
mod wither;
mod world_loading;
mod zombie_villagers;
use horizon_client::{
    Result,
    assets::{Assets, RenderAssets},
    audio::Audio,
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    chunk::{HEIGHT, MIN_Y},
    clock::FixedStep,
    collisions,
    containers::{BlockEntity, Providers},
    crafting::Recipes,
    creative::{Creative, Names},
    entities::{Experience, ExperienceOrb, ItemEntity},
    environment::Environment,
    first_person::{Hand, SwingAnimation},
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
    collision_context::{CollisionContext, CollisionEntity, CollisionItem},
    entity_obstruction::{EntityIdentity, EntityObstructionWorld, ObstructionEntity},
    level::{
        Behaviours, Dimension as LevelDimension, EntityQuery, Landing, LevelAccess, LevelEvent,
        PlaceContext, PlaceOutcome, PlayerContext, PlayerEffect, Ticks, flags as level_flags,
    },
    registry::BlockStateId,
};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicU32, Ordering},
        mpsc::{Receiver, TryRecvError},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};
use winit::{
    application::ApplicationHandler,
    event::{DeviceEvent, DeviceId, ElementState, MouseButton, MouseScrollDelta, WindowEvent},
    event_loop::{ActiveEventLoop, ControlFlow, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::{CursorGrabMode, Window, WindowId},
};

#[derive(Clone)]
struct Options {
    resources: PathBuf,
    distance: i32,
    distance_override: Option<i32>,
    seed: i64,
    creative: bool,
    test_terrain: bool,
    frames: Option<u64>,
    /// `--world-frames N`: exit after N frames drawn in the world (smoke runs; the world may load slowly).
    world_frames: Option<u64>,
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
    /// `--width` and `--height` (vanilla's own arguments): the window's
    /// starting size, 1280x720 by default.
    window_size: [u32; 2],
    /// Multiplayer: `--server HOST[:PORT]` joins at start; `--name` is the offline player name.
    server: Option<String>,
    /// `--name`, `HORIZON_PLAYER_NAME` or the signed-in account's name; else
    /// [`Options::player_name`] uses the kept offline name.
    player_name: Option<String>,
    /// `--dedicated`: run as a dedicated server; vanilla's server arguments
    /// (`--nogui`, `--port`, `--world`, `--universe`, `--initSettings`) follow.
    dedicated: Option<Vec<String>>,
    /// `--benchmark SCENARIO`: run the benchmark, write its report and quit.
    benchmark: Option<String>,
    /// `--dev-mod PATH` (repeatable): mods always on and reloaded when rebuilt.
    dev_mods: Vec<PathBuf>,
    /// Single player runs on an integrated server thread (game/integrated.rs),
    /// as vanilla's does: the default. `--in-frame` (or `--no-server-thread`,
    /// or `HORIZON_SERVER_THREAD=0`) runs the world in the frame instead.
    server_thread: bool,
    /// What this Game executes; independent of its current server connection.
    execution_role: execution_role::ExecutionRole,
}
impl Options {
    /// The name to join servers with: the one given, else the offline name kept
    /// next to options.txt (created on first use).
    fn player_name(&self) -> String {
        self.player_name.clone().unwrap_or_else(|| {
            std::env::current_dir().map_or_else(
                |_| horizon_client::offline_name::generate(0),
                |dir| horizon_client::offline_name::load_or_create(&dir),
            )
        })
    }
    fn read() -> Result<Option<Self>> {
        let sibling = std::env::current_exe()?
            .parent()
            .ok_or("missing executable directory")?
            .join("resources");
        let mut o = Self {
            resources: if sibling.join("horizon-resources.json").exists() {
                sibling
            } else {
                std::path::Path::new(".local/horizon-client").join(horizon_core::VANILLA_VERSION)
            },
            distance: 12,
            distance_override: None,
            seed: 0,
            creative: false,
            test_terrain: false,
            frames: None,
            world_frames: None,
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
            window_size: [1280, 720],
            spikes: std::env::var("HORIZON_SPIKES")
                .ok()
                .and_then(|v| v.parse().ok()),
            server: None,
            player_name: std::env::var("HORIZON_PLAYER_NAME")
                .ok()
                .filter(|n| !n.is_empty()),
            dedicated: None,
            benchmark: None,
            dev_mods: Vec::new(),
            // The command harness (HORIZON_COMMAND_SCRIPT) drives the world
            // itself, without the event loop: it stays in the frame.
            server_thread: std::env::var("HORIZON_SERVER_THREAD").map_or(true, |v| v != "0")
                && std::env::var_os("HORIZON_COMMAND_SCRIPT").is_none(),
            execution_role: execution_role::ExecutionRole::Client,
        };
        let mut session_file = std::env::var("HORIZON_SESSION_FILE").ok();
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--help" | "-h" => {
                    println!(
                        "Horizon — Minecraft Java Edition {} in Rust\n\n  --resources PATH        Prepared resources (horizon-prepare)\n  --render-distance N     Chunks, 2..64 (default 12)\n  --world NAME            World folder under saves/ (default \"New World\")\n  --seed N                World seed for new worlds (default 0)\n  --width N, --height N   Window size (default 1280 x 720)\n  --creative              Creative mode (default survival)\n  --test-terrain          Simple test terrain instead of vanilla generation\n  --frames N              Exit after N frames (testing)\n  --world-frames N        Exit after N frames drawn in the world (testing)\n  --smoke-report PATH     Write a JSON report on exit\n  --benchmark SCENARIO    Run a benchmark (standard, village, forest, ocean, mountain), write its report to benchmarks/ and quit\n  --control               Accept horizon-ctl commands on a local socket\n  --control-socket PATH   Control socket path (default $XDG_RUNTIME_DIR/horizon-control.sock)\n  --server HOST[:PORT]    Join a multiplayer server (offline mode) at start\n  --name NAME             Player name for multiplayer (default: generated once, kept in horizon-player.txt)\n  --session-file PATH     Signed-in account from the launcher (read once, then deleted)\n  --dev-mod PATH          A Horizon mod (.hmod or folder) to develop: always on, reloaded when rebuilt\n  --in-frame              Run single player's world in the frame, not on its server thread\n\nControls: mouse look, WASD move, Space jump, Left Shift sneak, Left Ctrl sprint,\nleft click break, right click place/use, 1-9/scroll hotbar, E inventory,\nQ drop, F3 debug, Esc pause.",
                        horizon_core::VANILLA_VERSION
                    );
                    return Ok(None);
                }
                "--dedicated" => {
                    o.execution_role = execution_role::ExecutionRole::DedicatedServer;
                    o.dedicated = Some(args.by_ref().collect());
                    break;
                }
                "--creative" => o.creative = true,
                "--test-terrain" => o.test_terrain = true,
                "--no-pause" => o.no_pause = true,
                "--offscreen" => o.offscreen = true,
                "--control" => o.control = Some(horizon_client::control::default_path()),
                "--server-thread" => o.server_thread = true,
                "--in-frame" | "--no-server-thread" => o.server_thread = false,
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
                        "--world-frames" => {
                            o.world_frames = Some(value.parse()?);
                            // A frame-limited run in every other respect; --timeout-seconds still applies.
                            o.frames.get_or_insert(u64::MAX);
                        }
                        "--timeout-seconds" => o.timeout = Duration::from_secs(value.parse()?),
                        "--smoke-report" => o.report = Some(value.into()),
                        "--screenshot-after" => o.screenshot_after = Some(value.parse()?),
                        "--width" => o.window_size[0] = value.parse::<u32>()?.max(1),
                        "--height" => o.window_size[1] = value.parse::<u32>()?.max(1),
                        "--world" => o.world = Some(value),
                        "--dev-mod" => o.dev_mods.push(std::path::absolute(&value)?),
                        "--benchmark" => {
                            if horizon_client::benchmark::scenes(&value).is_none() {
                                return Err(format!(
                                    "unknown benchmark {value}: standard, village, forest, ocean or mountain"
                                )
                                .into());
                            }
                            o.benchmark = Some(value);
                        }
                        "--server" => o.server = Some(value),
                        "--name" => o.player_name = Some(value),
                        "--session-file" => session_file = Some(value),
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
        // The launcher's account: online-mode servers need it. A bad file
        // only leaves the game offline.
        if let Some(path) = session_file.filter(|p| !p.is_empty()) {
            match horizon_client::auth::AuthSession::load(path.as_ref()) {
                Ok(session) => {
                    o.player_name = Some(session.name.clone());
                    horizon_client::auth::set_current(session);
                }
                Err(e) => eprintln!("Horizon: not signed in: {e}"),
            }
        }
        Ok(Some(o))
    }
}
#[derive(Clone, Copy, PartialEq, Eq)]
enum Screen {
    Playing,
    GameModeSwitcher,
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
    /// Play with Friends, from the pause menu's Friends button
    /// (`friends_screen`, game/friends.rs).
    Friends,
    /// CommandBlockEditScreen (game/command_blocks.rs).
    CommandBlock,
    /// WinScreen: the end poem and the credits (game/win.rs).
    Win,
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
/// What a falling block's tick did (`FallingBlockEntity.tick`).
enum FallStep {
    Falling,
    Gone,
    /// Gave up falling: drops itself as an item here.
    Dropped(Vec3),
    /// Hit the ground in this block.
    Landed([i32; 3]),
}
/// A guest's right click comes as one packet per hand and part
/// (`Minecraft.startUseItem`): `ServerGamePacketListenerImpl.handleUseItemOn`
/// does the block's and `ItemStack.useOn`'s part, `handleUseItem` the
/// item's own (`Item.use`). Running both for either would use the item twice.
#[derive(Clone, Copy)]
enum GuestUse {
    OnBlock(Hand),
    Item(Hand),
    /// `handleInteract`: `Player.interactOn` with the hand, nothing else (the
    /// client sends UseItem itself when the entity passed).
    Entity(Hand),
}

/// The client world as seen by the vanilla block-behaviour library.
struct GameLevel<'w> {
    world: &'w mut World,
    facts: &'w BlockFacts,
    collision: &'w BlockCollisionRegistry<'w>,
    events: &'w mut Vec<LevelEvent>,
    dimension: LevelDimension,
    darken: i32,
    raining: bool,
    thundering: bool,
    player_box: Option<([f64; 3], [f64; 3])>,
    /// The LAN guests in this level who trigger blocks (not spectators).
    guest_boxes: Vec<([f64; 3], [f64; 3])>,
    player_collision: Option<PlayerCollision>,
    /// Minecart boxes (detector rails).
    cart_boxes: Vec<([f64; 3], [f64; 3])>,
    /// The other entities that keep blocks from being placed into them.
    blockers: Blockers<'w>,
    biomes: &'w Biomes,
    seed: i64,
    behaviours: &'w Behaviours,
    items: &'w Items,
    /// Container loot tables (`unpackLootTable`).
    loot: Option<std::sync::Arc<horizon_world::loot::LootData>>,
    registry: &'static horizon_world::registry::BlockRegistry,
    rules: horizon_world::level::GameRules,
    /// Block loot random sequences (`ServerLevel.getRandomSequence`).
    rand_sequences: &'w mut horizon_client::random_sequences::RandomSequences,
    pending: &'w mut spill::Pending,
    spill: &'w mut spill::Spill,
    /// Where the chunk tickets let blocks tick.
    tickets: horizon_client::ticket_levels::Levels,
    random_tick_sections: &'w mut horizon_client::random_tick_sections::RandomTickSections,
}
struct PlayerCollision {
    y: f64,
    descending: bool,
    fall_distance: f64,
    main_hand_item: Arc<str>,
    main_hand_count: i32,
    leather_boots: bool,
}
/// The entities `Entity.blocksBuilding` sets besides the players and
/// minecarts, borrowed for `Level.isUnobstructed`: living entities (mobs,
/// armor stands), boats, end crystals, primed TNT and falling blocks.
#[derive(Clone, Copy)]
struct Blockers<'w> {
    mobs: &'w [Mob],
    armor_stands: &'w [horizon_client::entities::ArmorStand],
    boats: &'w [boats::Boat],
    crystals: &'w [dragon::Crystal],
    tnts: &'w [tnt::PrimedTnt],
    falling: &'w [FallingBlock],
}
/// The boxes of the entities that block building, as obstruction candidates.
struct BuildingObstruction(Vec<Aabb>);
impl EntityObstructionWorld for BuildingObstruction {
    type Error = std::convert::Infallible;
    fn obstruction_candidates(
        &mut self,
        _excluded: Option<EntityIdentity>,
        bounds: Aabb,
    ) -> std::result::Result<Vec<ObstructionEntity>, Self::Error> {
        Ok(self
            .0
            .iter()
            .filter(|b| b.intersects(bounds))
            .enumerate()
            .map(|(i, &bounds)| ObstructionEntity {
                identity: EntityIdentity(i as u64),
                root_vehicle: EntityIdentity(i as u64),
                bounds,
                removed: false,
                spectator: false,
                blocks_building: true,
            })
            .collect())
    }
}
impl GameLevel<'_> {
    /// Every entity box that blocks building: the players who aren't
    /// spectators, the minecarts and the `Blockers` (a removed mob or a marker
    /// armor stand, whose box is empty, doesn't).
    fn building_blocker_boxes(&self) -> Vec<Aabb> {
        let boxed = |(lo, hi): &([f64; 3], [f64; 3])| {
            Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            )
        };
        let b = self.blockers;
        self.player_box
            .iter()
            .chain(&self.guest_boxes)
            .chain(&self.cart_boxes)
            .map(boxed)
            .chain(b.mobs.iter().filter(|m| !m.removed).map(Mob::bounds))
            .chain(
                b.armor_stands
                    .iter()
                    .filter(|s| !s.marker)
                    .map(|s| s.bounds()),
            )
            .chain(b.boats.iter().map(boats::Boat::bounds))
            .chain(b.crystals.iter().map(|c| {
                // EntityType.END_CRYSTAL: 2 x 2.
                let p = c.position;
                Aabb::new(
                    Vec3::new(p.x - 1.0, p.y, p.z - 1.0),
                    Vec3::new(p.x + 1.0, p.y + 2.0, p.z + 1.0),
                )
            }))
            .chain(b.tnts.iter().map(tnt::PrimedTnt::bounds))
            .chain(
                b.falling
                    .iter()
                    .map(|f| horizon_client::falling_block::bounds(f.position)),
            )
            .collect()
    }
}
impl LevelAccess for GameLevel<'_> {
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.world.world_surface(x, z, |s| self.facts.is_air(s))
    }
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
        if let Some(signal) = command_blocks::analog_signal(self.world, (pos.x, pos.y, pos.z)) {
            return signal;
        }
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
    /// `Level.isUnobstructed`: the placed state's collision shape against the player.
    /// `Level.isUnobstructed(state, pos, CollisionContext.of(player))`: no
    /// entity that blocks building is in the state's collision shape.
    fn is_unobstructed(&self, pos: BlockPos, state: BlockStateId) -> bool {
        let player = self.player_collision.as_ref();
        let context = player.map_or_else(
            || CollisionContext::empty(false),
            |player| {
                CollisionContext::for_entity(
                    CollisionEntity {
                        identity: EntityIdentity(0),
                        y: player.y,
                        descending: player.descending,
                        main_hand: Some(CollisionItem {
                            item: &player.main_hand_item,
                            count: player.main_hand_count,
                        }),
                    },
                    false,
                )
            },
        );
        let mut scene = collisions::SceneCollisions {
            world: &*self.world,
            binding: self.collision,
            subject: player.map(|player| horizon_client::collisions::PowderSnowSubject {
                fall_distance: player.fall_distance,
                falling_block: false,
                leather_boots: player.leather_boots,
                walkable_type: false,
                lava_walker: false,
            }),
        };
        let Ok(shape) = self
            .collision
            .collision_shape_with_context(&mut scene, state, pos, &context)
        else {
            return false;
        };
        if shape.is_empty() {
            return true;
        }
        let shape = shape.moved(horizon_core::geometry::Vec3::new(
            f64::from(pos.x),
            f64::from(pos.y),
            f64::from(pos.z),
        ));
        horizon_world::entity_obstruction::is_unobstructed(
            &mut BuildingObstruction(self.building_blocker_boxes()),
            None,
            &shape,
        )
        .unwrap_or(false)
    }
    /// `ServerLevel.isPositionTickingWithEntitiesLoaded`: block ticking, and
    /// a ticking chunk.
    fn is_ticking_chunk(&self, chunk: horizon_core::position::ChunkPos) -> bool {
        self.tickets.block_ticking(chunk.x, chunk.z) && ticking_chunk(self.world, chunk.x, chunk.z)
    }
    /// Entities that trigger pressure plates, tripwires and detector rails:
    /// the players (the host and the LAN guests, mobs) and the minecarts.
    fn count_entities(&self, area: Aabb, query: EntityQuery) -> usize {
        let hit = |(lo, hi): &([f64; 3], [f64; 3])| {
            area.intersects(Aabb::new(
                Vec3::new(lo[0], lo[1], lo[2]),
                Vec3::new(hi[0], hi[1], hi[2]),
            ))
        };
        let carts = || self.cart_boxes.iter().filter(|b| hit(b)).count();
        let players = || {
            self.player_box
                .iter()
                .chain(&self.guest_boxes)
                .filter(|b| hit(b))
                .count()
        };
        match query {
            EntityQuery::Minecart => carts(),
            EntityQuery::Mobs => players(),
            EntityQuery::Everything => players() + carts(),
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
            // Picking a sweet berry bush or cave vines: the harvest loot,
            // popped before the pick sound draws its pitch.
            LevelEvent::Player {
                pos,
                state,
                effect: PlayerEffect::InteractLoot(table),
            } => self.drop_interact_loot(pos, state, table),
            event => self.events.push(event),
        }
    }
    fn remove_block_entity(&mut self, pos: BlockPos, old: BlockStateId, side_effects: bool) -> u8 {
        self.remove_entity(pos, old, side_effects)
    }
    fn load_block_entity(&mut self, pos: BlockPos, mut nbt: horizon_world::nbt::Compound) {
        use horizon_world::nbt::Tag;
        for (k, v) in [("x", pos.x), ("y", pos.y), ("z", pos.z)] {
            nbt.insert(k.into(), Tag::Int(v));
        }
        if let Some((key, entity)) = horizon_client::save::decode_block_entity(&Tag::Compound(nbt))
        {
            self.world.block_entities.insert(key, entity);
        }
    }
    fn set_command_block_powered(&mut self, pos: BlockPos, powered: bool) -> Option<(bool, bool)> {
        command_blocks::set_powered(self.world, self.registry, (pos.x, pos.y, pos.z), powered)
    }
}
/// FallingBlockEntity carrying a block state.
#[derive(Clone, Debug)]
struct FallingBlock {
    /// The entity id a server tracks it by (0 until it is tracked).
    id: u32,
    state: BlockStateId,
    /// `DATA_START_POS`: spawn position used to select weighted block models.
    start_pos: BlockPos,
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
    /// `Entity.stuckSpeedMultiplier`: a cobweb it is in scales its next move.
    stuck: Vec3,
    damage: horizon_client::falling_block::Damage,
    /// The UUID and compound it was loaded with, kept while it moves.
    saved: Option<Box<([i32; 4], horizon_world::nbt::Tag)>>,
}
/// The data loaded once at startup and never changed: blocks, items,
/// recipes, biomes, mobs, collision and motion. Read-only and `Send + Sync`,
/// so the client and an integrated server thread can share one copy.
pub struct Resources {
    core: WorldCore,
    rest: std::sync::OnceLock<WorldRest>,
}

struct WorldCore {
    facts: BlockFacts,
    biomes: Arc<Biomes>,
    environment: Environment,
    collision: BlockCollisionRegistry<'static>,
    motion: MotionResources,
    threads: usize,
}

struct WorldRest {
    items: Items,
    recipes: Recipes,
    transformers: horizon_client::block_transform::Transformers,
    mob_types: HashMap<String, Arc<MobType>>,
    mob_data: horizon_client::mobs::MobData,
    providers: Providers,
    block_entity_render: horizon_client::block_entity_render::BlockEntityRenderer,
    animate: horizon_client::animate_tick::AnimateTables,
    fog: horizon_client::fog::FogTables,
    music: horizon_client::music::MusicTables,
    ambient: horizon_client::ambience::AmbientTables,
    nether_environment: Environment,
    end_environment: Environment,
    behaviours: Behaviours,
    player_arm: Option<horizon_client::mobs::Part>,
    player_type: Option<Arc<horizon_client::mobs::MobType>>,
    /// `SpinAttackEffectModel` (Riptide's spinning boxes).
    spin_attack: Option<horizon_client::mobs::Part>,
    stations: horizon_client::stations::StationData,
}

impl Resources {
    fn rest(&self) -> &WorldRest {
        self.rest
            .get()
            .expect("world resources joined before entry")
    }
    fn facts(&self) -> &BlockFacts {
        &self.core.facts
    }
    fn items(&self) -> &Items {
        &self.rest().items
    }
    fn recipes(&self) -> &Recipes {
        &self.rest().recipes
    }
    fn transformers(&self) -> &horizon_client::block_transform::Transformers {
        &self.rest().transformers
    }
    fn biomes(&self) -> &Arc<Biomes> {
        &self.core.biomes
    }
    fn environment(&self) -> &Environment {
        &self.core.environment
    }
    fn mob_types(&self) -> &HashMap<String, Arc<MobType>> {
        &self.rest().mob_types
    }
    fn mob_data(&self) -> &horizon_client::mobs::MobData {
        &self.rest().mob_data
    }
    fn providers(&self) -> &Providers {
        &self.rest().providers
    }
    fn block_entity_render(&self) -> &horizon_client::block_entity_render::BlockEntityRenderer {
        &self.rest().block_entity_render
    }
    fn animate(&self) -> &horizon_client::animate_tick::AnimateTables {
        &self.rest().animate
    }
    fn fog(&self) -> &horizon_client::fog::FogTables {
        &self.rest().fog
    }
    fn music(&self) -> &horizon_client::music::MusicTables {
        &self.rest().music
    }
    fn ambient(&self) -> &horizon_client::ambience::AmbientTables {
        &self.rest().ambient
    }
    fn nether_environment(&self) -> &Environment {
        &self.rest().nether_environment
    }
    fn end_environment(&self) -> &Environment {
        &self.rest().end_environment
    }
    fn behaviours(&self) -> &Behaviours {
        &self.rest().behaviours
    }
    fn collision(&self) -> &BlockCollisionRegistry<'static> {
        &self.core.collision
    }
    fn motion(&self) -> &MotionResources {
        &self.core.motion
    }
    fn threads(&self) -> usize {
        self.core.threads
    }
    fn player_arm(&self) -> &Option<horizon_client::mobs::Part> {
        &self.rest().player_arm
    }
    fn player_type(&self) -> &Option<Arc<horizon_client::mobs::MobType>> {
        &self.rest().player_type
    }
    fn spin_attack(&self) -> &Option<horizon_client::mobs::Part> {
        &self.rest().spin_attack
    }
    fn stations(&self) -> &horizon_client::stations::StationData {
        &self.rest().stations
    }
}

/// Everything the title and menus draw: the GUI atlas with its font and the
/// title panorama. Kept apart from the world bundle so the title can appear
/// before the block sprites, models and gameplay tables are ready.
pub struct UiResources {
    /// The GUI, font, particle, map and skin atlas.
    pub atlas: HudAtlas,
    /// The title screen's panorama faces.
    pub panorama: Arc<horizon_client::gfx::panorama::Faces>,
}
impl UiResources {
    /// From a resource root layering packs over `defaults`, the game's own
    /// resources (`root` itself when no pack is on).
    pub fn load(root: &std::path::Path, defaults: &std::path::Path) -> Result<Self> {
        Ok(Self {
            atlas: HudAtlas::build_over(root, defaults)?,
            panorama: Arc::new(horizon_client::gfx::panorama::Faces::load(root)),
        })
    }
}
/// What a resource pack reload replaces (`Minecraft.reloadResourcePacks`):
/// block models and sprites with their meshing tables, item models,
/// translations and the textures read on the CPU. Each game holds its own;
/// cloning shares the loaded data.
#[derive(Clone)]
pub struct PackResources {
    assets: Arc<Assets>,
    render: Arc<RenderAssets>,
    context: Arc<WorkerContext>,
    layer_names: Arc<HashMap<u32, String>>,
    /// Item definitions, baked item models and special renderers.
    item_renderer: Arc<horizon_client::item_render::ItemRenderer>,
    names: Arc<Names>,
    /// clouds.png: (width, height, filled cells).
    clouds: Arc<(u32, u32, Vec<bool>)>,
    /// misc/forcefield.png: (width, height, RGBA) for the world border wall.
    forcefield: Arc<(u32, u32, Vec<u8>)>,
}
impl PackResources {
    /// Everything a pack overlay at `root` provides, over `assets` and `render`
    /// loaded from it.
    fn load(
        root: &std::path::Path,
        assets: Arc<Assets>,
        render: Arc<RenderAssets>,
        facts: &BlockFacts,
        biomes: Arc<Biomes>,
        light: Arc<LightTables>,
    ) -> Result<Self> {
        let visibility = models::load_visibility(root, assets.registry)?;
        let tables = Arc::new(MeshTables::new(&assets, &render, facts, visibility)?);
        let item_renderer = horizon_client::item_render::ItemRenderer::load_with_library(
            root,
            render.model_library.clone(),
        )?;
        Ok(Self {
            layer_names: Arc::new(
                render
                    .sprite_layers
                    .iter()
                    .map(|(name, layer)| (*layer, name.trim_start_matches("minecraft:").to_owned()))
                    .collect(),
            ),
            context: Arc::new(WorkerContext {
                assets: assets.clone(),
                render: render.clone(),
                tables,
                biomes,
                light,
            }),
            assets,
            render,
            item_renderer: Arc::new(item_renderer),
            names: Arc::new(Names::load(root)),
            clouds: Arc::new(load_clouds(root)),
            forcefield: Arc::new(load_forcefield(root)),
        })
    }
}
/// A quick craft drag as the screen draws it: each dragged slot's count and
/// whether the slot caps it (None: hidden), and what stays in hand.
struct DragView {
    slots: HashMap<usize, Option<(u32, bool)>>,
    left: Option<u32>,
}
/// Ctrl+Q reads the physical keyboard, independently of sprint key mappings.
/// A chunk whose ticking future can complete (`ChunkMap.prepareTickingChunk`):
/// lit, with its neighbours loaded, as `postprocess_chunks` waits for.
fn ticking_chunk(world: &World, x: i32, z: i32) -> bool {
    world.chunks.get(&(x, z)).is_some_and(|c| c.lit)
        && (-1..=1).all(|dz| (-1..=1).all(|dx| world.chunks.contains_key(&(x + dx, z + dz))))
}
fn drop_whole_stack(physical_keys: &HashSet<KeyCode>) -> bool {
    physical_keys.contains(&KeyCode::ControlLeft) || physical_keys.contains(&KeyCode::ControlRight)
}

fn select_creative_tab(
    creative: &mut Creative,
    clicks: &mut horizon_client::container_screen::Clicks,
    tab: usize,
) {
    creative.select(tab);
    *clicks = Default::default();
    // Tabs are outside the panel; their release must not drop a carried stack.
    clicks.skip_release();
}

struct Game<'a> {
    /// Shared with an integrated server; `collision` and `motion` borrow from it.
    res: Arc<Resources>,
    rest_loader: Option<WorldRestLoader>,
    pending_entry: Option<PendingWorldEntry>,
    world_loader: Option<world_loading::WorldLoader>,
    pending_pack_reload: bool,
    /// A server's resource packs (game/server_packs.rs).
    server_packs: server_packs::ServerPackState,
    world_renderer_ready: bool,
    collision: &'a BlockCollisionRegistry<'a>,
    motion: &'a MotionResources,
    /// This game's resource packs (`reload_packs` replaces them).
    pack: PackResources,
    /// The title and menu bundle (GUI atlas, font and panorama).
    gui: UiResources,
    /// `FogRenderer`'s evolving state.
    fog: horizon_client::fog::FogState,
    screen_fx: horizon_client::screen_effects::ScreenEffects,
    weather_parameters: horizon_client::weather::ParameterCache,
    panorama_rotation: horizon_client::gfx::panorama::Rotation,
    world: World,
    streaming: Streaming,
    lod: Option<horizon_client::lod::Lod>,
    lod_source_attempted: bool,
    lod_observe: horizon_client::fast_hash::FxHashSet<(i32, i32)>,
    /// The detail revision, centre and radius `Lod::update` last had covered for.
    lod_covered_at: Option<(u64, (i32, i32), i32)>,
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
    /// The crafting table the 3x3 grid belongs to (`CraftingMenu.access`).
    craft_table: Option<[i32; 3]>,
    /// A quick craft drag over inventory slots: (button, game slots so far).
    inv_drag: Option<(i32, Vec<usize>)>,
    /// `AbstractContainerScreen`'s quick craft and skip-release state.
    inv_clicks: horizon_client::container_screen::Clicks,
    /// The Mouse Tweaks extra's drag and wheel (`tweaks`).
    tweaks: tweaks::Tweaks,
    /// The last inventory click, for the double click that gathers a stack.
    last_inv_click: Option<(std::time::Instant, usize, bool)>,
    /// `lastQuickMoved`: what the last Shift click moved, which a Shift
    /// double click moves every stack of.
    last_quick_moved: Option<ItemStack>,
    /// Open container block entity, if any.
    container: Option<(i32, i32, i32)>,
    /// Open workstation menu (enchanting table, anvil, ...).
    station: Option<stations::Station>,
    /// Status effects and thrown potions (game/potions.rs).
    potions: potions::PotionState,
    scratch_slot: Option<ItemStack>,
    mining: Option<Mining>,
    /// The other players' cracks (`LevelRenderer.destroyingBlocks`): breaker id to the block,
    /// the stage (0..=9) and the game time it last changed.
    destroying: HashMap<i32, ([i32; 3], u8, i64)>,
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
    /// CPU geometry storage retained across frames, including empty/title frames.
    entity_vertices: Vec<Vertex>,
    /// Horizon Extras' dynamic lights this frame (`dynamic_lights.rs`), and how brightly each
    /// item glows (built the first time the extra is on).
    dynamic_lights: Vec<[f32; 4]>,
    luminance: Option<horizon_client::dynamic_light::Luminance>,
    /// Name tags drawn over everything (`RenderPipelines.TEXT_SEE_THROUGH`).
    see_through_vertices: Vec<Vertex>,
    started: Instant,
    previous: Instant,
    /// FramerateLimitTracker.latestInputTime.
    last_input: Instant,
    /// Controllers (Horizon's own).
    pad: pad::Pads,
    frames: u64,
    /// Frames drawn with the player in the world, for `--world-frames`.
    world_frames: u64,
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
    /// Minecarts whose `MinecartSoundInstance` is playing.
    cart_sounds: HashSet<u32>,
    /// The minecart the riding sounds were started for (`LocalPlayer.startRiding`).
    riding_cart_sounds: Option<u32>,
    dropped: Vec<ItemEntity>,
    next_entity: u32,
    particles: horizon_client::particles::ParticleEngine,
    /// `ItemPickupParticle`s: taken items, orbs and arrows flying to their collector.
    pickups: pickups::Pickups,
    /// Sign text being edited (`SignEditScreen`).
    sign_edit: Option<sign_edit::SignEditor>,
    /// An open book, book and quill or lectern screen.
    book: Option<books::BookScreen>,
    /// Lecterns whose page-turn pulse is running (ticks left).
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    /// LevelLoadingScreen.smoothedProgress while the player is not placed yet.
    loading_progress: f32,
    /// The world spawn search's origin chunk and fallback height, once known.
    spawn_origin: Option<((i32, i32), i32)>,
    spawn_spiral: horizon_client::world_spawn::ChunkSpiral,
    /// A new player's spawn search, while its candidates' chunks load.
    player_spawn: Option<horizon_client::world_spawn::PlayerSpawnFinder>,
    mobs: Vec<Mob>,
    creatures: creatures::Creatures,
    sculk: sculk::SculkState,
    raiding: raids::Raiding,
    attack_ticks: u32,
    /// The main hand's item at the last player tick, whose attack damage and
    /// speed modifiers apply (`LivingEntity.detectEquipmentUpdates`): a hotbar
    /// swap just before an attack keeps them, with the new item's enchantments
    /// (attribute swapping).
    attack_item: Option<Arc<str>>,
    /// Mobs a player hurt (`LivingEntity.lastHurtByPlayer`), with the guest
    /// it was by connection (None: the host).
    last_hurt_by_player: HashMap<u32, Option<u32>>,
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
    /// The hand an item use acts with (`InteractionHand`): the off hand only
    /// for its turn in `use_item` and while its item is being used.
    hand: Hand,
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
    dragging: bool,
    /// `ClientAvatarState`: walked distance and view bob.
    avatar: horizon_client::camera::AvatarBob,
    flash: u32,
    /// Bolts that struck here and sky flashes they made, in total (`status`).
    lightning_seen: [u32; 2],
    /// The head of the bed while sleeping; the visual timer also survives waking.
    sleeping: Option<[i32; 3]>,
    sleep_timer: u32,
    dim: Dim,
    world_root: PathBuf,
    /// A player carried through a portal, re-placed once the destination loads.
    travelling: Option<(PlayerMotion, Vec3)>,
    /// A server's players on their way to another level, each waiting for
    /// the destination to load around them.
    server_trips: Vec<server_travel::Trip>,
    /// Players a command sends to another level, once it has run.
    level_teleports: Vec<server_travel::LevelTeleport>,
    /// A server's levels other than the one in place (game/levels.rs).
    levels: levels::Levels,
    /// Which pass of a server tick runs (`tick_server`): the first one reads
    /// the network, the last one ends the players' tick.
    level_pass: levels::Pass,
    /// Nether portals: the player's, items' and mobs' (game/nether_portal.rs).
    portals: nether_portal::Portals,
    /// The pending travel goes through an End portal (EndPortalBlock destinations).
    end_travel: bool,
    /// The respawn point when it is not the world spawn or a bed: a respawn
    /// anchor (`game/respawn_anchor.rs`).
    respawn_config: Option<respawn_anchor::RespawnConfig>,
    /// A respawn travelling to its anchor's dimension and chunks.
    respawn_travel: Option<respawn_anchor::RespawnConfig>,
    /// The next enter_dimension call comes from an End portal.
    end_portal_travel: bool,
    /// The open WinScreen (game/win.rs).
    win: Option<win::Win>,
    end: end::EndState,
    block_ticks: Ticks,
    tick_chunks: horizon_client::tick_chunk_membership::TickChunkMembership,
    tickets: horizon_client::ticket_levels::ChunkTickets,
    random_tick_sections: horizon_client::random_tick_sections::RandomTickSections,
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
    /// `WaypointStyleManager`.
    waypoint_styles: horizon_client::locator_bar::WaypointStyles,
    /// `ClientWaypointManager`: the locator bar's waypoints.
    waypoints: horizon_client::locator_bar::ClientWaypoints,
    /// The camera the last frame drew from (position, yaw, pitch, fov), for the locator bar.
    locator_camera: Option<([f64; 3], f32, f32, f32)>,
    /// `LocalPlayer.experienceDisplayStartTick`: (last progress, seen a change, tick).
    experience_display: (f32, bool, u64),
    boats: Vec<boats::Boat>,
    boat_models: boats::BoatModels,
    /// EndCrystalModel's baked layer.
    crystal_model: Option<horizon_client::mobs::Part>,
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
    text_displays: std::collections::HashMap<i32, text_displays::RemoteTextDisplay>,
    text_display_vertices: usize,
    /// Player skin downloads, and which skin each atlas skin slot holds
    /// (least recently shown first).
    skin_downloads: RefCell<horizon_client::skins::Downloader>,
    skin_slots: Vec<(String, usize)>,
    /// Atlas trim slots in use: (sprite name, slot), least recently drawn first.
    trim_slots: Vec<(String, usize)>,
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
    /// The other players' hooks a server tracks to this client.
    remote_hooks: Vec<fishing::RemoteHook>,
    tridents: Vec<trident::ThrownTrident>,
    rockets: Vec<fireworks::Rocket>,
    /// Ominous trial spawners' item spawners.
    ominous_spawners: Vec<ominous_item_spawner::OminousItemSpawner>,
    creaking: creaking::CreakingState,
    /// Monster spawners' display mobs (`game/spawner.rs`).
    spawners: spawner::SpawnerState,
    trident_model: Option<horizon_client::mobs::Part>,
    arrow_model: Option<horizon_client::mobs::Part>,
    tnts: Vec<tnt::PrimedTnt>,
    hoppers: HashMap<(i32, i32, i32), machines::HopperState>,
    /// Where item entities are, while the block entities tick (hoppers).
    item_columns: Option<machines::ItemColumns>,
    creative_screen: Creative,
    control: Option<horizon_client::control::Server>,
    /// The Agent Control setting the socket was last opened or closed for (`agent_settings`).
    agent_control_on: bool,
    /// When an agent's last command came.
    agent_last: Option<Instant>,
    frame_stats: horizon_client::control::FrameStats,
    /// This play session's frame and tick times, for opt-in performance statistics.
    perf: horizon_client::telemetry::Session,
    /// Control replies waiting for a rendered frame or a benchmark deadline.
    pending_screenshot: Option<std::sync::mpsc::Sender<String>>,
    pending_bench: Option<(Instant, std::sync::mpsc::Sender<String>)>,
    /// Recent frame times for the performance overlay.
    frame_graph: horizon_client::perf_overlay::FrameGraph,
    /// Horizon's extras (zoom, FPS counter, keystrokes): `game/extras.rs`.
    extras: extras::ExtrasState,
    /// Horizon mods (`game/mods.rs`), none running unless turned on.
    mods: mods::ModsState,
    /// Meshed sections waiting for an upload budget (Horizon settings).
    pending_uploads: Vec<((i32, i32, i32), horizon_client::mesher::SectionMesh)>,
    /// This frame's view frustum, for culling entities and block entities.
    view_frustum: Option<horizon_client::gfx::Frustum>,
    entity_culler: Option<horizon_client::entity_culling::Culler>,
    /// A scripted flight from the control socket (`fly`).
    flight: Option<control::Flight>,
    /// The running agent action from the control socket (game/agent.rs).
    agent: agent::Agent,
    /// Widget commands waiting for the frame after a click (game/ui_control.rs).
    ui_wait: ui_control::UiWait,
    /// What happened, for `events` and `wait-for` (game/events.rs).
    events: events::Events,
    /// The overworld's last `sky_darken` and its inputs (time, rain, thunder).
    sky_darken_memo: std::cell::Cell<Option<((i64, u32, u32), i32)>>,
    persist: persist::EntityMemory,
    /// Connected to a server: the world comes from the network (game/remote.rs).
    remote: Option<remote::Remote>,
    /// This world is open to the network (game/lan.rs).
    lan: Option<lan::LanHost>,
    /// Single player's server thread, which `remote` is connected to.
    integrated: Option<integrated::Integrated>,
    /// This game is that server thread's (game/integrated.rs).
    serving: Option<integrated::Serving>,
    /// The open world is shared behind an invite code (game/friends.rs).
    friends: Option<friends::Sharing>,
    world_options: Option<lan_screen::WorldOptions>,
    /// Chat Settings opened from RestrictionsScreen: Done returns to World Options.
    options_to_world_options: bool,
    command_block_screen: Option<command_blocks::CommandBlockScreen>,
    /// KeyboardHandler's debug key state (game/debug_keys.rs), the physical
    /// keys held, and the debug renderers F3+B and F3+G switch on.
    debug_keys: horizon_client::debug_keys::DebugKeys,
    /// The F3 overlay's charts (F3+2, F3+3).
    debug_charts: horizon_client::debug_chart::DebugCharts,
    show_hitboxes: bool,
    show_chunk_borders: bool,
    /// PauseScreen.showPauseMenu (false after F3+Escape).
    pause_menu: bool,
    /// MultiPlayerGameMode.previousLocalPlayerMode (F3+N goes back to it).
    previous_mode: Option<horizon_client::command::args::GameMode>,
    gamemode_switcher: Option<horizon_client::game_mode_switcher::GameModeSwitcher>,
    /// The system clipboard (`KeyboardHandler.clipboardManager`).
    clipboard: horizon_client::clipboard::SystemClipboard,
    /// Select All in the append-only text fields.
    text_selection: horizon_client::clipboard::AppendSelection,
    /// The world was saved for the application ending (`save_for_exit`).
    saved_for_exit: bool,
    pending_save: Option<saving::ClosingSave>,
    /// The block a guest is acting on (game/lan_actions.rs).
    forced_hit: Option<Hit>,
    /// The part of a right click a guest's packet asks for (game/lan_actions.rs).
    guest_use: Option<GuestUse>,
    /// Level events of this tick for the guests, and the guest acting now.
    lan_events: Vec<(i32, [i32; 3], i32, Option<u32>)>,
    /// Block events (position, b0, b1, state) and entity events (id, event,
    /// position) for the guests.
    lan_block_events: Vec<([i32; 3], i32, i32, u32)>,
    lan_entity_events: Vec<(i32, i8, [f64; 3])>,
    /// `ContainerOpenersCounter`: how many players (the host and the guests)
    /// have each chest open.
    lid_openers: HashMap<(i32, i32, i32), i32>,
    lan_acting: Option<u32>,
    /// The name of the guest `lan_acting` acts for (`Player.getName`).
    lan_acting_name: Option<String>,
    /// Their UUID (`Player.getUUID`).
    lan_acting_uuid: Option<u128>,
    /// Command feedback to forward to a guest.
    chat_capture: Option<Vec<horizon_client::text::Text>>,
    /// Packets for the guest acting now, sent once its action returns.
    guest_packets: Vec<horizon_protocol::packets::play::Clientbound>,
    /// `broadcastToAdmins` lines a guest's command queued, sent once it
    /// returns: the line, whether operators get it and whether the console
    /// logs it.
    admin_feedback: Vec<(horizon_client::text::Text, bool, bool)>,
    /// The server console is running a command: whether its feedback goes to
    /// the operators (`DedicatedServer.shouldInformAdmins`,
    /// `broadcast-console-to-ops`).
    console_broadcast: Option<bool>,
    /// State of `/spectate`, `/posteffect`, `/debug`, `/waypoint` and friends.
    cmds: commands::extra::CommandExtras,
    ui: chat::UiState,
    combat: combat::CombatTracker,
    mount: mounts::MountState,
    stats: stats::Stats,
    score: scoreboard::ScoreState,
    fns: functions::FnState,
    bosses: bossbars::BossState,
    /// `Hud.bossOverlay`: the bars this window is showing.
    boss_overlay: horizon_client::boss_overlay::BossOverlay,
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
    restore_player_state: bool,
    stats_ui: stats_screen::StatsUi,
    adv_ui: advancements_screen::AdvUi,
    camera_type: perspective::CameraType,
    fangs: fangs::FangState,
    /// The local player as a model (third person, the inventory).
    player_model: Option<Mob>,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// `Level.getMinY`: the dimension type's minimum y.
    fn min_y(self) -> i32 {
        match self {
            Self::Overworld => MIN_Y,
            Self::Nether | Self::End => 0,
        }
    }
    /// `DimensionType.logicalHeight`.
    fn logical_height(self) -> i32 {
        match self {
            Self::Overworld => HEIGHT,
            Self::Nether => 128,
            Self::End => 256,
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
/// How far along `dir` (within `reach`) a ray from `eye` enters `b`.
fn ray_box(eye: [f64; 3], dir: [f64; 3], reach: f64, b: Aabb) -> Option<f64> {
    let (min, max) = (b.min_position(), b.max_position());
    let (lo, hi) = ([min.x, min.y, min.z], [max.x, max.y, max.z]);
    let (mut t0, mut t1) = (0.0f64, reach);
    for a in 0..3 {
        if dir[a].abs() < 1e-9 {
            if eye[a] < lo[a] || eye[a] > hi[a] {
                return None;
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
    (t0 <= t1).then_some(t0)
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
        let f = self.res.facts().get(state);
        let o = if f.outline_offset {
            self.pack
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
            let f = self.res.facts().get(state);
            if !self.res.facts().is_air(state)
                && (f.fluid == Fluid::None
                    || f.waterlogged
                    || &*f.name == "minecraft:kelp"
                    || f.render_shape == horizon_client::blocks::RenderShape::Model)
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
    /// The stack in the hand that acts (`getItemInHand(hand)`).
    fn held(&self) -> Option<&ItemStack> {
        self.stack_in(self.held_slot())
    }
    fn held_mut(&mut self) -> &mut Option<ItemStack> {
        self.stack_in_mut(self.held_slot())
    }
    /// The slot of the hand that acts: the selected one, or `OFFHAND_SLOT`.
    /// `LivingEntity.canGlideUsing`: a glider in the chest slot that will
    /// not break with its next point of wear.
    fn glider_worn(&self) -> bool {
        self.armor[1].as_ref().is_some_and(|s| {
            self.res
                .items()
                .get(&s.item)
                .is_some_and(|i| i.glider && (i.max_damage == 0 || s.damage + 1 < i.max_damage))
        })
    }
    /// What the window decides for its player's body every tick, in frame and
    /// on a server alike: the look (Player.travel steers sprint-swimming by
    /// the pitch), Auto Jump (LocalPlayer.sendPosition), the slowdown of an
    /// item in use (LocalPlayer.modifyInput), the helmet's Respiration
    /// (decreaseAirSupply), the wings, leather boots on powder snow
    /// (PowderSnowBlock.canEntityWalkOnPowderSnow), freeze-immune wear
    /// (LivingEntity.canFreeze) and the dimension's FAST_LAVA.
    pub(crate) fn sync_player_body(&mut self) {
        let glider = self.glider_worn();
        let item_use_speed = self
            .using
            .and_then(|(slot, _)| {
                if slot == shield::OFFHAND_SLOT {
                    self.offhand.as_ref()
                } else {
                    self.inventory.get(slot)?.as_ref()
                }
            })
            .and_then(|s| self.res.items().get(&s.item))
            .map(|i| i.use_speed);
        let respiration = self.armor[0]
            .as_ref()
            .map_or(0, |s| s.enchantment_level("minecraft:respiration") as i32);
        let leather_boots = self.armor[3]
            .as_ref()
            .is_some_and(|s| &*s.item == "minecraft:leather_boots");
        let immune = self
            .res
            .items()
            .item_tags
            .get("minecraft:freeze_immune_wearables");
        let freeze_immune = self
            .armor
            .iter()
            .flatten()
            .any(|s| immune.is_some_and(|t| t.contains(&*s.item)));
        let Some(player) = self.player.as_mut() else {
            return;
        };
        player.yaw = self.yaw;
        player.pitch = self.pitch;
        player.auto_jump_enabled = self.settings.auto_jump;
        player.item_use_speed = item_use_speed;
        player.respiration = respiration;
        player.glider = glider;
        player.leather_boots = leather_boots;
        player.freeze_immune = freeze_immune;
        player.fluids.fast_lava = self.dim == Dim::Nether;
    }
    fn held_slot(&self) -> usize {
        match self.hand {
            Hand::Main => self.selected,
            Hand::Off => shield::OFFHAND_SLOT,
        }
    }
    /// `LivingEntity.getUsedItemHand`: the hand of the item in use.
    fn used_hand(&self) -> Option<Hand> {
        self.using.map(|(slot, _)| {
            if slot == shield::OFFHAND_SLOT {
                Hand::Off
            } else {
                Hand::Main
            }
        })
    }
    /// Run `f` with `hand` acting (`held`, `consume_held`, `swing_arm`).
    fn with_hand<T>(&mut self, hand: Hand, f: impl FnOnce(&mut Self) -> T) -> T {
        let prior = std::mem::replace(&mut self.hand, hand);
        let out = f(self);
        self.hand = prior;
        out
    }
    /// Inventory.add: returns the stack that did not fit.
    fn give(&mut self, mut stack: ItemStack) -> Option<ItemStack> {
        let max = self
            .res
            .items()
            .get(&stack.item)
            .map_or(64, |i| i.max_stack);
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
    /// The block's first up face (or any face) and its fixed tint, for block
    /// models drawn as entities (falling blocks, carried blocks).
    fn block_face_sprite(&self, state: BlockStateId) -> Option<(String, u32)> {
        let pack = &self.pack;
        let models = pack
            .render
            .model_catalog
            .collect(state, horizon_core::position::BlockPos::new(0, 0, 0))
            .ok()?;
        let face = models
            .iter()
            .flat_map(|(_, m)| m.faces.iter())
            .find(|f| f.quad.facing == horizon_assets::model::Direction::Up)
            .or_else(|| models.iter().flat_map(|(_, m)| m.faces.iter()).next())?;
        let name = pack.layer_names.get(&face.layer)?.clone();
        let tint = if face.quad.tint_index >= 0 {
            let n = &self.res.facts().get(state).name;
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
    /// `TerrainParticle` at `pos`: the model's particle sprite and tint source
    /// 0's `colorAsTerrainParticle` (white when the block has none).
    fn particle_sprite(&self, state: BlockStateId, pos: [i32; 3]) -> Option<(String, u32)> {
        let sprite = Self::block_particle_sprite(&self.pack, state)?;
        let chunks = &self.world.chunks;
        let (cx, cz) = (pos[0] >> 4, pos[2] >> 4);
        let region: [[Option<&horizon_client::chunk::Chunk>; 3]; 3] = std::array::from_fn(|dz| {
            std::array::from_fn(|dx| {
                chunks
                    .get(&(cx + dx as i32 - 1, cz + dz as i32 - 1))
                    .map(|c| &**c)
            })
        });
        let tint = self
            .pack
            .context
            .tables
            .terrain_particle_tint(
                state,
                pos,
                &region,
                self.res.biomes(),
                self.streaming.biome_zoom,
                self.streaming.biome_blend,
            )
            .unwrap_or(0xFFFFFF);
        Some((sprite, tint))
    }
    /// `BlockStateModelSet.getParticleMaterial`: the sprite of the model's
    /// `particle` texture, else its first face (entity-rendered blocks may
    /// have a particle texture without faces, and faces without one).
    fn block_particle_sprite(pack: &PackResources, state: BlockStateId) -> Option<String> {
        let catalog = &pack.render.model_catalog;
        let layer = catalog.particle(state).or_else(|| {
            let models = catalog
                .collect(state, horizon_core::position::BlockPos::new(0, 0, 0))
                .ok()?;
            models
                .iter()
                .flat_map(|(_, m)| m.faces.iter())
                .next()
                .map(|f| f.layer)
        })?;
        pack.layer_names.get(&layer).cloned()
    }
    fn particle_shape(&self, pos: [i32; 3], state: BlockStateId) -> Vec<[f64; 6]> {
        let f = self.res.facts().get(state);
        let offset = if f.outline_offset {
            self.pack
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
        let Some((sprite, tint)) = self.particle_sprite(state, pos) else {
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
        let Some((sprite, tint)) = self.particle_sprite(state, pos) else {
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
    /// `Player.dropEquipment` for the player who died: unless keep_inventory
    /// is on, `destroyVanishingCursedItems`, then `Inventory.dropAll` from
    /// `at` (the eyes less 0.3): slots 0..36, armor feet to head (36..39),
    /// the off hand (40), each thrown in a random direction.
    fn drop_inventory(&mut self, at: [f64; 3]) {
        if self.ui.rules.bool("keep_inventory") {
            return;
        }
        let mut stacks: Vec<ItemStack> =
            self.inventory.iter_mut().filter_map(Option::take).collect();
        stacks.extend(self.armor.iter_mut().rev().filter_map(Option::take));
        stacks.extend(self.offhand.take());
        stacks.retain(|s| s.enchantment_level("minecraft:vanishing_curse") == 0);
        self.apply_equipment();
        for stack in stacks {
            let r = &mut self.world.random;
            let strength = f64::from(r.next_float()) * 0.5;
            let angle = f64::from(r.next_float()) * std::f64::consts::TAU;
            let v = [-angle.sin() * strength, 0.2, angle.cos() * strength];
            self.spawn_item(stack, at, v, 40);
        }
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
            health: 5,
            fire_ticks: 0,
            thrower: None,
            id: self.next_entity,
            stack,
            position: p,
            previous: p,
            velocity: Vec3::new(velocity[0], velocity[1], velocity[2]),
            on_ground: false,
            age: 0,
            pickup_delay,
            bob_offset: bob,
            ticks: 0,
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
        let registry = self.pack.assets.registry;
        let dynamic = self.pending.dynamic(pos);
        match self.res.items().block_loot_sequence(registry, state) {
            Some(sequence) => {
                let random = self.rand_sequences.get(&sequence, self.level.seed);
                self.res
                    .items()
                    .block_drops_with(registry, state, held, &dynamic, random)
            }
            None => self.res.items().block_drops_with(
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
        let spectator = self.spectator();
        let Some(thrown) = horizon_client::items::take_held_drop(
            &mut self.inventory[self.selected],
            all,
            spectator,
        ) else {
            return;
        };
        self.player_drop(thrown);
    }
    /// `Player.drop`: the statistics and a thrown item entity in front of the player.
    fn player_drop(&mut self, thrown: ItemStack) {
        let Some(eye) = self.eye_position() else {
            return;
        };
        // LivingEntity.drop: every drop swings the main hand, a menu's
        // THROW and a creative tab's drop included.
        self.swing_hand(Hand::Main, SwingAnimation::DEFAULT);
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
        if self.lan_acting.is_none() {
            let uuid = self.level.ensure_player_uuid();
            self.dropped.last_mut().unwrap().thrower = Some(uuid);
        }
    }
    fn tick_items(&mut self) {
        let mut items = std::mem::take(&mut self.dropped);
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        let mut removed = vec![false; items.len()];
        let mut cells = horizon_client::entities::ItemCells::new(&items);
        let catalog = self.res.items();
        let max_stack =
            |stack: &ItemStack| catalog.get(&stack.item).map_or(64, |i| i.max_stack).max(1);
        // A LAN or server guest only shows the host's items.
        let merging = self.remote.is_none();
        for i in 0..items.len() {
            if removed[i] {
                continue;
            }
            let item = &mut items[i];
            if !ticking(
                item.position.x.floor() as i32,
                item.position.z.floor() as i32,
            ) {
                continue;
            }
            // ItemEntity.tick's noPhysics (servers only): out of a block
            // placed on it, towards the closest open side (#1252).
            let no_physics = merging && item.inside_block(&self.world, self.collision);
            if no_physics {
                let speed = self.world.random.next_float() * 0.2 + 0.1;
                item.move_towards_closest_space(&self.world, self.res.facts(), speed);
            }
            let alive = item.tick(&self.world, self.res.facts(), self.collision, no_physics);
            cells.moved(i, item.previous, item.position);
            // ItemEntity.tick: merge every 2 ticks while moving between
            // blocks, every 40 otherwise, before the lifetime check.
            let block = |v: Vec3| (v.x.floor(), v.y.floor(), v.z.floor());
            let rate = if block(item.previous) == block(item.position) {
                40
            } else {
                2
            };
            if merging
                && (alive || item.age == 6000 && item.position.y > -128.0)
                && item.ticks.is_multiple_of(rate)
            {
                horizon_client::entities::merge_with_neighbours(
                    &mut items,
                    &mut removed,
                    &mut cells,
                    i,
                    &max_stack,
                );
            }
            let item = &mut items[i];
            if !alive || removed[i] {
                removed[i] = true;
                cells.remove(i, item.position);
                continue;
            }
            let environment = item.environment_tick(
                &self.world,
                self.res.facts(),
                self.pack.assets.registry,
                self.res.items(),
            );
            // shortcut: age is the fresh-drop clock; use entity ticks for saved/frozen-age phase parity.
            if environment.should_play_lava_hurt_sound(item.age) {
                let pitch = 2.0 + self.world.random.next_float() * 0.4;
                self.audio.play(
                    "minecraft:entity.generic.burn",
                    Some([item.position.x, item.position.y, item.position.z]),
                    0.4,
                    pitch,
                );
            }
            if !environment.alive {
                removed[i] = true;
                cells.remove(i, item.position);
            }
        }
        let mut removed = removed.into_iter();
        items.retain(|_| !removed.next().unwrap_or(false));
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
                        self.add_pickup(
                            pickups::Taken::Item(Box::new(item)),
                            pickups::Collector::Player,
                        );
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
                            self.add_pickup(
                                pickups::Taken::Item(Box::new(item.clone())),
                                pickups::Collector::Player,
                            );
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
        let player_collision = self.player.as_ref().filter(|_| !spectator).map(|p| {
            let main_hand = self.inventory.get(self.selected).and_then(Option::as_ref);
            PlayerCollision {
                y: p.position.y,
                descending: p.velocity.y < 0.0,
                fall_distance: p.fall_distance,
                main_hand_item: main_hand.map_or_else(
                    || Arc::from("minecraft:air"),
                    |stack| Arc::clone(&stack.item),
                ),
                main_hand_count: main_hand.map_or(0, |stack| stack.count as i32),
                leather_boots: self.armor[3]
                    .as_ref()
                    .is_some_and(|stack| stack.item.as_ref() == "minecraft:leather_boots"),
            }
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
        let guest_boxes = self.lan_block_trigger_boxes();
        let dimension = match self.dim {
            Dim::Overworld => LevelDimension::Overworld,
            Dim::Nether => LevelDimension::Nether,
            Dim::End => LevelDimension::End,
        };
        let rules = self.level_rules();
        let blockers = Blockers {
            mobs: &self.mobs,
            armor_stands: &self.armor_stands,
            boats: &self.boats,
            crystals: &self.crystals,
            tnts: &self.tnts,
            falling: &self.falling,
        };
        let mut level = GameLevel {
            cart_boxes,
            blockers,
            world: &mut self.world,
            facts: self.res.facts(),
            collision: self.collision,
            events: &mut self.level_events,
            dimension,
            darken,
            raining: rain > 0.2,
            thundering: thunder > 0.9,
            player_box,
            guest_boxes,
            player_collision,
            biomes: self.res.biomes(),
            seed: self.level.seed,
            behaviours: self.res.behaviours(),
            items: self.res.items(),
            loot: horizon_client::structure_blocks::loot_data(&self.options.resources),
            registry: self.pack.assets.registry,
            rules,
            rand_sequences: &mut self.rand_sequences,
            pending: &mut self.pending,
            spill: &mut self.spill,
            tickets: self.tickets.levels(),
            random_tick_sections: &mut self.random_tick_sections,
        };
        let result = f(self.res.behaviours(), &mut level, &mut self.block_ticks);
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
    fn attack_block(&mut self, pos: [i32; 3]) -> Result<()> {
        self.with_level(|b, level, ticks| {
            b.attack_block(level, ticks, BlockPos::new(pos[0], pos[1], pos[2]));
        });
        self.process_level_events()
    }
    /// Track which chunks are ticking (Ticks needs chunk load/unload).
    fn sync_tick_chunks(&mut self) {
        if !self.tick_chunks.synchronize(&self.world.chunks) {
            return;
        }
        // Move only the added keys out so registration can borrow all of Game.
        let added = self.tick_chunks.take_added();
        for &key in &added {
            self.register_chunk_ticks(key);
        }
        for &key in self.tick_chunks.removed() {
            self.random_tick_sections.remove(&key);
            self.portals.sections.remove(&key);
            self.block_ticks
                .remove_chunk(horizon_core::position::ChunkPos::new(key.0, key.1));
        }
        self.tick_chunks.recycle_added(added);
    }
    /// `DistanceManager`'s tickets of the level in place, from every player in
    /// it (the local player, LAN guests, a server's players) and its forced
    /// chunks. A game with neither a server nor a player (the headless
    /// fixtures) ticks everywhere.
    fn update_chunk_tickets(&mut self) {
        let mut players: Vec<(i32, i32)> = self.lan.as_ref().map_or_else(Vec::new, |h| {
            h.hub
                .guests
                .iter()
                .filter(|g| h.hub.here(g))
                .map(|g| g.chunk())
                .collect()
        });
        if let Some(p) = &self.player {
            players.push((
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            ));
        }
        let sources = (self.options.dedicated.is_some() || !players.is_empty()).then(|| {
            horizon_client::ticket_levels::Sources {
                players,
                forced: self.streaming.forced.iter().copied().collect(),
                view_distance: self.settings.render_distance.clamp(2, 32),
                simulation_distance: self.settings.simulation_distance.max(2),
            }
        });
        self.tickets.update(sources);
    }
    /// `ServerLevel.startTickingChunk`: a loaded chunk's saved ticks are
    /// scheduled from the game time at which it becomes a ticking chunk
    /// (block ticking, with its neighbours loaded). Chunks become ticking
    /// between ticks in vanilla: before this tick's clock advanced.
    fn start_ticking_chunks(&mut self) {
        let levels = self.tickets.levels();
        let world = &self.world;
        let started = self
            .tick_chunks
            .start_ticking(|(x, z)| levels.loaded_ticking(x, z) && ticking_chunk(world, x, z));
        let time = self.world.game_time - 1;
        for (x, z) in started {
            let pos = horizon_core::position::ChunkPos::new(x, z);
            self.block_ticks.blocks.unpack_chunk(pos, time);
            self.block_ticks.fluids.unpack_chunk(pos, time);
        }
    }
    /// ServerLevel.tick block phases: scheduled ticks, random ticks in the
    /// chunks the tickets simulate, block events and moving pistons.
    fn tick_blocks(&mut self) -> Result<()> {
        self.sync_tick_chunks();
        self.start_ticking_chunks();
        let levels = self.tickets.levels();
        // Keep the world map's chunk order, including empty chunks
        // (tick_chunk_random still draws for precipitation in those).
        let ticking: Vec<_> = self
            .world
            .chunks
            .keys()
            .copied()
            .filter(|&(x, z)| levels.random_ticking(x, z) && ticking_chunk(&self.world, x, z))
            .collect();
        // Entity.checkInsideBlocks for the plates, tripwires and detector rails
        // under the players (the host and the LAN guests) and the minecarts.
        let mut inside = Vec::new();
        let guests = self.lan_block_trigger_boxes();
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
            }))
            .chain(guests);
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
            for (x, z) in ticking {
                // The sections are read again only after a block changed.
                let mut read = None;
                let mut sections = Default::default();
                let pos = horizon_core::position::ChunkPos::new(x, z);
                b.tick_chunk_random(level, ticks, pos, |level, from| {
                    if read != Some(level.world.revision) {
                        read = Some(level.world.revision);
                        sections = level
                            .world
                            .chunks
                            .get(&(x, z))
                            .map_or_else(Default::default, |chunk| {
                                level.random_tick_sections.get((x, z), chunk, level.facts)
                            });
                    }
                    sections.from(from)
                });
            }
            b.run_block_events(level, ticks);
            b.tick_block_entities(level, ticks);
        });
        self.sync_moving_pistons();
        self.process_level_events()
    }
    /// Copies the moving-piston block entities into the world, where the
    /// collision queries and the renderer read them.
    fn sync_moving_pistons(&mut self) {
        let pistons = &mut self.world.moving_pistons;
        pistons.clear();
        pistons.extend(
            self.block_ticks
                .moving_pistons
                .iter()
                .map(|(p, piston)| ([p.x, p.y, p.z], piston.shape_state())),
        );
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
                    if self.res.facts().is_air(state) {
                        continue;
                    }
                    let sounds = self.res.facts().get(state).sounds.clone();
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
                LevelEvent::LevelEvent { id: 2004, pos, .. } => {
                    self.spawner_burst([pos.x, pos.y, pos.z])
                }
                // LevelEventHandler 1505: the growth particles, then the sound.
                LevelEvent::LevelEvent {
                    id: 1505,
                    pos,
                    data,
                } => {
                    self.growth_particles(pos, data);
                    let at = [pos.x, pos.y, pos.z].map(|v| f64::from(v) + 0.5);
                    self.audio
                        .play("minecraft:item.bone_meal.use", Some(at), 1.0, 1.0);
                }
                LevelEvent::LevelEvent {
                    id: 2015,
                    pos,
                    data,
                } => self.dragon_egg_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2017,
                    pos,
                    data,
                } => self.consume_teleport_particles([pos.x, pos.y, pos.z], data),
                LevelEvent::LevelEvent {
                    id: 2013,
                    pos,
                    data,
                } => self.smash_dust(pos, data),
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
                LevelEvent::LevelEvent {
                    id: id @ 3011..=3021,
                    pos,
                    data,
                } => self.trial_level_event(id, [pos.x, pos.y, pos.z], data),
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
                        1029 => Some(("minecraft:block.anvil.destroy", 1.0)),
                        1031 => Some(("minecraft:block.anvil.land", 0.3)),
                        1042 => Some(("minecraft:block.grindstone.use", 1.0)),
                        1044 => Some(("minecraft:block.smithing_table.use", 1.0)),
                        // LevelEvent.SOUND_PAGE_TURN (a lectern's page).
                        1043 => Some(("minecraft:item.book.page_turn", 1.0)),
                        _ => None,
                    };
                    if let Some((name, volume)) = sound {
                        let pitch = match id {
                            1009 => {
                                2.6 + (self.world.random.next_float()
                                    - self.world.random.next_float())
                                    * 0.8
                            }
                            1029 | 1030 | 1042 | 1043 | 1044 => {
                                self.world.random.next_float() * 0.1 + 0.9
                            }
                            _ => 1.0,
                        };
                        self.audio.play(name, c, volume, pitch);
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
                        id: 0,
                        state,
                        start_pos: pos,
                        position: p,
                        previous: p,
                        velocity: Vec3::ZERO,
                        age: 0,
                        no_gravity: false,
                        drop_item: true,
                        fall_distance: 0.0,
                        stuck: Vec3::ZERO,
                        damage: if self.res.facts().get(state).name.ends_with("anvil") {
                            horizon_client::falling_block::Damage::anvil()
                        } else {
                            Default::default()
                        },
                        saved: None,
                    });
                }
                LevelEvent::Player { pos, effect, .. } => match effect {
                    PlayerEffect::OpenMenu => self.open_menu([pos.x, pos.y, pos.z]),
                    PlayerEffect::Sleep if self.beds_destroy_on_use() => {
                        self.destroy_bed_on_use([pos.x, pos.y, pos.z])?
                    }
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
                    PlayerEffect::RespawnAnchor => {
                        self.respawn_anchor_used([pos.x, pos.y, pos.z])?
                    }
                    PlayerEffect::InteractLoot(_) => {}
                },
                LevelEvent::PrimeTnt { pos, rot } => self.add_primed_tnt(pos, rot, 80),
                LevelEvent::BlockEvent {
                    pos,
                    state,
                    kind: "dispense",
                } => self.dispense_from(pos, state)?,
                LevelEvent::BlockEvent {
                    pos,
                    kind: "command_block",
                    ..
                } => self.command_block_tick(pos),
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
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            .clone();
        let key = (pos[0], pos[1], pos[2]);
        if &*name == "minecraft:lectern" {
            self.open_lectern(pos);
            return;
        }
        if horizon_client::command_block::Mode::of_block(&name).is_some() {
            self.open_command_block(key);
            return;
        }
        if &*name == "minecraft:crafting_table" {
            self.craft_size = 3;
            self.craft_table = Some(pos);
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
            self.world.block_entities.get_or_insert_with(key, || {
                BlockEntity::for_block(&name).unwrap_or(BlockEntity::Chest(Vec::new()))
            });
        } else if storage::is_shulker_box(&name) && !self.shulker_can_open(pos) {
            // ShulkerBoxBlock.canOpen: the lid has no room; nothing opens.
        } else if let Some(entity) = BlockEntity::for_block(&name) {
            self.world.block_entities.get_or_insert_with(key, || entity);
            let centre = [
                f64::from(key.0) + 0.5,
                f64::from(key.1) + 0.5,
                f64::from(key.2) + 0.5,
            ];
            self.resolve_container_maps(key, centre);
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
        let falling = std::mem::take(&mut self.falling);
        let mut kept = Vec::with_capacity(falling.len());
        let mut collision_error = None;
        let entity_drops = self.ui.rules.bool("entity_drops");
        // ServerLevel.tick: only entities in entity-ticking chunks tick.
        let ticking = self.entity_ticking_area();
        // FallingBlockEntity.tick places or drops each block in its own tick,
        // so a block falling onto one that just landed lands on top of it.
        for mut f in falling {
            if !ticking(f.position.x.floor() as i32, f.position.z.floor() as i32) {
                kept.push(f);
                continue;
            }
            let original = f.clone();
            match self.fall_step(&mut f, entity_drops) {
                Ok(FallStep::Falling) => kept.push(f),
                Ok(FallStep::Gone) => {}
                Ok(FallStep::Dropped(at)) => self.drop_falling_block(f.state, at),
                Ok(FallStep::Landed(pos)) => self.finish_falling_block(f, pos),
                Err(error) => {
                    kept.push(original);
                    collision_error.get_or_insert(error);
                }
            }
        }
        // Blocks that started falling meanwhile tick from the next tick on.
        let started = std::mem::replace(&mut self.falling, kept);
        self.falling.extend(started);
        self.process_level_events()?;
        if let Some(error) = collision_error {
            return Err(error);
        }
        Ok(())
    }
    /// One `FallingBlockEntity.tick` move of `f`.
    fn fall_step(&self, f: &mut FallingBlock, entity_drops: bool) -> Result<FallStep> {
        f.previous = f.position;
        f.age = f.age.wrapping_add(1);
        if !f.no_gravity {
            f.velocity.y -= 0.04;
        }
        let mut wanted = f.velocity;
        // Entity.move: a cobweb it was in scales this move, which then
        // starts from rest.
        if f.stuck.length_squared() > 1.0e-7 {
            wanted = wanted.multiply(f.stuck);
            f.stuck = Vec3::ZERO;
            f.velocity = Vec3::ZERO;
        }
        let mut scene = collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
            subject: Some(collisions::PowderSnowSubject {
                fall_distance: f.fall_distance,
                falling_block: true,
                ..Default::default()
            }),
        };
        let context = CollisionContext::for_entity(
            CollisionEntity {
                identity: EntityIdentity(0),
                y: f.position.y,
                descending: wanted.y < 0.0,
                main_hand: None,
            },
            false,
        );
        // On an error the caller keeps the block as it was before this tick.
        let movement = scene.resolve_with_context(
            horizon_client::falling_block::bounds(f.position),
            wanted,
            0.0,
            false,
            context,
        )?;
        let next = Vec3::new(
            f.position.x + movement.x,
            f.position.y + movement.y,
            f.position.z + movement.z,
        );
        let on_ground = wanted.y < 0.0 && movement.y != wanted.y;
        // FallingBlockEntity.tick: off the ground, it gives up after 600
        // ticks (100 outside the world) and drops itself as an item.
        let y = next.y.floor() as i32;
        let outside = y <= horizon_client::chunk::MIN_Y || y >= horizon_client::chunk::MIN_Y + 384;
        if !on_ground && (f.age > 100 && outside || f.age > 600) {
            return Ok(if f.drop_item && entity_drops {
                FallStep::Dropped(next)
            } else {
                FallStep::Gone
            });
        }
        if next.y < f.position.y {
            f.fall_distance += f.position.y - next.y;
        }
        let from = f.position;
        f.position = next;
        if on_ground {
            let pos = [
                f.position.x.floor() as i32,
                f.position.y.floor() as i32,
                f.position.z.floor() as i32,
            ];
            return Ok(FallStep::Landed(pos));
        }
        // applyEffectsFromBlocks: WebBlock.entityInside makes it stuck
        // (Entity.makeStuckInBlock, which forgets the fall). Entity.move
        // records the move unless it was blocked short of a tiny one.
        let moved = movement.length_squared();
        let recorded = moved > 1.0e-7 || wanted.length_squared() - moved < 1.0e-7;
        let world = &self.world;
        let facts = self.res.facts();
        if horizon_client::falling_block::passes_cobweb(
            from,
            f.position,
            recorded.then_some(wanted),
            |p| &*facts.get(world.block(p.x, p.y, p.z)).name == "minecraft:cobweb",
        ) {
            f.fall_distance = 0.0;
            f.stuck = horizon_client::falling_block::web_multiplier();
        }
        if movement.x != wanted.x {
            f.velocity.x = 0.0;
        }
        if movement.y != wanted.y {
            f.velocity.y = 0.0;
        }
        if movement.z != wanted.z {
            f.velocity.z = 0.0;
        }
        f.velocity = Vec3::new(
            f.velocity.x * 0.98,
            f.velocity.y * 0.98,
            f.velocity.z * 0.98,
        );
        Ok(FallStep::Falling)
    }
    /// `Entity.spawnAtLocation`: an item entity where the block was.
    fn drop_falling_block(&mut self, state: BlockStateId, at: Vec3) {
        // Entity.spawnAtLocation: an item entity where the block was.
        let item = self.res.facts().get(state).item.clone();
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
    /// `DecoratedPotBlock.playerWillDestroy`: a pot mined with an item in
    /// `breaks_decorated_pots`, unless an enchantment prevents shattering, is
    /// cracked first (flags 260), so it shatters into its sherds.
    fn crack_mined_pot(&mut self, pos: [i32; 3], state: BlockStateId) -> BlockStateId {
        let registry = &self.pack.assets.registry;
        let Some(mut pot) = registry
            .state(state)
            .filter(|s| s.name == "minecraft:decorated_pot")
            .cloned()
        else {
            return state;
        };
        let Some(held) = self.held() else {
            return state;
        };
        let breaks = self
            .res
            .items()
            .item_tags
            .get("minecraft:breaks_decorated_pots")
            .is_some_and(|t| t.contains(&*held.item));
        let enchantments = &self.res.stations().enchantments;
        let kept = held
            .enchantments()
            .iter()
            .any(|(e, _)| enchantments.in_tag("minecraft:prevents_decorated_pot_shattering", e));
        if !breaks || kept {
            return state;
        }
        pot.properties.insert("cracked".into(), "true".into());
        let Ok(cracked) = registry.resolve(&pot) else {
            return state;
        };
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.with_level(|b, level, ticks| b.set_block(level, ticks, at, cracked, 260));
        cracked
    }
    /// `ItemStack.mineBlock` for the held stack once a survival player has
    /// mined the block `name`: an item with a `tool` component counts as used
    /// and loses its `damage_per_block` on a block that doesn't break
    /// instantly (`ShearsItem`: on any block but fire).
    fn mine_block_with_held(&mut self, name: &str, hardness: f32) {
        let Some(stack) = self.inventory[self.selected].clone() else {
            return;
        };
        let Some((per_block, max)) = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| Some((i.tool_damage_per_block?, i.max_damage)))
        else {
            return;
        };
        let wears = if &*stack.item == "minecraft:shears" {
            !self.res.items().block_in_tag("minecraft:fire", name)
        } else {
            hardness != 0.0
        };
        if wears && per_block > 0 && max > 0 {
            let loss = self.durability_loss(&stack, per_block);
            if let Some(held) = self.inventory[self.selected].as_mut() {
                held.damage += loss;
                if held.damage >= max {
                    self.inventory[self.selected] = None;
                }
            }
        }
        self.stats.add("used", &stack.item, 1);
    }
    fn break_block(&mut self, pos: [i32; 3]) -> Result<()> {
        self.with_player_actor(|game| game.break_block_as_player(pos))
    }
    fn break_block_as_player(&mut self, pos: [i32; 3]) -> Result<()> {
        self.pad_rumble(horizon_client::gamepad::Rumble::BlockBroken);
        // ServerPlayerGameMode.destroyBlock.
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let facts = self.res.facts().get(state).clone();
        let state = self.crack_mined_pot(pos, state);
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
        let hive = match self.world.block_entities.get(&key) {
            Some(BlockEntity::Beehive(h)) => self.hive_state(pos).map(|s| ((**h).clone(), s)),
            _ => None,
        };
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
            self.mine_block_with_held(&facts.name, facts.hardness);
            let harvest = !facts.requires_tool
                || self
                    .res
                    .items()
                    .correct_for_drops(held.as_ref(), &facts.name);
            if harvest {
                // Block.playerDestroy: the mined statistic, the exhaustion and the drops.
                self.stats.add("mined", &facts.name, 1);
                if let Some(p) = self.player.as_mut() {
                    p.cause_food_exhaustion(0.005);
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
                // BeehiveBlock.playerDestroy: silk touch keeps the bees in.
                if let Some((hive, (honey, facing))) = hive
                    && !silk
                {
                    self.hive_broken_by_player(pos, hive, honey, facing);
                }
                // SpawnerBlock.spawnAfterBreak pops its experience whatever the tool.
                let xp = if silk && &*facts.name != "minecraft:spawner" {
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
    /// The hand whose use ended the click, if any.
    fn use_item(&mut self) -> Result<Option<Hand>> {
        self.with_player_actor(|game| game.use_item_as_player())
    }
    /// `Minecraft.startUseItem`: the main hand, then the off hand.
    fn use_item_as_player(&mut self) -> Result<Option<Hand>> {
        // Spectators only look (MultiPlayerGameMode.useItemOn/useItem pass).
        if self.spectator() {
            return Ok(None);
        }
        // A carrot or warped fungus on a stick boosts the ridden pig or strider.
        // A guest's use packets name one hand and touch no entity: the client
        // sends one per hand and part of its click, and Interact for entities.
        let guest = self.guest_use;
        if let Some(GuestUse::Entity(hand)) = guest {
            return Ok(self.with_hand(hand, Game::interact_entity)?.then_some(hand));
        }
        let on_block = matches!(guest, Some(GuestUse::OnBlock(_)));
        if !on_block
            && self.riding.is_some()
            && let Some(item) = self.held().map(|s| s.item.clone())
            && self.use_steering_stick(&item)
        {
            return Ok(Some(Hand::Main));
        }
        if guest.is_none() && self.interact_entity()? {
            return Ok(Some(Hand::Main));
        }
        // An EntityHitResult may fall through to Item.use, never to useItemOn.
        let block_hit = if matches!(guest, Some(GuestUse::Item(_))) || self.entity_targeted() {
            None
        } else {
            self.pick()
        };
        let hands = match guest {
            Some(GuestUse::OnBlock(hand) | GuestUse::Item(hand) | GuestUse::Entity(hand)) => {
                vec![hand]
            }
            None => Hand::BOTH.to_vec(),
        };
        let entity = guest.is_none() && self.entity_targeted();
        for hand in hands {
            // The off hand's turn starts on the entity too (Minecraft.startUseItem).
            if entity && hand == Hand::Off && self.with_hand(hand, Game::interact_entity)? {
                return Ok(Some(hand));
            }
            if self.with_hand(hand, |game| game.use_hand(block_hit.as_ref()))? {
                return Ok(Some(hand));
            }
        }
        Ok(None)
    }
    /// `Player.interactOn` for the entity under the crosshair with the
    /// acting hand: whether it took the click.
    fn interact_entity(&mut self) -> Result<bool> {
        Ok(self.interact_frame()
            || self.interact_armor_stand()
            || self.interact_boat()
            || self.interact_cart()
            || self.interact_mob()?
            || self.interact_knot())
    }
    /// One hand's turn in `startUseItem`: `useItemOn` the block aimed at,
    /// then `useItem`. Whether the click ends here (it acted or failed).
    fn use_hand(&mut self, block_hit: Option<&Hit>) -> Result<bool> {
        // isSecondaryUseActive with something in either hand keeps blocks
        // from reacting.
        let sneaking = self.keys.contains(&KeyCode::ShiftLeft);
        let hands_full = self.stack_in(self.selected).is_some() || self.offhand.is_some();
        let mut anchor_passed = false;
        // MultiPlayerGameMode.useItemOn: blocks react first.
        if !(sneaking && hands_full)
            && let Some(hit) = block_hit
        {
            let held = self.held().cloned();
            if self.use_lectern(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_jukebox(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_composter(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_vault(hit.pos, held.as_ref())? {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_decorated_pot(hit.pos, held.as_ref()) {
                self.swing_arm();
                return Ok(true);
            }
            if self.use_pumpkin(hit.pos, hit.face, held.as_ref())? {
                self.swing_arm();
                return Ok(true);
            }
            match self.use_respawn_anchor(hit.pos, held.as_ref())? {
                respawn_anchor::AnchorUse::Charged => {
                    self.swing_arm();
                    return Ok(true);
                }
                // The main hand passes; its item is used, then the off hand's glowstone.
                respawn_anchor::AnchorUse::Pass => anchor_passed = true,
                respawn_anchor::AnchorUse::Other => {}
            }
            // TRY_WITH_EMPTY_HAND: only the main hand's turn uses the block
            // itself (useWithoutItem).
            if !anchor_passed && self.hand == Hand::Main {
                let face = match hit.face {
                    [0, 1, 0] => Direction::Up,
                    [0, -1, 0] => Direction::Down,
                    [0, 0, -1] => Direction::North,
                    [0, 0, 1] => Direction::South,
                    [-1, 0, 0] => Direction::West,
                    _ => Direction::East,
                };
                let player = self.use_context();
                let at = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
                let item = held.as_ref().map(|s| s.item.to_string());
                let result = self.with_level(|b, level, ticks| {
                    b.use_item_on(
                        level,
                        ticks,
                        BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]),
                        face,
                        at,
                        player,
                        item.as_deref(),
                    )
                });
                self.process_level_events()?;
                if result.consumes_action() {
                    self.swing_arm();
                    return Ok(true);
                }
            }
        }
        let Some(stack) = self.held().cloned() else {
            return Ok(false);
        };
        let Some(info) = self.res.items().get(&stack.item).cloned() else {
            return Ok(false);
        };
        if block_hit.is_some() && (self.use_bone_meal(&stack)? || self.ignite_tnt(&stack)?) {
            return Ok(true);
        }
        // A guest's UseItemOn stops at ItemStack.useOn: what Item.use does
        // (throw, drink, eat, raise, equip) waits for its UseItem.
        let on_block = matches!(self.guest_use, Some(GuestUse::OnBlock(_)));
        let spawn_egg = stack.item.ends_with("_spawn_egg");
        use horizon_client::first_person::UseResult;
        let swings = self.swings_started;
        match self.use_tool(&stack, block_hit.is_some())? {
            UseResult::Success { swing } => {
                if (UseResult::Success { swing }).should_swing(swings, self.swings_started) {
                    self.swing_arm();
                }
                return Ok(true);
            }
            UseResult::Fail => return Ok(true),
            UseResult::Pass => {}
        }
        if (!on_block && self.use_potion_item(&stack)?)
            || ((block_hit.is_some() || !spawn_egg)
                && (!on_block || spawn_egg)
                && self.throw_item(&stack)?)
        {
            return Ok(true);
        }
        if !on_block
            && (self.raise_shield(self.held_slot())
                || self.use_fishing_rod(&stack)
            || self.use_crossbow(&stack)
            || self.use_trident(&stack)
            || self.boost_firework(&stack)
            || self.use_empty_map(&stack)
                || self.use_book(&stack)
                // Block items (heads, carved pumpkins) aimed at a block are
                // placed first (useItemOn before useItem).
                || (!(info.block.is_some() && block_hit.is_some())
                    && self.equip_from_hand(&stack)))
        {
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:spyglass" {
            self.use_spyglass();
            return Ok(true);
        }
        if !on_block && &*stack.item == "minecraft:bow" {
            if self.using.is_none()
                && (self.creative || self.projectile_slot("minecraft:bow").is_some())
            {
                self.using = Some((self.held_slot(), 0));
            }
            return Ok(true);
        }
        // Block foods attempt placement before starting consumption.
        if !on_block && (info.block.is_none() || block_hit.is_none()) && self.start_eating(&info) {
            return Ok(true);
        }
        // BoatItem.use also performs its own fluid/block raycast.
        if !on_block && self.place_boat(&stack) {
            return Ok(true);
        }
        if let Some(hit) = block_hit
            && (self.launch_firework(hit, &stack)
                || self.place_minecart(hit, &stack)
                || self.place_painting(hit, &stack)
                || self.place_item_frame(hit, &stack)
                || self.place_armor_stand(hit, &stack)
                || self.place_end_crystal(hit, &stack))
        {
            return Ok(true);
        }
        if self.use_ender_eye(&stack, block_hit.is_some())? {
            return Ok(true);
        }
        let Some(hit) = block_hit else {
            return Ok(false);
        };
        if info.block.is_none() {
            return Ok(false);
        }
        if !self.may_build() {
            // ItemStack.useOn passes in adventure mode; useItem eats.
            return Ok(!on_block && self.start_eating(&info));
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
            can_use_game_master_blocks: self.can_use_game_master_blocks(),
        };
        let outcome = self.with_level(|b, level, ticks| b.place_block(level, ticks, &ctx));
        let PlaceOutcome::Placed { pos, state } = outcome else {
            self.process_level_events()?;
            // BlockItem.useOn: a consumable that could not be placed is used,
            // which ItemStack.useOn counts as a use of the item.
            if self.start_eating(&info) {
                self.stats.add("used", &stack.item, 1);
            }
            return Ok(true);
        };
        let target = [pos.x, pos.y, pos.z];
        if let Some(entity) = BlockEntity::for_block(&self.res.facts().get(state).name) {
            let key = (target[0], target[1], target[2]);
            self.world.block_entities.insert(key, entity);
            let game_master = self.can_use_game_master_blocks();
            self.apply_placed_item(target, &stack, game_master);
            // Block.setPlacedBy.
            match self.world.block_entities.get(&key) {
                Some(BlockEntity::CommandBlock(_)) => {
                    let data = stack.data().is_some_and(|d| {
                        d.other(horizon_client::block_entity_item::BLOCK_ENTITY_DATA)
                            .is_some()
                    });
                    self.command_block_placed(key, !data);
                }
                Some(BlockEntity::Sign(sign)) if !sign.waxed => self.open_sign_editor(key, true),
                _ => {}
            }
        }
        self.swing_arm();
        // BlockItem.place's sound is among the level events;
        // SolidBucketItem.getPlaceSound replaces it with the bucket's own.
        let solid_bucket = &*stack.item == "minecraft:powder_snow_bucket";
        if solid_bucket {
            for event in &mut self.level_events {
                if let LevelEvent::Sound { pos: at, sound, .. } = event
                    && *at == pos
                {
                    *sound = "minecraft:item.bucket.empty_powder_snow".into();
                }
            }
        }
        self.process_level_events()?;
        // BlockItem.place: the used statistic and the placed_block trigger.
        self.stats.add("used", &stack.item, 1);
        self.trigger_block("minecraft:placed_block", target, &stack);
        self.check_wither_spawn(target)?;
        self.check_golem_spawn(target)?;
        self.consume_held();
        // SolidBucketItem.useOn: BucketItem.getEmptySuccessItem in hand.
        if solid_bucket && !self.creative {
            *self.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
        }
        Ok(true)
    }
    /// The player as a block's `useItemOn` / `useWithoutItem` sees them.
    fn use_context(&self) -> PlayerContext {
        PlayerContext {
            yaw: self.yaw,
            pitch: self.pitch,
            may_build: self.may_build(),
            can_eat: self.creative
                || self
                    .player
                    .as_ref()
                    .is_some_and(|p| p.health.food.food < 20),
            game_master: self.can_use_game_master_blocks(),
        }
    }
    /// `Consumable.startConsuming` for a held consumable: food needs
    /// `Player.canEat` (`Consumable.canConsume`), anything else is always
    /// consumed. The right click is held for `consume_seconds`.
    fn start_eating(&mut self, info: &horizon_client::items::ItemInfo) -> bool {
        if info.consumable.is_none()
            || (info.food.is_some()
                && !self.player.as_ref().is_some_and(|p| {
                    info.can_always_eat || p.health.food.food < 20 || self.creative
                }))
        {
            return false;
        }
        if self.using.is_none() {
            self.using = Some((self.held_slot(), 0));
        }
        true
    }
    fn apply_consume_effect(&mut self, effect: &horizon_client::items::ConsumeEffect) {
        use horizon_client::items::ConsumeEffect;
        match effect {
            ConsumeEffect::ApplyEffects {
                effects,
                probability,
            } => {
                // ApplyStatusEffectsConsumeEffect.apply: the user's random
                // (the level's stands in, as for every player draw) is drawn
                // even at probability 1.
                if self.world.random.next_float() < *probability {
                    for eff in effects {
                        if let Some(def) = horizon_client::brewing::effect(&eff.id) {
                            if def.instant {
                                if let Some(a) =
                                    horizon_client::effects::instant(def.id, eff.amplifier, 1.0)
                                {
                                    self.apply_effect_action(a);
                                }
                            } else {
                                self.add_player_effect(
                                    horizon_client::effects::EffectInstance::new(
                                        def.id,
                                        eff.duration,
                                        eff.amplifier,
                                    ),
                                );
                            }
                        }
                    }
                }
            }
            ConsumeEffect::ClearAllEffects => {
                self.potions.effects.clear();
                self.apply_effect_status();
            }
            ConsumeEffect::RemoveEffects(to_remove) => {
                for target in to_remove {
                    let id = target.trim_start_matches("minecraft:");
                    self.potions.effects.list.retain(|e| e.id != id);
                }
                self.apply_effect_status();
            }
            ConsumeEffect::TeleportRandomly(teleport) => self.teleport_randomly(teleport),
            ConsumeEffect::PlaySound(sound) => {
                // PlaySoundConsumeEffect: at the user's block position.
                let at = self.player.as_ref().map(|p| {
                    let b = horizon_core::position::BlockPos::containing(p.position);
                    [b.x, b.y, b.z].map(|v| f64::from(v) + 0.5)
                });
                self.audio.play(sound, at, 1.0, 1.0);
            }
        }
    }
    fn consume_held(&mut self) {
        if self.creative {
            return;
        }
        let cell = self.held_mut();
        if let Some(s) = cell.as_mut() {
            s.count -= 1;
            if s.count == 0 {
                *cell = None;
            }
        }
    }
    fn tick_using(&mut self) {
        let Some(hand) = self.used_hand() else {
            return;
        };
        self.with_hand(hand, Self::tick_used_item);
    }
    /// `LivingEntity.updatingUsingItem` for the item in use, its hand acting.
    fn tick_used_item(&mut self) {
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
            self.using = None;
            self.release_used_item(&stack, ticks, hand_changed);
            return;
        }
        let Some(info) = self.res.items().get(&stack.item).cloned() else {
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
        let Some(consumable) = info.consumable.as_ref() else {
            self.using = None;
            return;
        };
        // ItemStack.onUseTick: Consumable.shouldEmitParticlesAndSounds with
        // the remaining ticks before this tick counts them down.
        let duration = consumable.ticks().max(0) as u32;
        let remaining = duration - ticks.min(duration);
        let ticks = ticks + 1;
        if consumable.should_emit(remaining as i32) {
            self.emit_consume(consumable, &stack, 5);
        }
        if ticks >= duration {
            self.using = None;
            self.finish_consuming(&info, consumable, &stack);
        } else {
            self.using = Some((slot, ticks));
        }
    }
    /// `LivingEntity.releaseUsingItem` (`Item.releaseUsing`) for `stack`,
    /// used for `ticks`, its hand acting: a bow or a trident lets go (not
    /// when its hand changed), a spyglass stops scoping.
    pub(super) fn release_used_item(&mut self, stack: &ItemStack, ticks: u32, hand_changed: bool) {
        match &*stack.item {
            "minecraft:bow" if !hand_changed => self.release_bow(ticks),
            "minecraft:trident" if !hand_changed => self.release_trident(ticks),
            "minecraft:spyglass" => self.stop_spyglass(),
            _ => {}
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
            self.attack_block(hit.pos)?;
            self.mining = Some(Mining {
                pos: hit.pos,
                progress: 0.0,
                ticks: 0,
            });
        }
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        let facts = self.res.facts().get(state);
        if facts.hardness < 0.0 {
            return Ok(());
        }
        // Player.getDestroySpeed / BlockBehaviour.getDestroyProgress.
        let held = self.held();
        let mut speed = self.res.items().mining_speed(held, &facts.name);
        speed = self.modify_destroy_speed(speed);
        let player = self.player.as_ref().ok_or("no player")?;
        if !player.ground.on_ground {
            speed /= 5.0;
        }
        let eye = self.eye_position().unwrap_or_default();
        if self
            .res
            .facts()
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
        let harvest = !facts.requires_tool || self.res.items().correct_for_drops(held, &facts.name);
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
        // ServerPlayerGameMode breaks a player's block at once only when it is instant (START's
        // insta-mine); otherwise it waits for the client's STOP_DESTROY_BLOCK
        // (`guest_stop_digging`), never breaking it on its own count. Breaking it here would
        // take the block from under the player's crack before the player's client predicts the
        // break, so it gets neither particles nor sound.
        if m.progress >= 1.0 && (self.lan_acting.is_none() || m.ticks == 1) {
            let pos = m.pos;
            self.mining = None;
            self.break_block(pos)?;
            self.destroy_delay = 5;
        }
        Ok(())
    }
    /// A column whose surface is a full solid, non-fluid, non-leaf block open to the sky.
    fn land_column(&self, x: i32, z: i32) -> Option<i32> {
        if self.dim == Dim::Nether {
            // Below the bedrock ceiling: a solid floor with two air blocks above.
            for y in (32..120).rev() {
                let floor = self.res.facts().get(self.world.block(x, y - 1, z));
                if floor.full_collision
                    && floor.fluid == Fluid::None
                    && self.res.facts().is_air(self.world.block(x, y, z))
                    && self.res.facts().is_air(self.world.block(x, y + 1, z))
                {
                    return Some(y);
                }
            }
            return None;
        }
        let mut y = MIN_Y + HEIGHT - 1;
        while y > MIN_Y {
            let s = self.world.block(x, y, z);
            if !self.res.facts().is_air(s) {
                let f = self.res.facts().get(s);
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
            if self.win_screen_holds_arrival() {
                return Ok(());
            }
            let (cx, cz) = (
                (target.x.floor() as i32) >> 4,
                (target.z.floor() as i32) >> 4,
            );
            self.streaming.center = (cx, cz);
            if (-2..=2).all(|dz| (-2..=2).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                let (player, target) = self.travelling.take().ok_or("travel")?;
                if let Some(config) = self.respawn_travel.take() {
                    drop(player);
                    self.place_respawned(config)?;
                } else if self.end_travel {
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
        let feet = if let Some(p) = saved {
            let (cx, cz) = ((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4);
            self.streaming.center = (cx, cz);
            if !(-1..=1).all(|dz| (-1..=1).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                return Ok(());
            }
            self.level.position = None;
            p
        } else if self.spawn.is_none() || self.player_spawn.is_some() {
            // A new world: its spawn, then the first player around it.
            if self.spawn.is_none() {
                let Some([x, y, z]) = self.find_world_spawn()? else {
                    return Ok(());
                };
                self.spawn = Some(Vec3::new(
                    f64::from(x) + 0.5,
                    f64::from(y),
                    f64::from(z) + 0.5,
                ));
                self.level.spawn = Some([f64::from(x), f64::from(y), f64::from(z)]);
                // The world's spawn from the start, not only once saved: a bed set
                // before the first save must not become it.
                self.level.world_spawn.get_or_insert([x, y, z]);
            }
            let s = self
                .level
                .spawn
                .unwrap_or_default()
                .map(|v| v.floor() as i32);
            let Some(feet) = self.find_player_spawn(s)? else {
                return Ok(());
            };
            feet
        } else {
            let s = self.spawn.unwrap_or_default();
            let (cx, cz) = ((s.x.floor() as i32) >> 4, (s.z.floor() as i32) >> 4);
            self.streaming.center = (cx, cz);
            if !(-1..=1).all(|dz| (-1..=1).all(|dx| self.streaming.is_lit(cx + dx, cz + dz))) {
                return Ok(());
            }
            // WorldSpawn: the stored column, lifted onto its current surface.
            let y = self
                .land_column(s.x.floor() as i32, s.z.floor() as i32)
                .map_or(s.y, f64::from);
            Vec3::new(s.x, y.max(s.y), s.z)
        };
        let mut player = PlayerMotion::new(self.motion, feet)?;
        player.yaw = self.yaw;
        player.load_saved_state_once(&mut self.restore_player_state, &self.level)?;
        let spectator = self.mode == horizon_client::command::args::GameMode::Spectator;
        player.may_fly = self.creative || spectator;
        player.invulnerable = self.creative || spectator;
        player.spectator = spectator;
        if spectator {
            player.flying = true;
        } else if !player.may_fly {
            player.flying = false;
        }
        player.respiration = self.armor[0]
            .as_ref()
            .map_or(0, |s| s.enchantment_level("minecraft:respiration") as i32);
        player.pitch = self.pitch;
        player.glider = self.glider_worn();
        // A saved death already dropped inventory/XP and counted the death.
        if player.health.dead_or_dying() {
            self.screen = Screen::Dead;
            self.ui.died_at = Some(Instant::now());
            self.ui.death_text = None;
            self.capture(false);
        }
        self.previous_position = feet;
        self.player = Some(player);
        self.pad.player_replaced();
        self.sway = Default::default();
        self.apply_equipment();
        self.eye = 1.62;
        self.previous_eye = 1.62;
        self.clock.reset();
        self.capture_loaded_world();
        Ok(())
    }
    /// Landing rules (farmland trampling, turtle eggs) wrote or removed the block
    /// raw: finish `Level.setBlock`'s neighbour updates, so a crop on trampled
    /// farmland breaks. The local player's and every server player's landings.
    fn landing_block_updates(&mut self, event: &horizon_client::landing::FallEvent) {
        use horizon_client::landing::FallEvent;
        let (position, flags) = match *event {
            FallEvent::Write {
                position, flags, ..
            } => (position, flags as i32),
            FallEvent::Destroy { position, .. } => (position, level_flags::UPDATE_ALL),
            _ => return,
        };
        self.with_level(|b, level, ticks| b.notify_written(level, ticks, position, flags));
    }
    fn tick(&mut self) -> Result<()> {
        self.tick_debug_keys()?;
        self.tick_debug_charts();
        if self.remote.is_some() {
            return self.tick_remote();
        }
        // LevelLoadingScreen.tick.
        self.loading_progress = match self.loading_reason() {
            Some(_) => horizon_client::level_loading::smooth(
                self.loading_progress,
                self.level_load_progress(),
            ),
            None => 0.0,
        };
        self.ticks += 1;
        self.track_experience_display();
        // `/tick freeze` stops the world but not the player.
        let running = self.clock.world_runs();
        // GameRenderer.tick: a wither's bar darkens the world, then fades.
        self.boss_overlay.tick_darken(running);
        if running {
            self.world.game_time += 1;
            if self.ui.rules.bool("advance_weather") {
                self.world.weather.tick(&mut self.world.random);
            }
            self.tick_weather_effects();
        }
        if self.ticks.is_multiple_of(6000) {
            self.save_game_with(false)?;
        }
        if running {
            self.advance_clock();
        }
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        self.tick_totem();
        self.update_totem_ready();
        self.tick_command_extras();
        self.tick_mods();
        let camera_eye = self.camera_eye_height();
        let stick = self.pad_move();
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
                analog: None,
            }
            .with_stick(stick.filter(|_| playing));
            self.tick_riding(keys)?;
        }
        let riding = self.riding.is_some();
        let entity_collisions = self.player_entity_collisions();
        let spin_targets = self.spin_targets();
        self.sync_player_body();
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
        let playing = self.screen == Screen::Playing && self.sleeping.is_none();
        let held = |k| playing && self.keys.contains(&k);
        let keys = Keys {
            forward: held(KeyCode::KeyW),
            backward: held(KeyCode::KeyS),
            left: held(KeyCode::KeyA),
            right: held(KeyCode::KeyD),
            jump: held(KeyCode::Space),
            shift: held(KeyCode::ShiftLeft),
            sprint: held(KeyCode::ControlLeft),
            analog: None,
        }
        .with_stick(stick.filter(|_| playing));
        if !riding {
            self.previous_position = player.position;
        }
        let spectator = self.mode == horizon_client::command::args::GameMode::Spectator;
        player.may_fly = self.creative || spectator;
        player.invulnerable = self.creative || spectator;
        player.spectator = spectator;
        player.spin_targets = spin_targets;
        let grounded = player.ground.on_ground;
        let before = (
            grounded,
            [player.velocity.x, player.velocity.y, player.velocity.z],
            [player.position.x, player.position.y, player.position.z],
        );
        if !riding {
            MotionWorld {
                entities: &entity_collisions,
                player,
                scene: &mut self.world,
                assets: &self.pack.assets,
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
        let spin_hit = player.spin_hit.take();
        let glider_wear = std::mem::take(&mut player.glider_wear);
        if glider_wear > 0
            && !self.creative
            && let Some(elytra) = self.armor[1].as_mut()
        {
            elytra.damage += glider_wear;
        }
        let feet = [player.position.x, player.position.y, player.position.z];
        let player_velocity = [player.velocity.x, player.velocity.y, player.velocity.z];
        self.previous_eye = self.eye;
        let eye_height = if self.sleeping.is_some() {
            0.2
        } else {
            player.dimensions.eye_height
        };
        self.eye += (camera_eye.unwrap_or(eye_height) - self.eye) * 0.5;
        let crouching = player.pose == horizon_client::pose::Pose::Crouching;
        self.tick_avatar(before, riding);
        self.end_spin_attack(spin_hit);
        self.record_combat(&events);
        for event in events {
            match event {
                MotionEvent::HurtSound(sound) => self.player_hurt_sound(sound, feet),
                MotionEvent::Emission {
                    state, in_water, ..
                } if !crouching => {
                    let sounds = self.res.facts().get(state).sounds.clone();
                    // Entity.playStepSound: the block's step sound at 0.15 volume,
                    // muffled under the swim sound in water.
                    let (volume, pitch) = if in_water {
                        (sounds.volume * 0.05, sounds.pitch * 0.8)
                    } else {
                        (sounds.volume * 0.15, sounds.pitch)
                    };
                    self.audio.play(&sounds.step, Some(feet), volume, pitch);
                }
                MotionEvent::Swim { volume, pitch } => {
                    self.audio
                        .play("minecraft:entity.player.swim", Some(feet), volume, pitch);
                }
                MotionEvent::Drown => {
                    // LivingEntity.makeDrownParticles: eight bubbles moving
                    // with the player.
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    for _ in 0..8 {
                        let mut triangle = || {
                            let r = self.particles.random();
                            r.next_double() - r.next_double()
                        };
                        let p = [
                            feet[0] + triangle(),
                            feet[1] + triangle(),
                            feet[2] + triangle(),
                        ];
                        self.particles.add(
                            &ParticleOptions::simple(ParticleType::Bubble),
                            p,
                            player_velocity,
                        );
                    }
                }
                MotionEvent::BubbleColumnParticle { bubble, position } => {
                    // ServerLevel.sendParticles: SPLASH (speed 1) or BUBBLE
                    // (y spread 0.01, speed 0.2), one each.
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let (kind, spread, speed) = if bubble {
                        (ParticleType::Bubble, [0.0, 0.01, 0.0], 0.2)
                    } else {
                        (ParticleType::Splash, [0.0; 3], 1.0)
                    };
                    self.particles.send_particles(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        spread,
                        speed,
                        1,
                    );
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
                MotionEvent::Fall(event) => self.landing_block_updates(&event),
                MotionEvent::Snowflake { position, velocity } => self.snowflake(position, velocity),
                MotionEvent::EnterBlock { state } => self.trigger_enter_block(state),
                MotionEvent::InsidePortal { pos } => self.player_inside_nether_portal(pos),
                MotionEvent::SmashLanding {
                    state,
                    position,
                    fall_distance,
                } => self.smash_landing_particles(state, position, fall_distance),
                MotionEvent::Splash {
                    high_speed,
                    volume,
                    pitch,
                } => {
                    let name = if high_speed {
                        "minecraft:entity.player.splash.high_speed"
                    } else {
                        "minecraft:entity.player.splash"
                    };
                    self.audio.play(name, Some(feet), volume, pitch);
                }
                MotionEvent::SplashParticle {
                    bubble,
                    position,
                    velocity,
                } => {
                    use horizon_client::particles::{ParticleOptions, ParticleType};
                    let kind = if bubble {
                        ParticleType::Bubble
                    } else {
                        ParticleType::Splash
                    };
                    self.particles.add(
                        &ParticleOptions::simple(kind),
                        [position.x, position.y, position.z],
                        [velocity.x, velocity.y, velocity.z],
                    );
                }
                _ => {}
            }
        }
        self.powder_snow_fire()?;
        self.tick_totem();
        let Some(player) = self.player.as_mut() else {
            return Ok(());
        };
        if player.position.y < f64::from(MIN_Y - 64) {
            // Out of the world.
            let _ = player.set_health(0.0, false);
        }
        if player.health.dead_or_dying() && self.screen != Screen::Dead {
            // Inventory.dropAll: every stack thrown in a random direction.
            let feet = [player.position.x, player.position.y, player.position.z];
            let at = [feet[0], feet[1] + 1.32, feet[2]];
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
            let reward = self
                .experience
                .death_reward(self.ui.rules.bool("keep_inventory") || self.spectator());
            self.spawn_orbs(feet, reward);
            if !self.spectator() {
                self.drop_inventory(at);
            }
            self.audio
                .play("minecraft:entity.player.death", Some(at), 1.0, 1.0);
            // LivingEntity.die: the LAN guests see the host's death animation.
            if let Some(host) = &self.lan {
                host.hub.entity_event(host.host_entity, 3, feet);
            }
            self.screen = Screen::Dead;
            self.capture(false);
        }
        self.tick_world(running)
    }
    /// What the tick runs after the local player's own update: effects, the
    /// world's entities and blocks, and the network hub.
    fn tick_world(&mut self, running: bool) -> Result<()> {
        self.update_chunk_tickets();
        horizon_client::spans::time("tick_effects", || self.tick_effects(running));
        // Player.tick's ItemCooldowns.tick for the window's player (a
        // server's players count theirs down in `tick_guests`).
        self.shield_cooldown = self.shield_cooldown.saturating_sub(1);
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
            horizon_client::spans::time("tick_ominous_item_spawners", || {
                self.tick_ominous_item_spawners()
            });
        }
        if running {
            horizon_client::spans::time("entity_portals", || self.handle_entity_portals());
            horizon_client::spans::time("tick_items", || self.tick_items());
            horizon_client::spans::time("touch_arrows", || self.touch_arrows());
        }
        horizon_client::spans::time("spawn_loaded_entities", || self.spawn_loaded_entities());
        if running {
            horizon_client::spans::time("tick_mobs", || self.tick_mobs())?;
            horizon_client::spans::time("recheck_golem_openers", || self.recheck_golem_openers());
            horizon_client::spans::time("tick_experience", || self.tick_experience());
            horizon_client::spans::time("entity_portals", || {
                self.find_entity_portals();
                self.resolve_arrivals();
            });
        }
        horizon_client::spans::time("tick_lan", || self.tick_lan(false));
        self.tick_friends();
        horizon_client::spans::time("tick_hand", || self.tick_hand());
        horizon_client::spans::time("tick_sleep", || self.tick_sleep());
        horizon_client::spans::time("tick_portal", || self.tick_nether_portal())?;
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
        }
        if running {
            // ClientLevel.tickBlockEntities (not while paused or frozen):
            // chest lids, books, shulker boxes, bells, conduits, heads.
            horizon_client::spans::time("tick_block_entity_anims", || {
                self.tick_block_entity_anims()
            });
            horizon_client::spans::time("tick_particles", || self.tick_particles());
        }
        // ServerBossEvent broadcasts, after this level's entities have ticked.
        self.publish_boss_bars();
        // Minecraft.handleKeybinds: held use repeats only while no item is in
        // use (a raised shield or a drawn bow places and uses nothing).
        if self.right
            && self.use_delay == 0
            && self.using.is_none()
            && self.screen == Screen::Playing
            && self.sleeping.is_none()
        {
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
    /// A key event after rebinding, as the default key of its mapping (the
    /// keyboard and controllers both end here).
    fn mapped_key(&mut self, key: KeyCode, pressed: bool, repeat: bool) -> Result<()> {
        // Sneak and sprint are ToggleKeyMappings: a press (or a key
        // repeat) flips them when toggling; only releases reach them
        // behind a screen (KeyboardHandler.keyPress).
        let toggle = match key {
            KeyCode::ShiftLeft => Some((0, self.settings.toggle_crouch)),
            KeyCode::ControlLeft => Some((1, self.settings.toggle_sprint)),
            _ => None,
        };
        if let Some((slot, option)) = toggle {
            let action = match (pressed, repeat) {
                (false, _) => KeyAction::Release,
                (_, false) => KeyAction::Press,
                (_, true) => KeyAction::Repeat,
            };
            let screen_open = self.screen != Screen::Playing || self.title.is_some();
            self.toggles.key(slot, action, screen_open, option);
            self.sync_toggle(slot);
            if !pressed {
                return Ok(());
            }
        } else if !pressed {
            self.keys.remove(&key);
            return Ok(());
        } else {
            self.keys.insert(key);
        }
        // KeyboardHandler.keyPress: a repeat (InputConstants.REPEAT) reaches
        // Screen.keyPressed and KeyMapping.click like a press, so holding
        // key.drop keeps throwing, in game and over a menu slot.
        if repeat && key != KeyCode::KeyQ {
            return Ok(());
        }
        self.key_pressed(key)
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
            let zoom = self.zoom_turn_scale();
            rotation.turn(xo * zoom, yo * zoom);
            self.yaw = rotation.y_rot;
            self.pitch = rotation.x_rot;
        }
    }
    /// `Minecraft.pause`: a single-player game with a pausing screen open.
    fn pauses_game(&self) -> bool {
        self.has_singleplayer_server()
            && !self.is_published()
            && self.title.is_none()
            && (matches!(
                self.screen,
                Screen::Paused
                    | Screen::Options
                    | Screen::Stats
                    | Screen::Advancements
                    | Screen::WorldOptions
                    | Screen::Friends
                    | Screen::CommandBlock
            ) || self.book.is_some()
                || self.win_screen_pauses())
    }
    /// `MouseHandler.grabMouse` / `releaseMouse`. Both put the pointer at
    /// the window's centre, so a screen opened from gameplay (inventory,
    /// chest, pause, chat, sign, book, death) starts with the cursor in the
    /// middle instead of wherever the system last left it. Releasing when
    /// already released does nothing, so moving from screen to screen keeps
    /// the cursor where it is. Like GLFW's `glfwSetCursorPos`, nothing warps
    /// while the window is unfocused (losing focus releases the pointer).
    fn capture(&mut self, capture: bool) {
        // A guest's action (`as_guest`) opens and closes screens on the
        // guest's own client: the window's pointer stays the host's (#687).
        if self.lan_acting.is_some() {
            return;
        }
        // MouseHandler.grabMouse never captures an inactive window.
        if capture && !self.focused {
            return;
        }
        if !capture {
            self.stop_mining();
        }
        // The real pointer takes over from an agent's.
        self.agent_pointer = false;
        let Some(window) = &self.window else { return };
        let size = self.renderer.as_ref().map_or_else(
            || window.inner_size().into(),
            |r| [r.config.width, r.config.height],
        );
        let centre =
            winit::dpi::PhysicalPosition::new(f64::from(size[0]) / 2.0, f64::from(size[1]) / 2.0);
        let warp = |window: &Window| {
            let _ = window.set_cursor_position(centre);
        };
        if capture {
            self.mouse.set_ignore_first_move();
            // macOS warps by re-associating the mouse with the pointer,
            // which undoes a Locked grab and leaves the pointer free: warp
            // first there. Wayland moves a pointer only while it is locked,
            // so everywhere else warp after grabbing.
            #[cfg(target_os = "macos")]
            warp(window);
            self.captured = window
                .set_cursor_grab(CursorGrabMode::Locked)
                .or_else(|_| window.set_cursor_grab(CursorGrabMode::Confined))
                .is_ok();
            #[cfg(not(target_os = "macos"))]
            if self.captured && self.focused {
                warp(window);
            }
        } else {
            if self.captured {
                // Wayland moves a pointer only as the locked pointer's
                // position hint, which the compositor applies as the lock
                // goes (sway and Hyprland do): set it before ungrabbing.
                // X11, Windows and macOS warp, so warp again once free
                // (Wayland refuses that one).
                if self.focused {
                    warp(window);
                }
                let _ = window.set_cursor_grab(CursorGrabMode::None);
                if self.focused {
                    warp(window);
                }
                let scale = HudBatch::new(size).scale;
                self.cursor = (size[0] as f32 / 2.0 / scale, size[1] as f32 / 2.0 / scale);
            } else {
                let _ = window.set_cursor_grab(CursorGrabMode::None);
            }
            self.captured = false;
        }
        window.set_cursor_visible(!self.captured);
    }
    /// LevelLoadingScreen.onClose -> Gui.setScreen(null) -> grabMouse.
    fn capture_loaded_world(&mut self) {
        // Loading may finish after focus was lost or another screen opened.
        if self.title.is_none() && self.screen == Screen::Playing {
            self.capture(true);
        }
    }
    fn draw_item(&mut self, batch: &mut HudBatch, stack: &ItemStack, x: f32, y: f32) {
        // `GuiGraphics.renderItem`: the resolved item model (tints, foil and
        // special renderers included), then the decorations.
        self.draw_item_model(batch, stack, x, y);
        self.draw_item_decorations(batch, stack, x, y, None);
    }
    /// `extractSlot` of slot `slot` holding `stack` at (x, y): during a quick
    /// craft drag a dragged slot shows what it would hold over a white
    /// highlight (its count yellow when the slot caps it), and hides while
    /// it is the drag's only slot.
    fn draw_slot_item(
        &mut self,
        batch: &mut HudBatch,
        drag: Option<&DragView>,
        slot: usize,
        stack: Option<ItemStack>,
        x: f32,
        y: f32,
    ) {
        let (stack, capped) = match drag.and_then(|d| d.slots.get(&slot)) {
            Some(None) => return,
            Some(Some((count, capped))) => {
                batch.fill(&self.gui.atlas, x, y, 16.0, 16.0, rgba(255, 255, 255, 128));
                let carried = self
                    .carried
                    .clone()
                    .map(|c| ItemStack { count: *count, ..c });
                (carried, *capped)
            }
            None => (stack, false),
        };
        if let Some(stack) = stack {
            self.draw_item_model(batch, &stack, x, y);
            let text = capped.then(|| format!("\u{a7}e{}", stack.count));
            self.draw_item_decorations(batch, &stack, x, y, text);
        }
    }
    /// `extractCarriedItem`: the carried stack at the cursor, showing what a
    /// quick craft drag over several slots leaves in hand.
    fn draw_carried(&mut self, batch: &mut HudBatch) {
        let Some(mut stack) = self.carried.clone() else {
            return;
        };
        if let Some(left) = self.drag_view().and_then(|d| d.left) {
            if left == 0 {
                return;
            }
            stack.count = left;
        }
        self.draw_item(batch, &stack, self.cursor.0 - 8.0, self.cursor.1 - 8.0);
    }
    /// The quick craft drag in progress, as the menu engine previews it.
    fn drag_view(&mut self) -> Option<DragView> {
        let (ty, slots) = self.inv_drag.clone()?;
        let mut view = None;
        self.menu_session(|menu, env, to_menu| {
            let menu_slots: Option<Vec<usize>> = slots.iter().map(|&g| to_menu(g)).collect();
            let shown = match menu.quick_craft_preview(env, ty, &menu_slots?) {
                Some((shown, left)) => {
                    let slots = slots.iter().copied().zip(shown).collect();
                    DragView {
                        slots,
                        left: Some(left),
                    }
                }
                // A single dragged slot is hidden.
                None => DragView {
                    slots: slots.iter().map(|&g| (g, None)).collect(),
                    left: None,
                },
            };
            view = Some(shown);
            Some(false)
        });
        view
    }
    /// Whether a quick craft drag takes `slot`
    /// (`shouldAddSlotToQuickCraft`); screens the menu engine doesn't
    /// model take every slot.
    fn quick_craft_accepts(&mut self, slot: usize, ty: i32, dragged: usize) -> bool {
        let mut accepts = true;
        self.menu_session(|menu, env, to_menu| {
            accepts = to_menu(slot).is_some_and(|s| menu.quick_craft_accepts(env, s, ty, dragged));
            Some(false)
        });
        accepts
    }
    /// `GuiGraphicsExtractor.itemDecorations`: the count (or `count_text`)
    /// and damage bar.
    fn draw_item_decorations(
        &mut self,
        batch: &mut HudBatch,
        stack: &ItemStack,
        x: f32,
        y: f32,
        count_text: Option<String>,
    ) {
        // GuiGraphicsExtractor.itemDecorations, laid out at the origin and
        // moved to (x, y). Horizon tracks no item cooldowns to shade yet.
        let max_damage = self
            .res
            .items()
            .get(&stack.item)
            .map_or(0, |info| info.max_damage as i32);
        let decorations = hud_overlays::ItemDecorations {
            count: stack.count,
            count_text,
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
            &self.gui.atlas.font,
        ));
        draws.push(hud_layout::GuiDraw::Pop);
        hud_layout::paint(batch, &self.gui.atlas, &draws);
    }
    fn hud(&mut self, size: [u32; 2]) -> HudBatch {
        // The pack prompt replaces the connecting screen (setScreen), but
        // in game it opens over the world.
        let mut batch = if self.server_packs.prompt.is_some() && self.player.is_none() {
            HudBatch::new(size)
        } else {
            self.hud_layers(size)
        };
        self.draw_pack_prompt(&mut batch);
        // Gui.extractRenderState: the debug overlay comes after the screen
        // and the toasts, so it is over everything, menus included.
        self.draw_debug_overlay(&mut batch, size);
        self.frame_graph
            .draw(self.settings.horizon.overlay, &mut batch, &self.gui.atlas);
        self.draw_pad_cursor(&mut batch);
        batch
    }
    fn hud_layers(&mut self, size: [u32; 2]) -> HudBatch {
        let mut batch = HudBatch::new(size);
        let (w, h) = (batch.width, batch.height);
        if self.screen == Screen::Win {
            self.draw_win_screen(&mut batch);
            return batch;
        }
        if self.screen == Screen::Options {
            self.draw_options(&mut batch);
            return batch;
        }
        if let Some(title) = &mut self.title {
            title.poll_backup();
            if let Some(sound) = title.update_toast(self.settings.notification_display_time) {
                self.audio.play(sound, None, 1.0, 1.0);
            }
        }
        if let Some(title) = &self.title {
            title.draw(
                &self.gui.atlas,
                &mut batch,
                self.cursor,
                self.settings.file.bool("hideSplashTexts"),
            );
            return batch;
        }
        let atlas_ptr: *const HudAtlas = &self.gui.atlas;
        let _ = atlas_ptr;
        // The ToastManager draws over every screen: a server's packs
        // download while the connection screen shows.
        if self.remote_overlay(&mut batch) {
            self.draw_pack_toasts(&mut batch);
            return batch;
        }
        if let Some(reason) = self.loading_reason() {
            self.draw_level_loading(&mut batch, reason);
            self.draw_pack_toasts(&mut batch);
            return batch;
        }
        // Hud.extractRenderState: F1 (keyToggleGui) hides camera overlays, the
        // crosshair, hotbar, effects, bars, sidebar, action bar, titles and chat;
        // the sleep overlay, debug screen and open screens still draw.
        let spectator = self.spectator();
        let (wi, hi) = (w as i32, h as i32);
        let show_hud = !self.ui.hide_gui;
        if show_hud {
            self.draw_spyglass(&mut batch);
            // Hud.extractCrosshair / extractItemHotbar / extractPlayerHealth /
            // extractVehicleHealth and the experience bar, laid out by hud_layout.
            let indicator = self.settings.attack_indicator;
            let delay = horizon_client::attack_strength::delay(self.main_hand_attack_speed());
            let strength = self.hand_strength.scale(0.0, self.main_hand_attack_speed());
            // The 3D crosshair (F3) takes the 2D one's place.
            if self.camera_type.first_person() && !spectator && !self.debug_crosshair() {
                let attack = (indicator == 1).then_some(strength);
                let full = attack.is_some_and(|a| {
                    hud_layout::full_attack_indicator(a, delay, self.crosshair_living())
                });
                let draws = hud_layout::crosshair(wi, hi, attack, full);
                hud_layout::paint(&mut batch, &self.gui.atlas, &draws);
            }
            if !spectator {
                let inputs = hud_layout::HotbarInputs {
                    selected: self.selected as i32,
                    right_handed: !self.settings.main_hand_left,
                    offhand: self.offhand.is_some(),
                    slots: std::array::from_fn(|i| self.inventory[i].as_ref().map(|_| 0.0)),
                    offhand_pop: 0.0,
                    attack: (indicator == 2).then_some(strength),
                };
                let draws = hud_layout::hotbar(wi, hi, &inputs);
                hud_layout::paint(&mut batch, &self.gui.atlas, &draws);
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
            self.draw_extras(&mut batch);
        }
        if show_hud {
            // Hud.nextContextualInfoState: the jump bar, the experience bar or
            // the locator bar above the hotbar.
            let has_experience = !self.creative && !spectator;
            let contextual = horizon_client::locator_bar::ContextualBar::next(
                self.waypoints.has_waypoints(),
                !spectator && self.jumpable_mount(),
                self.mount_jumping(),
                has_experience,
                self.experience_recent(),
            );
            use horizon_client::locator_bar::ContextualBar;
            // The mount's hearts (and the jump bar).
            let vehicle_hearts = if spectator {
                0
            } else {
                self.draw_mount_hud(
                    &mut batch,
                    wi,
                    hi,
                    contextual == ContextualBar::JumpableVehicle,
                )
            };
            if contextual == ContextualBar::Locator {
                let (sprite, [x, y, w, h]) = horizon_client::locator_bar::background(wi, hi);
                horizon_client::ui::nine_slice(
                    &mut batch,
                    &self.gui.atlas,
                    sprite,
                    x as f32,
                    y as f32,
                    w as f32,
                    h as f32,
                    u32::MAX,
                );
            }
            if !self.creative
                && !spectator
                && let Some(p) = &mut self.player
            {
                let atlas = &self.gui.atlas;
                let width = |s: &str| HudBatch::text_width(atlas, s) as i32;
                let draws = hud_layout::experience(
                    wi,
                    hi,
                    self.experience.progress,
                    self.experience.level,
                    contextual == ContextualBar::Experience,
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
                    .facts()
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
                    air: p.air,
                    max_air: 300,
                    under_water,
                    vehicle_hearts,
                };
                let draws = self.health_hud.layout(&inputs, wi, hi);
                hud_layout::paint(&mut batch, atlas, &draws);
            }
            if contextual == ContextualBar::Locator {
                let draws = self.locator_bar_draws(wi, hi);
                hud_layout::paint(&mut batch, &self.gui.atlas, &draws);
            }
            if !spectator {
                self.draw_selected_name(&mut batch, !self.creative);
            }
        }
        // Hud.extractSleepOverlay: over all health/food bars, under titles and chat.
        let draws = hud_overlays::sleep_overlay(wi, hi, self.sleep_timer as i32);
        hud_layout::paint(&mut batch, &self.gui.atlas, &draws);
        if show_hud {
            self.draw_titles(&mut batch);
            self.draw_scoreboard(&mut batch);
            self.draw_subtitles(&mut batch);
        }
        let atlas = &self.gui.atlas;
        if show_hud
            && let Some((text, at)) = &self.message
            && at.elapsed() < Duration::from_secs(3)
        {
            let tw = HudBatch::text_width(atlas, text);
            batch.text(atlas, text, (w - tw) / 2.0, h - 59.0, u32::MAX, true);
        }
        self.draw_chat(&mut batch);
        self.draw_tab_list(&mut batch);
        // Screen.extractBlurredBackground blurs the world and HUD, while the
        // screen's background, widgets, toasts and debug overlay stay sharp.
        batch.blur_at = batch.vertices.len() as u32;
        match self.screen {
            Screen::Paused if self.pause_menu => self.draw_pause_menu(&mut batch),
            // PauseScreen(false): only its title, at the top.
            Screen::Paused => {
                let title = self.translate("menu.paused", "Game Paused");
                let tw = HudBatch::text_width(&self.gui.atlas, &title);
                batch.text(
                    &self.gui.atlas,
                    &title,
                    ((batch.width / 2.0).floor() - (tw / 2.0).floor()).floor(),
                    10.0,
                    u32::MAX,
                    true,
                );
            }
            Screen::GameModeSwitcher => self.draw_gamemode_switcher(&mut batch),
            Screen::Dead => self.draw_death_screen(&mut batch),
            Screen::Stats => self.draw_stats(&mut batch),
            Screen::Advancements => self.draw_advancements(&mut batch),
            Screen::WorldOptions => self.draw_world_options(&mut batch),
            Screen::Friends => self.draw_friends_screen(&mut batch),
            Screen::CommandBlock => self.draw_command_block(&mut batch),
            Screen::Inventory => {
                self.draw_inventory(&mut batch);
            }
            Screen::Creative => {
                self.draw_creative(&mut batch);
            }
            Screen::Chat => self.draw_chat_screen(&mut batch),
            Screen::Playing | Screen::Options | Screen::Win => {}
        }
        self.draw_toasts(&mut batch);
        self.draw_pack_toasts(&mut batch);
        self.remote_hud(&mut batch);
        self.draw_sign_editor(&mut batch);
        self.draw_book(&mut batch);
        batch
    }
    fn creative_inventory_open(&self) -> bool {
        self.screen == Screen::Creative && self.creative_screen.tab().inventory
    }
    /// An item tab of the creative screen (its ItemPickerMenu).
    fn creative_tab_open(&self) -> bool {
        self.screen == Screen::Creative && !self.creative_screen.tab().inventory
    }
    fn inventory_origin(&self, batch: &HudBatch) -> (f32, f32) {
        if self.screen == Screen::Creative {
            return Self::creative_origin(batch);
        }
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
        if self.creative_inventory_open() {
            return Creative::inventory_slots().collect();
        }
        if self.creative_tab_open() {
            return Creative::tab_slots().collect();
        }
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
                | BlockEntity::CreakingHeart(_)
                | BlockEntity::Spawner(_)
                | BlockEntity::TrialSpawner(_)
                | BlockEntity::Vault(_)
                | BlockEntity::Beehive(_)
                | BlockEntity::CommandBlock(_),
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
        if self.creative_inventory_open()
            && let Some(position) = Creative::inventory_slot_position(i)
        {
            return position;
        }
        if self.creative_tab_open()
            && let Some(position) = Creative::tab_slot_position(i)
        {
            return position;
        }
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
            309 if matches!(self.container_kind(), Some(BlockEntity::Crafter(_))) => {
                self.crafter_result()
            }
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
        let index = &self.res.stations().recipe_index;
        let (_, unlocked) = self.level.recipe_book.add_recipes(index, recipes);
        self.recipe_unlocked(&unlocked);
    }
    fn craft_recipe(&self) -> Option<(String, ItemStack)> {
        let n = self.craft_size;
        let grid: Vec<Option<ItemStack>> = (0..n * n).map(|i| self.craft_grid[i].clone()).collect();
        let scale = |id: i32| self.maps.data.get(&id).map(|m| m.scale);
        self.res
            .recipes()
            .find_with(&grid, n, &scale)
            .map(|(name, result)| (name.to_owned(), result))
    }
    fn draw_inventory_slots(&mut self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let drag = self.drag_view();
        for i in self.slots() {
            let (sx, sy) = self.slot_position(i);
            if i == 504 && self.offhand.is_none() {
                batch.sprite(
                    &self.gui.atlas,
                    "gui/sprites/container/slot/shield",
                    ox + sx,
                    oy + sy,
                );
            }
            if (500..504).contains(&i) && self.armor[i - 500].is_none() {
                let icon = ["helmet", "chestplate", "leggings", "boots"][i - 500];
                batch.sprite(
                    &self.gui.atlas,
                    &format!("gui/sprites/container/slot/{icon}"),
                    ox + sx,
                    oy + sy,
                );
            }
            let stack = self.slot_view(i);
            self.draw_slot_item(batch, drag.as_ref(), i, stack, ox + sx, oy + sy);
        }
    }
    fn draw_inventory(&mut self, batch: &mut HudBatch) {
        if self.draw_station(batch) {
            return;
        }
        let (ox, oy) = self.inventory_origin(batch);
        {
            let atlas = &self.gui.atlas;
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
                    | BlockEntity::CreakingHeart(_)
                    | BlockEntity::Spawner(_)
                    | BlockEntity::TrialSpawner(_)
                    | BlockEntity::Vault(_)
                    | BlockEntity::Beehive(_)
                    | BlockEntity::CommandBlock(_),
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
                    batch.text(atlas, title, ox + tx, oy + 6.0, grey, false);
                    if self.craft_size == 3 {
                        batch.text(atlas, "Inventory", ox + 8.0, oy + 72.0, grey, false);
                    }
                }
            }
        }
        if self.container.is_none() && self.craft_size != 3 && !self.mount_active() {
            self.draw_inventory_player(batch, ox, oy);
        }
        self.draw_inventory_slots(batch, ox, oy);
        self.draw_sort_buttons(batch, ox, oy);
        self.draw_recipe_book(batch);
        if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
            let (sx, sy) = self.slot_position(slot);
            batch.fill(
                &self.gui.atlas,
                ox + sx,
                oy + sy,
                16.0,
                16.0,
                rgba(255, 255, 255, 128),
            );
        }
        self.draw_carried(batch);
        self.draw_slot_tooltip(batch);
        if self.container.is_none() && self.craft_size == 2 && !self.mount_active() {
            self.draw_inventory_effects(batch, (ox, oy), 176);
        }
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
        self.craft_table = None;
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
                .facts()
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
        if let Some(slot) = slot
            && slot >= Creative::DISPLAY
            && self.creative_tab_open()
        {
            self.creative_display_click(slot, button, input);
            return true;
        }
        self.menu_ops(&[(slot, button, input)])
    }
    /// A click on an item tab's display slot: the screen's own handling
    /// (`creative::click_display`), never the menu's.
    fn creative_display_click(
        &mut self,
        slot: usize,
        button: i32,
        input: horizon_client::menu::Input,
    ) {
        use horizon_client::creative::{DisplayClick, click_display};
        if self.creative_screen.tab().search {
            // CreativeModeInventoryScreen.slotClicked: moveCursorToEnd, then
            // setHighlightPos(0), so typing replaces the whole query.
            let search = &self.creative_screen.search;
            self.text_selection.select(("creative.search", 0), search);
        }
        let items = self.creative_screen.items(&self.pack.names);
        let first = self.creative_screen.first_row(&self.pack.names);
        let shown = items.get(first * 9 + slot - Creative::DISPLAY);
        let catalog = self.res.items();
        let max = |s: &ItemStack| catalog.get(&s.item).map_or(64, |i| i.max_stack);
        match click_display(&mut self.carried, shown, max, button, input) {
            DisplayClick::Hotbar(i, stack) => self.inventory[i] = Some(stack),
            DisplayClick::Drop(stack) => self.player_drop(stack),
            DisplayClick::Nothing => {}
        }
    }
    /// Several clicks on one menu instance (a quick craft drag is three).
    fn menu_ops(&mut self, ops: &[(Option<usize>, i32, horizon_client::menu::Input)]) -> bool {
        use horizon_client::menu::Input;
        if self.remote.is_some() && self.screen == Screen::Inventory {
            // On a server the clicks are the server's: predicted and sent.
            return self.remote_menu_ops(ops);
        }
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
    /// A sound a menu click made the player play (a bundle's insert or
    /// remove): `Entity.playSound` at the player.
    pub(super) fn player_menu_sound(&mut self, sound: &str) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let at = [p.position.x, p.position.y, p.position.z];
        let (volume, pitch) = if sound == horizon_client::bundle::INSERT_FAIL {
            (1.0, 1.0)
        } else {
            (0.8, 0.8 + self.world.random.next_float() * 0.4)
        };
        self.audio.play(sound, Some(at), volume, pitch);
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
        let stonecutter_open = self
            .station
            .as_ref()
            .is_some_and(|s| s.kind == stations::Kind::Stonecutter);
        let station_kind = self.station.as_ref().map(|s| s.kind);
        let grindstone_open = station_kind == Some(stations::Kind::Grindstone);
        let cartography_open = station_kind == Some(stations::Kind::Cartography);
        let anvil_open = station_kind == Some(stations::Kind::Anvil);
        let smithing_open = station_kind == Some(stations::Kind::Smithing);
        let enchanting_open = station_kind == Some(stations::Kind::Enchanting);
        let beacon_open = station_kind == Some(stations::Kind::Beacon);
        let (kind, game_of): (Kind, Vec<Option<usize>>) = if self.creative_tab_open() {
            // ItemPickerMenu: the display slots (clicked by the screen), then the hotbar.
            let mut v = vec![None; 45];
            v.extend((0..9).map(Some));
            (Kind::ItemPicker, v)
        } else if let Some(columns) = self
            .mount_menu_columns()
            .filter(|_| self.remote.is_some() && self.mount_active())
        {
            // A server's HorseInventoryMenu: the saddle, the body armour,
            // the chest, then the inventory.
            let mut v = vec![Some(mount_menu::SADDLE), Some(mount_menu::ARMOR)];
            v.extend((0..columns * 3).map(|i| Some(mount_menu::CHEST + i)));
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Mount(columns), v)
        } else if merchant_open {
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
        } else if stonecutter_open {
            // StonecutterMenu: the input, the result, then the inventory.
            let mut v = vec![Some(stations::INPUT), Some(stations::RESULT)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Stonecutter, v)
        } else if grindstone_open || cartography_open {
            // GrindstoneMenu and CartographyTableMenu: the two inputs, the
            // result, then the inventory.
            let mut v = vec![
                Some(stations::INPUT),
                Some(stations::INPUT + 1),
                Some(stations::RESULT),
            ];
            v.extend((9..36).chain(0..9).map(Some));
            let kind = if grindstone_open {
                Kind::Grindstone
            } else {
                Kind::Cartography
            };
            (kind, v)
        } else if enchanting_open {
            // EnchantmentMenu: the item, the lapis, then the inventory.
            let mut v = vec![Some(stations::INPUT), Some(stations::INPUT + 1)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Enchanting, v)
        } else if beacon_open {
            // BeaconMenu: the payment, then the inventory.
            let mut v = vec![Some(stations::INPUT)];
            v.extend((9..36).chain(0..9).map(Some));
            (Kind::Beacon, v)
        } else if anvil_open || smithing_open {
            // ItemCombinerMenu: the inputs, the result, then the inventory.
            let inputs = if anvil_open { 2 } else { 3 };
            let mut v: Vec<Option<usize>> =
                (0..inputs).map(|i| Some(stations::INPUT + i)).collect();
            v.push(Some(stations::RESULT));
            v.extend((9..36).chain(0..9).map(Some));
            (
                if anvil_open {
                    Kind::Anvil
                } else {
                    Kind::Smithing
                },
                v,
            )
        } else {
            let (kind, map) = if self.container.is_some() {
                let size = self.container_size();
                let (kind, count) = match self.container_kind() {
                    Some(BlockEntity::Chest(_)) => {
                        let shulker = self.container.is_some_and(|k| {
                            let id = self.world.block(k.0, k.1, k.2);
                            storage::is_shulker_box(&self.res.facts().get(id).name)
                        });
                        let kind = match size {
                            5 => Kind::Hopper,
                            9 => Kind::Dispenser,
                            27 if shulker => Kind::Shulker,
                            n => Kind::Chest((n / 9).clamp(1, 6)),
                        };
                        (kind, size)
                    }
                    Some(BlockEntity::Brewing(_)) => (Kind::Brewing, 5),
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
            } else if self.station.is_none() && !self.mount_active() {
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
        if matches!(kind, Kind::Mount(_)) {
            menu.mount = self.mount_menu_entity();
        }
        let total = menu.slot_count();
        let before: Vec<Option<ItemStack>> = game_of
            .iter()
            .map(|g| g.and_then(|g| self.slot_mut(g).clone()))
            .collect();
        let carried = self.carried.clone();
        let creative = self.creative;
        // CreativeModeInventoryScreen clicks the menu itself and sends every
        // changed slot, which the server sets by player.
        let equipment_before = (self.screen == Screen::Creative).then(|| self.player_equipment());
        let station_menu = self.station.as_ref().map(|s| {
            (
                s.loom.selected.map_or(-1, |i| i as i32),
                s.loom.patterns.clone(),
            )
        });
        let stone = self
            .station
            .as_ref()
            .map(|s| (s.selected.map_or(-1, |i| i as i32), s.stone.clone()));
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
        // The cartography table reads and makes the level's maps; a
        // server's (the client menu's ContainerLevelAccess.NULL) doesn't.
        let level_maps = (kind == Kind::Cartography && self.remote.is_none()).then(|| {
            let held: Vec<ItemStack> = before
                .iter()
                .flatten()
                .chain(self.inventory.iter().flatten())
                .chain(self.carried.iter())
                .cloned()
                .collect();
            self.level_maps(&held)
        });
        let next_map = if merchant_open { self.peek_map_id() } else { 0 };
        let tag = |t: &str| self.structure_tag(t);
        let maps = trading::GameMaps {
            terrain: self.end.locator(),
            structure_tag: &tag,
            made: Default::default(),
            next_id: std::cell::Cell::new(next_map),
        };
        let trade_env = horizon_client::merchant::TradeEnv {
            trades: &self.res.stations().trades,
            enchantments: &self.res.stations().enchantments,
            maps: Some(&maps),
        };
        let env = Env {
            items: self.res.items(),
            recipes: self.res.recipes(),
            brewing: &self.res.stations().brewing,
            banners: &self.res.stations().banners,
            station_recipes: &self.res.stations().recipes,
            enchantments: &self.res.stations().enchantments,
            names: &self.pack.names,
            trade: Some(&trade_env),
            maps: level_maps
                .as_ref()
                .map(|m| m as &dyn horizon_client::menu::Maps),
        };
        menu.creative = creative;
        menu.selected = self.selected;
        menu.levels = self.experience.level;
        if anvil_open {
            // The rename box as AnvilScreen.onNameChanged gives it to the menu.
            menu.anvil.name = self.station.as_ref().map(|s| self.anvil_item_name(s));
        }
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
        if kind == Kind::Stonecutter
            && let Some((selected, state)) = stone
        {
            // The same: the list and selection between clicks, then the result.
            menu.data[0] = selected;
            menu.stone = state;
            menu.set_slot(&env, 1, before[1].clone());
        }
        if kind == Kind::Enchanting
            && let Some(s) = self.station.as_ref()
        {
            // The offers between clicks (what copying the slots in flagged is not a change).
            menu.enchant_changed = false;
            for i in 0..3 {
                menu.data[i] = s.offers.costs[i];
                menu.data[7 + i] = s.offers.clues[i].as_ref().map_or(-1, |(_, l)| *l as i32);
            }
            menu.data[3] = self.level.xp_seed;
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
        // On a server the clicks only predict: the server drops, crafts and
        // makes maps itself.
        let predicting = self.remote.is_some();
        if !predicting {
            self.register_exploration_maps(made);
        }
        if let Some(s) = menu.merchant.take() {
            self.merchant_sync_out(s);
        }
        let after: Vec<Option<ItemStack>> = (0..total).map(|i| menu.slot(i).cloned()).collect();
        let carried = menu.carried.take();
        let dropped = std::mem::take(&mut menu.dropped);
        let sounds = std::mem::take(&mut menu.sounds);
        // A bundle's sounds are the player's own (`Entity.playSound`): the
        // client plays them as it clicks, predicting or not.
        for sound in std::mem::take(&mut menu.player_sounds) {
            self.player_menu_sound(sound);
        }
        let crafted = std::mem::take(&mut menu.crafted);
        let equipped = std::mem::take(&mut menu.equipped);
        for (i, stack) in after.into_iter().enumerate() {
            if let Some(g) = game_of[i] {
                *self.slot_mut(g) = stack;
            }
        }
        self.carried = carried;
        if !offhand_mapped {
            self.offhand = menu.player_item(40).cloned();
        }
        if !predicting {
            for stack in dropped {
                self.player_drop(stack);
            }
            self.award_crafted(&crafted);
        }
        if kind == Kind::Loom {
            if let Some(s) = self.station.as_mut() {
                s.loom.selected = usize::try_from(menu.data[0]).ok();
                s.loom.patterns = std::mem::take(&mut menu.loom_patterns);
            }
            // A server's loom plays its own sounds (the client's menu has no level).
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
            self.loom_screen_changed();
        } else if kind == Kind::Cartography {
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
        } else if kind == Kind::Stonecutter {
            if let Some(s) = self.station.as_mut() {
                s.selected = usize::try_from(menu.data[0]).ok();
                s.stone = std::mem::take(&mut menu.stone);
                s.stonecutter_changed();
            }
            for sound in sounds.into_iter().filter(|_| !predicting) {
                self.station_sound(sound);
            }
        }
        if let Some(amount) = menu.ground.take().filter(|_| !predicting)
            && let Some(pos) = self.station.as_ref().map(|s| s.pos)
        {
            self.grindstone_taken(pos, amount);
        }
        if kind == Kind::Anvil || kind == Kind::Smithing {
            self.combiner_session_end(&menu, predicting);
        }
        if kind == Kind::Enchanting && (menu.enchant_changed || before[0] != menu.slot(0).cloned())
        {
            // EnchantmentMenu.slotsChanged: new offers for the item.
            self.station_changed();
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
            f.input_clicked(self.res.recipes(), before[0].clone());
        }
        match equipment_before {
            Some(before) => self.player_equipment_changed(&before),
            None => {
                for (slot, stack) in equipped {
                    self.player_equipped(slot, stack.as_ref());
                }
            }
        }
        self.apply_equipment();
        if let Some(maps) = level_maps {
            self.register_made_maps(maps);
        }
        true
    }
    /// `CreativeModeInventoryScreen.keyPressed` on the search tab: a hotbar
    /// key over a slot swaps before the search box sees it.
    fn search_tab_hotbar_key(&mut self, code: KeyCode) -> bool {
        use horizon_client::creative::{Hover, search_key_swaps};
        let hotbar_key = HOTBAR_DIGITS.contains(&code);
        let numeric = hotbar_key || NUMERIC_KEYS.contains(&code);
        if !numeric {
            return false;
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return false;
        };
        let batch = HudBatch::new(size);
        let hover = match self.slot_at(&batch, self.cursor.0, self.cursor.1) {
            None => Hover::Nothing,
            Some(slot) if slot >= Creative::DISPLAY => {
                let first = self.creative_screen.first_row(&self.pack.names);
                let shown = self.creative_screen.items(&self.pack.names).len();
                Hover::Display {
                    filled: first * 9 + slot - Creative::DISPLAY < shown,
                }
            }
            Some(_) => Hover::Slot,
        };
        search_key_swaps(numeric, hotbar_key, hover, self.carried.is_some())
            && self.inventory_hotkey(code)
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
        let digit = HOTBAR_DIGITS.iter().position(|k| *k == key);
        let done = match (digit, key) {
            (Some(n), _) => self.menu_click(Some(slot), n as i32, Input::Swap),
            (None, KeyCode::KeyF) => self.menu_click(Some(slot), 40, Input::Swap),
            (None, KeyCode::KeyQ) => {
                let all = drop_whole_stack(&self.physical_keys);
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
        if !right && self.sort_press() {
            return;
        }
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
        if self.tweak_press(slot, right, shift) {
            return;
        }
        if let Some(slot) = slot
            && shift
            && slot != 200
        {
            let now = std::time::Instant::now();
            let double = self.last_inv_click.is_some_and(|(at, last, was_right)| {
                last == slot && was_right == right && now.duration_since(at).as_millis() < 250
            });
            self.last_inv_click = Some((now, slot, right));
            // mouseClicked's QUICK_MOVE (and Horizon's at-press QUICK_MOVE
            // of a carried stack, mouseReleased's in vanilla) remember the
            // stack it moves.
            if input == Some(Input::QuickMove) || (self.carried.is_some() && !double) {
                self.last_quick_moved = self.slot_view(slot);
            }
            if double && !right {
                // mouseReleased: every stack of lastQuickMoved's item in the
                // clicked slot's container is quick-moved.
                if let Some(input) = input
                    && !self.menu_click(Some(slot), 0, input)
                {
                    // A screen the menu engine doesn't model: its own click.
                    self.click_inventory(right);
                    return;
                }
                if let Some(moved) = self.last_quick_moved.clone() {
                    if self.remote.is_some() {
                        self.remote_quick_move_all(slot, &moved);
                    } else {
                        self.menu_session(|menu, env, to_menu| {
                            Some(menu.quick_move_all(env, to_menu(slot)?, &moved).is_ok())
                        });
                    }
                }
                self.last_inv_click = None;
                self.inv_clicks.skip_release();
                return;
            }
        }
        if let Some(slot) = slot
            && !shift
            && slot != 200
        {
            // An item tab's display slots take no drags or PICKUP_ALL.
            let display = slot >= Creative::DISPLAY;
            let now = std::time::Instant::now();
            let double = !right
                && !display
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
                let ty = i32::from(right);
                let first = if !display && self.quick_craft_accepts(slot, ty, 0) {
                    vec![slot]
                } else {
                    Vec::new()
                };
                self.inv_drag = Some((ty, first));
                return;
            }
        }
        if self.creative_tab_open() {
            // CreativeModeInventoryScreen goes through the same mouseClicked:
            // a click now, or a drag that its release ends.
            match input {
                Some(input) => {
                    self.menu_click(slot, i32::from(right), input);
                }
                None if self.inv_clicks.quick_crafting().is_some() => {
                    self.inv_drag = Some((i32::from(right), Vec::new()));
                }
                None => {}
            }
            return;
        }
        // mouseClicked: an empty hand's PICKUP is the menu's click at once
        // (a right click on a bundle takes its first stack out).
        if let (Some(input), Some(_)) = (input, slot)
            && self.remote.is_none()
            && self.menu_click(slot, i32::from(right), input)
        {
            if let Some(key) = self.container {
                self.container_changed(key);
            }
            return;
        }
        self.click_inventory(right);
    }
    /// Mouse movement while a quick craft drag is running.
    fn inventory_drag_move(&mut self) {
        if self.tweaks_on() && self.tweak_drag_move(self.inventory_target().0) {
            return;
        }
        let Some((ty, slots)) = self.inv_drag.clone() else {
            return;
        };
        let Some(r) = self.renderer.as_ref() else {
            return;
        };
        let batch = HudBatch::new([r.config.width, r.config.height]);
        if let Some(slot) = self.slot_at(&batch, self.cursor.0, self.cursor.1)
            && slot != 200
            && slot < Creative::DISPLAY
            && !slots.contains(&slot)
            && self.quick_craft_accepts(slot, ty, slots.len())
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
    /// `Minecraft.hasControlDown`: the physical Control keys, whatever the
    /// sprint key is bound or toggled to.
    pub(super) fn has_control_down(&self) -> bool {
        self.physical_keys.contains(&KeyCode::ControlLeft)
            || self.physical_keys.contains(&KeyCode::ControlRight)
    }
    /// `hasClickedOutside` of the open screen at the cursor.
    fn clicked_outside(&self, batch: &HudBatch) -> bool {
        use horizon_client::container_screen::{Book, Image, clicked_outside};
        let (ox, oy) = self.inventory_origin(batch);
        let (shift, width, height) = if self.screen == Screen::Creative {
            (0.0, Creative::WIDTH as i32, Creative::HEIGHT as i32)
        } else {
            let (shift, width) = match self.station.as_ref().map(|s| s.kind) {
                Some(stations::Kind::Merchant) => (-50.0, 276),
                Some(stations::Kind::Beacon) => (beacon::SHIFT, 230),
                _ => (0.0, 176),
            };
            (shift, width, self.screen_height() as i32)
        };
        let image = Image {
            left: (ox + shift) as i32,
            top: oy as i32,
            width,
            height,
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
        self.tweak_release();
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
    /// A press of the pick block button (`keyPickItem`, the middle button by
    /// default) over an inventory screen: `mouseClicked` with `cloning`
    /// (infinite materials). An empty hand clones the hovered stack at its
    /// full size (`CLONE`), a carried stack starts a quick craft drag of type
    /// 2 that fills every slot it crosses; without infinite materials the
    /// press does nothing.
    fn inventory_pick_press(&mut self) {
        use horizon_client::container_screen::{Button, Target};
        let (slot, outside) = self.inventory_target();
        let was_crafting = self.inv_clicks.quick_crafting().is_some();
        let input = self.inv_clicks.press(
            &Target::of(slot, outside),
            Button::Middle,
            self.carried.is_none(),
            self.has_shift_down(),
            self.creative,
        );
        match (input, slot) {
            // Slot -999 (outside the window) clones nothing.
            (Some(input), Some(slot)) if !outside => {
                self.menu_click(Some(slot), 2, input);
            }
            (None, _) if !was_crafting && self.inv_clicks.quick_crafting().is_some() => {
                // Item tab display slots take no drags.
                let first = slot.filter(|&s| {
                    s != 200 && s < Creative::DISPLAY && self.quick_craft_accepts(s, 2, 0)
                });
                self.inv_drag = Some((2, first.into_iter().collect()));
            }
            _ => {}
        }
    }
    /// The pick block button's release (`mouseReleased`): ends its quick
    /// craft drag, cancels another button's, or clones onto the slot under a
    /// carried stack (`CLONE`, which only an item tab's display slot takes).
    fn inventory_pick_release(&mut self) {
        use horizon_client::container_screen::{Button, Release, Target};
        let (slot, outside) = self.inventory_target();
        let dragged = self.inv_drag.as_ref().is_some_and(|(_, s)| !s.is_empty());
        let release = self.inv_clicks.release(
            &Target::of(slot, outside),
            Button::Middle,
            self.carried.is_none(),
            dragged,
            self.has_shift_down(),
            true,
        );
        match (release, slot) {
            (Release::QuickCraft, _) => self.inventory_release(),
            (Release::Click(input), Some(slot)) if !outside => {
                self.inv_drag = None;
                self.menu_click(Some(slot), 2, input);
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
                        && c.count + result.count <= max(self.res.items(), c) =>
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
                let remainders = self.res.recipes().remainders(
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
                                && c.count + result.count <= max(self.res.items(), c) =>
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
                    .items()
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
        let items = self.res.items();
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
            f.input_clicked(self.res.recipes(), before);
        }
    }
    /// `Camera.setRotation`: the camera's orientation, turned round in
    /// third-person front view.
    pub(super) fn camera_orientation(&self) -> glam::Quat {
        let (view_yaw, view_pitch) =
            if self.camera_type == perspective::CameraType::ThirdPersonFront {
                (self.yaw + 180.0, -self.pitch)
            } else {
                (self.yaw, self.pitch)
            };
        glam::Quat::from_euler(
            glam::EulerRot::YXZ,
            std::f32::consts::PI - view_yaw.to_radians(),
            -view_pitch.to_radians(),
            0.0,
        )
    }
    /// ItemEntityRenderer: items bob and spin; blocks render as small cubes.
    fn entity_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
        mut out: Vec<Vertex>,
    ) -> Vec<Vertex> {
        self.prepare_entity_culling(camera);
        self.prepare_trims();
        let fire_yaw = if self.camera_type == perspective::CameraType::ThirdPersonFront {
            self.yaw + 180.0
        } else {
            self.yaw
        };
        for m in &self.mobs {
            if m.fire_ticks > 0 && !m.removed {
                let at = m.previous.lerp(m.position, alpha);
                horizon_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    m.width(),
                    m.height(),
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        if !self.camera_type.first_person()
            && let Some(p) = &self.player
            && p.fire_ticks > 0
            && !p.spectator
        {
            let feet = self
                .previous_position
                .lerp(p.position, self.clock.camera_fraction());
            horizon_client::fire_render::flames(
                [feet.x, feet.y, feet.z],
                p.dimensions.width,
                p.dimensions.height,
                camera,
                fire_yaw,
                &self.gui.atlas.sprites,
                self.world.light(
                    feet.x.floor() as i32,
                    feet.y.floor() as i32,
                    feet.z.floor() as i32,
                ),
                &mut out,
            );
        }
        for item in &self.dropped {
            if item.fire_ticks > 0
                && !self
                    .res
                    .items()
                    .resists_fire(&item.stack, "minecraft:in_fire")
            {
                let at = item.previous.lerp(item.position, alpha);
                horizon_client::fire_render::flames(
                    [at.x, at.y, at.z],
                    0.25,
                    0.25,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        at.x.floor() as i32,
                        at.y.floor() as i32,
                        at.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        for stand in &self.armor_stands {
            if stand.fire_ticks > 0 && !stand.marker {
                let scale = if stand.small { 0.5 } else { 1.0 };
                horizon_client::fire_render::flames(
                    [stand.position.x, stand.position.y, stand.position.z],
                    0.5 * scale,
                    1.975 * scale,
                    camera,
                    fire_yaw,
                    &self.gui.atlas.sprites,
                    self.world.light(
                        stand.position.x.floor() as i32,
                        stand.position.y.floor() as i32,
                        stand.position.z.floor() as i32,
                    ),
                    &mut out,
                );
            }
        }
        // `ItemEntityRenderer`.
        self.dropped_item_geometry(camera, alpha, &mut out);
        // `VaultRenderer`.
        self.vault_geometry(camera, alpha, &mut out);
        self.ominous_item_spawner_geometry(camera, alpha, &mut out);
        // Mobs and their projectiles.
        self.mob_geometry(camera, alpha, &mut out);
        self.player_model_geometry(camera, self.clock.camera_fraction(), &mut out);
        self.guest_model_geometry(camera, alpha, &mut out);
        self.fangs_geometry(camera, alpha, &mut out);
        let mut see_through = std::mem::take(&mut self.see_through_vertices);
        see_through.clear();
        self.name_tag_geometry(camera, alpha, &mut out, &mut see_through);
        self.text_display_geometry(camera, alpha, &mut out, &mut see_through);
        self.see_through_vertices = see_through;
        // Entity shadows (EntityRenderer.extractShadow).
        if let Some(sprite) = self.gui.atlas.sprites.get("misc/shadow").copied() {
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
                    self.res.facts(),
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
            let scene = horizon_client::block_entity_render::Scene {
                camera,
                orientation: self.camera_orientation(),
                anim: &anim,
                texts: None,
                partial: alpha as f32,
                game_time: self.world.game_time,
                max_y: self.dim.max_y(),
                world: &self.world,
                sprites: &self.gui.atlas.sprites,
                nether: self.dim == Dim::Nether,
                lid: &lid,
                sign: &|p| match self.world.block_entities.get(&(p[0], p[1], p[2])) {
                    Some(BlockEntity::Sign(sign)) => Some(sign),
                    _ => None,
                },
                banner: &|p| self.banner_layers_at(p),
                glyph: &|c| self.gui.atlas.glyph_cell(c),
            };
            // Block entities outside the view frustum are skipped (with a
            // block of margin for models larger than their block).
            let view = self.view_frustum;
            // Beams and structure outlines reach far outside their block
            // (BlockEntityRenderer.shouldRenderOffScreen): never culled.
            let off_screen = |state: u32| {
                let name = &*self.res.facts().get(BlockStateId(state)).name;
                matches!(
                    name,
                    "minecraft:beacon"
                        | "minecraft:end_gateway"
                        | "minecraft:structure_block"
                        | "minecraft:test_instance_block"
                )
            };
            let visible = r.block_entities().filter(|(p, state)| {
                if !self.res.block_entity_render().renders(*state) {
                    return false;
                }
                off_screen(*state)
                    || view.is_none_or(|v| {
                        let o =
                            (glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
                                - camera)
                                .as_vec3();
                        v.intersects(o - glam::Vec3::ONE, o + glam::Vec3::splat(2.0))
                    })
            });
            if self.entity_culler.is_some() {
                for block in visible {
                    let start = out.len();
                    self.res
                        .block_entity_render()
                        .render(&scene, std::iter::once(block), &mut out);
                    if !off_screen(block.1) {
                        self.cull_entity_vertices(&mut out, start);
                    }
                }
            } else {
                self.res
                    .block_entity_render()
                    .render(&scene, visible, &mut out);
            }
            for (pos, stage) in self.cracks(camera) {
                let state = self.world.block(pos[0], pos[1], pos[2]);
                self.res
                    .block_entity_render()
                    .render_breaking(&scene, pos, state.0, stage, &mut out);
            }
        }
        // Experience orbs (ExperienceOrbRenderer): pulsing billboards from a 4x4 icon sheet.
        if let Some(r) = self
            .gui
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
            let live = self.orbs.iter().map(|o| {
                let at = [
                    o.previous.x + (o.position.x - o.previous.x) * alpha,
                    o.previous.y + (o.position.y - o.previous.y) * alpha,
                    o.previous.z + (o.position.z - o.previous.z) * alpha,
                ];
                (o, at, alpha as f32)
            });
            // ItemPickupParticle: the render state extracted at partial tick 1.
            let taken = self.pickups.live.iter().filter_map(|p| match &p.taken {
                pickups::Taken::Orb(o) => Some((&**o, pickups::position(p, alpha), 1.0)),
                _ => None,
            });
            for (o, at, partial) in live.chain(taken) {
                let p = glam::DVec3::new(at[0], at[1] + 0.1, at[2]) - camera;
                let icon = o.icon();
                let (du, dv) = ((r.u1 - r.u0) / 4.0, (r.v1 - r.v0) / 4.0);
                let (u0, v0) = (r.u0 + (icon % 4) as f32 * du, r.v0 + (icon / 4) as f32 * dv);
                let t = (o.age as f32 + partial) / 2.0;
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
        // Falling blocks use their baked model, including non-cubic shapes.
        for f in &self.falling {
            let Some((_, tint)) = self.block_face_sprite(f.state) else {
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
            let start = out.len();
            self.block_model_vertices(
                f.state,
                horizon_core::random::position_seed(f.start_pos),
                glam::Mat4::from_translation(o),
                packed,
                false,
                &mut out,
            );
            let tint = horizon_client::mobs::render::abgr(tint);
            for vertex in &mut out[start..] {
                vertex.color = horizon_client::hud_layout::multiply(vertex.color, tint);
            }
        }
        self.moving_piston_geometry(camera, alpha, &mut out);
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
        self.thrown_potion_geometry(camera, alpha, &mut out);
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
        // Breaking progress overlays (destroy_stage_0..9): ours and the other players'.
        for (pos, stage) in self.cracks(camera) {
            let o = (glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                - camera)
                .as_vec3();
            let state = self.world.block(pos[0], pos[1], pos[2]);
            if self.res.facts().get(state).render_shape
                == horizon_client::blocks::RenderShape::Model
                && let Ok(models) = self
                    .pack
                    .render
                    .model_catalog
                    .collect(state, BlockPos::new(pos[0], pos[1], pos[2]))
            {
                let offset = self
                    .pack
                    .context
                    .tables
                    .offset(state, pos[0], pos[2])
                    .unwrap_or([0.0; 3]);
                let origin = o + glam::DVec3::from(offset).as_vec3();
                let translucent = models
                    .iter()
                    .any(|(_, model)| model.faces.iter().any(|f| f.translucent));
                for (_, model) in models {
                    horizon_client::breaking::geometry(model, origin, translucent, stage, &mut out);
                }
            }
        }
        out
    }

    /// `LevelRenderer.destroyBlockProgress`: a player's crack on a block (0..=9), or none
    /// (any other progress). Cracks not updated for 400 ticks are dropped.
    pub(crate) fn destroy_block_progress(&mut self, breaker: i32, pos: [i32; 3], progress: i32) {
        let now = self.world.game_time;
        self.destroying
            .retain(|_, c| now.saturating_sub(c.2) <= 400);
        if (0..10).contains(&progress) {
            self.destroying.insert(breaker, (pos, progress as u8, now));
        } else {
            self.destroying.remove(&breaker);
        }
    }

    /// The cracks to draw and their stage sprite's atlas origin: our own mining and the
    /// other players' (`renderBlockDestroyAnimation`: within 32 blocks of the camera, the
    /// furthest progress on each block).
    fn cracks(&self, camera: glam::DVec3) -> Vec<([i32; 3], [f32; 2])> {
        let mut stages: Vec<([i32; 3], u32)> = Vec::new();
        let mut add = |pos: [i32; 3], stage: u32| match stages.iter_mut().find(|c| c.0 == pos) {
            Some(c) => c.1 = c.1.max(stage),
            None => stages.push((pos, stage)),
        };
        if let Some(m) = &self.mining
            && m.progress > 0.0
        {
            add(m.pos, ((m.progress * 10.0) as u32).min(9));
        }
        let now = self.world.game_time;
        for (pos, stage, at) in self.destroying.values() {
            if now.saturating_sub(*at) <= 400 {
                add(*pos, u32::from(*stage));
            }
        }
        stages
            .into_iter()
            .filter(|(pos, _)| {
                let d = glam::DVec3::new(f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]))
                    + 0.5
                    - camera;
                d.length_squared() <= 1024.0
            })
            .filter_map(|(pos, stage)| {
                let r = self
                    .gui
                    .atlas
                    .sprites
                    .get(&format!("block/destroy_stage_{stage}"))?;
                Some((pos, [r.u0, r.v0]))
            })
            .collect()
    }

    fn sky_darken(&self) -> i32 {
        if self.dim == Dim::Nether {
            return 11;
        }
        if self.dim == Dim::End {
            let sky = self
                .res
                .end_environment()
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
        let mut sky = self
            .res
            .environment()
            .sample(self.world.day_time, None, None);
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
            if let Some(t) = ray_box(eye, dir, reach, m.bounds())
                && best.is_none_or(|b| t < b.1)
            {
                best = Some((i, t));
            }
        }
        best
    }
    /// `Player.attack`'s damage against `target_kind` (with the attack
    /// strength and whether it is a critical hit): the weapon and effects
    /// scaled by the attack strength, a mace's smash, the critical hit, then
    /// enchantments.
    pub(super) fn melee_damage(&mut self, target_kind: &str) -> (f32, f32, bool) {
        let hand_damage = 1.0 + self.attack_item_info().map_or(0.0, |i| i.attack_damage);
        // Riptide's spin replaces the attack damage attribute.
        let spin = self.player.as_ref().map_or(0.0, |p| p.auto_spin_attack_dmg);
        let damage = if spin > 0.0 {
            spin
        } else {
            hand_damage + self.effect_attack_bonus()
        };
        let scale = self.attack_strength_scale();
        let mut amount = damage * (0.2 + scale * scale * 0.8);
        // Item.getAttackDamageBonus (a mace's smash), before the critical hit.
        amount += self.smash_damage_bonus(target_kind);
        let player = self.player.as_ref();
        // Player.canCriticalAttack: falling, off the ground, not in water,
        // not blind (isMobilityRestricted), not riding, not sprinting.
        let falling = player.is_some_and(|p| {
            !p.ground.on_ground
                && p.fall_distance > 0.0
                && !p.fluids.was_touching_water
                && !p.sprinting
        });
        let restricted = self.potions.effects.get("blindness").is_some() || self.riding.is_some();
        let crit = scale > 0.9 && falling && !restricted;
        if crit {
            amount *= 1.5;
        }
        // Enchantment damage (sharpness, smite, bane) scaled by the attack strength.
        amount += self.enchant_damage_bonus(target_kind) * scale;
        (amount, scale, crit)
    }
    /// `Player.getAttackStrengthScale(0.5)`.
    fn attack_strength_scale(&self) -> f32 {
        let speed = self.attack_item_info().map_or(0.0, |i| i.attack_speed);
        let delay = 20.0 / (4.0 + speed).max(0.1);
        ((self.attack_ticks as f32 + 0.5) / delay).clamp(0.0, 1.0)
    }
    /// `Player.isSweepAttack`: a charged, grounded sword hit below the movement limit.
    pub(super) fn sweep_attack_position(&self, scale: f32, crit: bool) -> Option<[f64; 3]> {
        let p = self.player.as_ref()?;
        let sword = self.held().is_some_and(|s| {
            self.res
                .items()
                .item_tags
                .get("minecraft:swords")
                .is_some_and(|tag| tag.contains(&*s.item))
        });
        if !p.is_sweep_attack(scale, crit, sword) {
            return None;
        }
        let yaw = f64::from(self.yaw.to_radians());
        Some([
            p.position.x - yaw.sin(),
            p.position.y + f64::from(p.dimensions.height) * 0.5,
            p.position.z + yaw.cos(),
        ])
    }
    /// The visual and sound part of `Player.doSweepAttack` after a successful hit.
    pub(super) fn sweep_attack(&mut self, at: [f64; 3], from: Option<[f64; 3]>) {
        self.particles.add(
            &horizon_client::particles::ParticleOptions::simple(
                horizon_client::particles::ParticleType::SweepAttack,
            ),
            at,
            [0.0; 3],
        );
        self.audio
            .play("minecraft:entity.player.attack.sweep", from, 1.0, 1.0);
    }
    /// `Player.attack`'s sounds when a vehicle takes the hit (it always does,
    /// and can't be hit critically): knockback when sprinting at full
    /// strength, then strong or weak. Before `swing_arm` resets the charge.
    pub(super) fn vehicle_attack_sounds(&mut self) {
        let full = self.attack_strength_scale() > 0.9;
        let player = self.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        if full && player.is_some_and(|p| p.sprinting) {
            self.audio
                .play("minecraft:entity.player.attack.knockback", from, 1.0, 1.0);
        }
        let sound = if full {
            "minecraft:entity.player.attack.strong"
        } else {
            "minecraft:entity.player.attack.weak"
        };
        if let Some(at) = self.sweep_attack_position(self.attack_strength_scale(), false) {
            self.sweep_attack(at, from);
        } else {
            self.audio.play(sound, from, 1.0, 1.0);
        }
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
        // A ghast's fireball or a wind charge in front: turned back.
        if self.attack_redirectable() {
            return true;
        }
        let hand_damage = 1.0
            + self
                .held()
                .and_then(|h| self.res.items().get(&h.item))
                .map_or(0.0, |i| i.attack_damage);
        if self.attack_frame()
            || self.attack_armor_stand()
            || self.attack_boat(hand_damage)
            || self.attack_cart(hand_damage)
            || self.attack_dragon()
            || self.attack_guest()
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
        self.attack_mob(index);
        // Minecraft.startAttack swings after the attack.
        self.swing_arm();
        true
    }
    /// `Player.attack` on a mob: the damage, knockback, sweep, particles and
    /// sounds.
    pub(super) fn attack_mob(&mut self, index: usize) {
        let target_kind = self.mobs[index].kind.id.clone();
        let target_kind = format!("minecraft:{}", target_kind.trim_start_matches("minecraft:"));
        let (amount, scale, crit) = self.melee_damage(&target_kind);
        let sweep = self.sweep_attack_position(scale, crit);
        let (knockback, knockback_attack) = self.extra_knockback(&target_kind, scale);
        self.attack_ticks = 0;
        if let Some(p) = self.player.as_mut()
            && !self.creative
        {
            p.cause_food_exhaustion(0.1);
        }
        let player = self.player.as_ref();
        let from = player.map(|p| [p.position.x, p.position.y, p.position.z]);
        if knockback_attack {
            self.audio
                .play("minecraft:entity.player.attack.knockback", from, 1.0, 1.0);
        }
        // LivingEntity.setLastHurtMob (tamed wolves join in).
        self.combat.last_hurt_mob = Some((self.mobs[index].id, self.world.game_time));
        let old_health = self.mobs[index].health;
        let source = match from {
            Some(f) => self.attack_source(f),
            None => horizon_client::mobs::Damage::of(horizon_client::mobs::DamageKind::Generic),
        };
        let landed = self.hurt_by_player(index, amount, source);
        if let Some(p) = self.player.as_mut() {
            if p.attack_log.len() >= 64 {
                p.attack_log.remove(0);
            }
            p.attack_log.push(horizon_client::health::AttackRecord {
                game_time: p.health.game_time(),
                target: target_kind.clone(),
                id: self.mobs[index].id,
                killed: landed && !self.mobs[index].alive(),
                amount,
                scale,
                crit,
                landed,
                dealt: old_health - self.mobs[index].health,
            });
        }
        if landed {
            if knockback > 0.0 {
                let (xd, zd) = self.extra_knockback_direction();
                self.mobs[index].knockback(f64::from(knockback), xd, zd);
                self.stop_sprint_after_extra_knockback();
            }
            let mob = &self.mobs[index];
            // Player.damageStatsAndHearts: the damage the hit actually did.
            let actual = old_health - mob.health;
            // Player.attack: crit emitter and damage indicator hearts.
            let (feet, width, height) = (
                [mob.position.x, mob.position.y, mob.position.z],
                mob.kind.width,
                mob.kind.height,
            );
            self.stats
                .custom("damage_dealt", (actual * 10.0).round() as i32);
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
            if actual > 2.0 {
                let indicator = horizon_client::particles::ParticleOptions::simple(
                    horizon_client::particles::ParticleType::DamageIndicator,
                );
                self.particles.send_particles(
                    &indicator,
                    [feet[0], feet[1] + height * 0.5, feet[2]],
                    [0.1, 0.0, 0.1],
                    0.2,
                    (actual * 0.5) as u32,
                );
            }
            self.last_hurt_by_player
                .insert(self.mobs[index].id, self.lan_acting);
            let sound = if crit {
                "minecraft:entity.player.attack.crit"
            } else if scale > 0.9 {
                "minecraft:entity.player.attack.strong"
            } else {
                "minecraft:entity.player.attack.weak"
            };
            // Player.playServerSideSound: at the player.
            if let Some(at) = sweep {
                self.sweep_attack(at, from);
            } else {
                self.audio.play(sound, from, 1.0, 1.0);
            }
            // The hurt or death sound the mob raised (getHurtSound / getDeathSound,
            // its voice pitch from its own random).
            let queued = std::mem::take(&mut self.mobs[index].outbox);
            for event in queued {
                if let horizon_client::mobs::MobEvent::Sound(s, at, volume, pitch) = event {
                    self.audio
                        .play(&format!("minecraft:{s}"), Some(at), volume, pitch);
                }
            }
            self.weapon_hit(index);
        } else {
            self.audio
                .play("minecraft:entity.player.attack.nodamage", from, 1.0, 1.0);
        }
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
    /// Copies of a save batch carrying its live block entities and save data.
    fn with_entities<'a>(
        &self,
        chunks: impl IntoIterator<Item = &'a Arc<horizon_client::chunk::Chunk>>,
    ) -> Vec<Arc<horizon_client::chunk::Chunk>> {
        horizon_client::save::chunks_with_block_entities(
            chunks,
            &self.world.block_entities,
            &self.world.block_names,
        )
        .into_iter()
        .map(|mut c| {
            self.attach_save_data(&mut c);
            Arc::new(c)
        })
        .collect()
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
    /// Returns the sun and moon quads, and the stars' frame and brightness
    /// (the renderer keeps their geometry, built once).
    fn celestial_geometry(
        &self,
        sky: &horizon_client::environment::Sky,
    ) -> (Vec<Vertex>, Option<(glam::Mat4, f32)>) {
        let mut out = Vec::new();
        let atlas = &self.gui.atlas;
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
        // SkyRenderer.renderStars: the static star buffer on the same frame.
        let stars =
            (sky.star_brightness > 0.0).then(|| (frame(sky.star_angle), sky.star_brightness));
        (out, stars)
    }
    /// Enter a local world from the client, selecting its simulation mode.
    fn open_world(
        &mut self,
        folder: &str,
        name: Option<&str>,
        seed: Option<i64>,
        creative: bool,
        create: Option<&horizon_client::title::WorldOptions>,
    ) -> Result<()> {
        if self.defer_world_entry(PendingWorldEntry::Local {
            folder: folder.to_owned(),
            name: name.map(str::to_owned),
            seed,
            creative,
            create: create.cloned().map(Box::new),
        })? {
            return Ok(());
        }
        self.join_world_rest()?;
        if self
            .options
            .execution_role
            .starts_integrated_server(self.options.server_thread)
        {
            return self.open_integrated(folder, name, seed, creative, create);
        }
        let request = world_loading::WorldRequest::new(folder, name, seed, creative, create)?;
        if self.window.is_some() {
            self.perf.world_opening();
            if self.title.is_none() {
                self.title = Some(Title::new(
                    std::env::current_dir()?.join("saves"),
                    &self.options.resources,
                ));
            }
            self.world_loader = Some(world_loading::WorldLoader::start(
                request,
                self.res.clone(),
                self.pack.clone(),
                self.options.clone(),
                self.settings.clone(),
            )?);
            return Ok(());
        }
        self.load_world(request)
    }

    /// Load authoritative world state synchronously, without client entry or
    /// integrated-server selection. Server startup must use this entry point.
    fn load_world(&mut self, request: world_loading::WorldRequest) -> Result<()> {
        self.join_world_rest()?;
        self.perf.world_opening();
        let prepared = request.prepare(&self.res, &self.pack, &self.options, &self.settings)?;
        self.finish_open_world(prepared)
    }

    fn finish_open_world(&mut self, prepared: world_loading::PreparedWorld) -> Result<()> {
        let world_loading::PreparedWorld {
            save_root,
            level,
            world,
            streaming,
            save,
            session_lock,
            rand_sequences,
            locator,
        } = prepared;
        let dim = Dim::from_id(&level.dimension);
        horizon_client::crash::set_detail(
            "World",
            format!("{} ({})", level.name, save_root.display()),
        );
        self.session_lock = Some(session_lock);
        self.end.set_locator(locator);
        self.score = Default::default();
        self.fns = Default::default();
        self.bosses = Default::default();
        reset_world_progress(&mut self.adv, &mut self.stats);
        self.prefetch_advancement_tree();
        self.restore_player_state = true;
        self.rand_sequences = rand_sequences;
        self.border = Default::default();
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
        let index = &self.res.stations().recipe_index;
        let book = &mut self.level.recipe_book;
        book.recipes.retain(|r| index.contains(r));
        book.to_be_displayed.retain(|r| index.contains(r));
        self.dim = dim;
        self.world_root = save_root.clone();
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.clear_entities();
        self.travelling = None;
        self.player = None;
        self.sleeping = None;
        self.sleep_timer = 0;
        self.spawn = None;
        self.respawn_travel = None;
        self.respawn_config = self
            .level
            .respawn_elsewhere
            .as_ref()
            .map(respawn_anchor::RespawnConfig::from_saved);
        self.spawn_origin = None;
        self.spawn_spiral = Default::default();
        self.player_spawn = None;
        self.creatures.reset_trader();
        self.portals = Default::default();
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
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.apply_settings();
        self.streaming.sky_light = dim != Dim::Nether;
        self.streaming.nether_shading = dim == Dim::Nether;
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
        let (streaming, save, locator) = world_loading::make_streaming(
            &self.res,
            &self.pack,
            &self.options,
            &self.settings,
            world_root,
            dim,
            seed,
        )?;
        self.end.set_locator(locator);
        Ok((streaming, save))
    }
    /// Change dimension through a portal (Entity.changeDimension with
    /// PortalForcer placement once the destination has loaded).
    fn enter_dimension(&mut self, target: Dim) -> Result<()> {
        let Some(player) = self.player.take() else {
            return Ok(());
        };
        let from = self.dim;
        let destination = self.switch_level(target, player.position)?;
        self.trigger_dimension(from, target);
        self.travelling = Some((player, destination));
        self.end_travel = (target == Dim::End || from == Dim::End) && self.end_portal_travel;
        Ok(())
    }
    /// The level swap under a dimension change: save the departing level,
    /// open `target`'s, and return where a player leaving from `from` goes
    /// (scaled coordinates, or the End portal's fixed destination).
    pub(super) fn switch_level(&mut self, target: Dim, from: Vec3) -> Result<Vec3> {
        // Save the departing dimension's changed chunks.
        self.streaming.save(&mut self.world, true);
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks = self.with_entities(keys.iter().filter_map(|k| self.world.chunks.get(k)));
        self.save
            .save(&chunks, self.pack.assets.registry, self.res.biomes())?;
        self.save_dimension_entities()?;
        // Coordinates scale by 8 between the Overworld and the Nether.
        let scale = match (self.dim, target) {
            (Dim::Overworld, Dim::Nether) => 1.0 / 8.0,
            (Dim::Nether, Dim::Overworld) => 8.0,
            (Dim::End, Dim::Nether) => 1.0 / 8.0,
            _ => 1.0,
        };
        let destination = self.end_destination(target).unwrap_or(Vec3::new(
            from.x * scale,
            from.y,
            from.z * scale,
        ));
        let (streaming, save) =
            self.make_streaming(&self.world_root.clone(), target, self.level.seed)?;
        let mut world = World::new(self.res.facts().air, self.level.seed);
        world.fast_lava = target == Dim::Nether;
        world.game_time = self.world.game_time;
        world.day_time = self.world.day_time;
        world.weather = self.world.weather;
        // The game rules and the difficulty hold in every dimension.
        world.health_rules = self.world.health_rules;
        // An open hub keeps hearing the new level's block changes.
        world.block_log = self.world.block_log.as_ref().map(|_| Vec::new());
        world.light_log = self.world.light_log.as_ref().map(|_| Default::default());
        // The sculk tables' watched blocks are built once per game.
        world.watched = std::mem::take(&mut self.world.watched);
        self.streaming = streaming;
        self.world = world;
        self.save = save;
        self.dim = target;
        self.level.dimension = target.id().to_owned();
        self.block_ticks = Ticks::new(self.level.seed as i32);
        self.tick_chunks.clear();
        self.falling.clear();
        self.tnts.clear();
        self.mobs.clear();
        self.creatures.projectiles.clear();
        self.dropped.clear();
        self.particles.clear();
        self.pickups.clear();
        self.orbs.clear();
        self.mining = None;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.apply_settings();
        Ok(destination)
    }
    /// PortalShape: an obsidian frame around 2..=21 by 3..=21 empty blocks.
    fn portal_shape(&self, start: [i32; 3], x_axis: bool) -> Option<([i32; 3], i32, i32)> {
        let name = |p: [i32; 3]| {
            self.res
                .facts()
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
    /// BaseFireBlock.onPlace: lighting fire inside a frame creates a portal,
    /// only in the Overworld and the Nether (`inPortalDimension`).
    fn try_light_portal(&mut self, at: [i32; 3]) -> Result<bool> {
        if !matches!(self.dim, Dim::Overworld | Dim::Nether) {
            return Ok(false);
        }
        for x_axis in [true, false] {
            if let Some((origin, width, height)) = self.portal_shape(at, x_axis) {
                let d = if x_axis { [1, 0, 0] } else { [0, 0, 1] };
                let registry = &self.pack.assets.registry;
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
    /// Arrival through a nether portal, once the destination is loaded.
    fn place_after_travel(&mut self, player: PlayerMotion, target: Vec3) -> Result<()> {
        self.arrive_through_nether_portal(player, target)
    }
    /// Leaving the game with a world open (window closed, Quit, `quit`):
    /// menu items go back (Player.closeContainer), then the world is saved,
    /// once.
    /// Keeps this play session's performance summary for the launcher to send, if the player
    /// allows performance statistics (`horizon_client::telemetry`), and starts the next one.
    pub(super) fn finish_perf(&mut self) {
        let s = &self.settings;
        let settings = horizon_telemetry::payload::Settings {
            render_distance: s.render_distance.clamp(0, 255) as u8,
            simulation_distance: s.simulation_distance.clamp(0, 255) as u8,
            // 260 is the slider's "unlimited".
            max_fps: if s.max_fps >= 260 {
                0
            } else {
                s.max_fps.min(u32::from(u16::MAX)) as u16
            },
            vsync: s.vsync,
            fullscreen: s.fullscreen,
            graphics: s.graphics_preset.clone(),
        };
        // Again now: on Wayland the window is on a screen only once it has been shown.
        if let Some(window) = &self.window {
            horizon_client::telemetry::note_display(window);
        }
        self.perf
            .finish(&horizon_client::logging::game_home(), settings);
    }

    pub(super) fn save_for_exit(&mut self) -> Result<()> {
        // Native application termination can bypass CloseRequested. Finish any
        // save screen synchronously here, before the process can go away.
        while self.pending_save.is_some() {
            self.poll_closing_save()?;
            if self.pending_save.is_some() {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
        if self.title.is_some() || self.saved_for_exit {
            return Ok(());
        }
        if let Some(server) = self.integrated.take() {
            // The server thread saves the world.
            server.stop()?;
            self.saved_for_exit = true;
            return Ok(());
        }
        self.close_container();
        if self.remote.is_none() {
            self.stats.custom("leave_game", 1);
        }
        self.lan_close(false);
        self.save_game()?;
        self.saved_for_exit = true;
        Ok(())
    }
    fn close_world(&mut self) -> Result<()> {
        self.clear_waypoints();
        self.begin_closing_save(false)
    }
    fn finish_close_world(&mut self) -> Result<()> {
        self.ambience.clear(&mut self.audio);
        // updateLevelInEngines(null): every sound stops; the music manager
        // lets go of its track.
        self.music.stop_playing(None, &mut self.audio);
        self.audio.stop_all();
        if self.remote.is_some() {
            return self.leave_server(None);
        }
        // The save screen has finished every write before releasing storage.
        self.session_lock = None;
        horizon_client::crash::clear_detail("World");
        self.debug_charts.reset();
        self.boss_overlay.reset();
        self.bosses = Default::default();
        self.streaming.paused = true;
        self.world = World::new(self.res.facts().air, 0);
        self.player = None;
        self.sleeping = None;
        self.sleep_timer = 0;
        self.clear_entities();
        self.screen = Screen::Playing;
        self.clear_lod();
        self.pending_uploads.clear();
        if let Some(r) = self.renderer.as_mut() {
            r.clear_sections();
        }
        self.title = Some(Title::new(
            std::env::current_dir()?.join("saves"),
            &self.options.resources,
        ));
        if let Some(title) = &mut self.title {
            title.last_server = self.settings.file.string("lastServer");
        }
        self.capture(false);
        Ok(())
    }
    /// Let go of every entity of the level (saved first): the next world
    /// opened must not inherit them, nor save them as its own.
    fn clear_entities(&mut self) {
        self.mobs.clear();
        self.dropped.clear();
        self.orbs.clear();
        self.creatures.projectiles.clear();
        self.falling.clear();
        self.tnts.clear();
        self.boats.clear();
        self.carts.clear();
        self.armor_stands.clear();
        self.item_frames.clear();
        self.paintings.clear();
        self.others.clear();
        self.dragon = None;
        self.crystals.clear();
        self.dragon_fireballs.clear();
        self.breath_clouds.clear();
        self.tridents.clear();
        self.rockets.clear();
        self.bolts.clear();
        self.particles.clear();
        self.pickups.clear();
        self.riding = None;
        self.persist = Default::default();
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
            let Some(info) = stack.and_then(|s| self.res.items().get(&s.item)) else {
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
                && let Some(info) = self.res.items().get(&stack.item)
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
                let f = self.res.facts().get(state);
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
                if !self.res.facts().is_air(state) && f.fluid == Fluid::None {
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
                self.0.held_mut()
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
        let registry = self.pack.assets.registry;
        let id = self.world.block(pos[0], pos[1], pos[2]);
        let Some(state) = registry
            .state(id)
            .filter(|s| s.name.ends_with("cauldron"))
            .cloned()
        else {
            return Ok(false);
        };
        let Some(held) = self.held() else {
            return Ok(false);
        };
        let removes_dye = cauldron::removes_dye(self.res.items(), &held.item);
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
    /// What `BlockTransformer.transformBlock` would do with a held axe, hoe
    /// or shovel on the block at `pos`, without doing it: the new state, its
    /// sound, loot table, the durability it costs and its particle.
    pub(super) fn block_transform(
        &self,
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
    ) -> Option<(BlockStateId, String, Option<String>, u32, String)> {
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
        let id = self
            .res
            .items()
            .get(&stack.item)
            .and_then(|i| i.block_transformer.clone())?;
        // playerHasBlockingItemUseIntent: a shield in the off hand is raised
        // instead, unless sneaking.
        let shield = self
            .offhand
            .as_ref()
            .and_then(|s| self.res.items().get(&s.item))
            .is_some_and(|i| i.blocks_attacks.is_some());
        if shield && !self.keys.contains(&KeyCode::ShiftLeft) {
            return None;
        }
        let face_name = match face {
            [0, -1, 0] => "down",
            [0, 1, 0] => "up",
            [0, 0, -1] => "north",
            [0, 0, 1] => "south",
            [-1, 0, 0] => "west",
            _ => "east",
        };
        let registry = self.pack.assets.registry;
        let around = Column {
            world: &self.world,
            registry,
            pos,
        };
        let (_, t, state) =
            self.res
                .transformers()
                .apply(self.res.items(), registry, &id, face_name, &around)?;
        let new = registry.resolve(&state).ok()?;
        Some((
            new,
            t.sound.clone(),
            t.loot.clone(),
            t.damage,
            t.particle.clone(),
        ))
    }
    /// `BlockTransformer.TransformParticle.send`: its level event.
    pub(super) fn transform_particle(&mut self, particle: &str, pos: [i32; 3]) {
        match particle {
            "scrape" => self.level_event_at(3005, pos, 0),
            "wax_on" => self.level_event_at(3003, pos, 0),
            "wax_off" => self.level_event_at(3004, pos, 0),
            _ => {}
        }
    }
    /// `BlockTransformer.transformBlock` for a held axe, hoe or shovel.
    fn use_block_transformer(
        &mut self,
        stack: &ItemStack,
        pos: [i32; 3],
        face: [i32; 3],
    ) -> Result<bool> {
        let Some((new, sound, loot, damage, particle)) = self.block_transform(stack, pos, face)
        else {
            return Ok(false);
        };
        self.level_set(pos, new)?;
        // BlockTransformer.TransformParticle.send.
        self.transform_particle(&particle, pos);
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
        self.post_block_game_event("minecraft:block_change", pos, self.sculk_actor(), Some(new));
        Ok(true)
    }
    fn set_state_named(&mut self, pos: [i32; 3], name: &str, keep: &[&str]) -> Result<bool> {
        let registry = &self.pack.assets.registry;
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
        let amount = match self.held().cloned() {
            Some(held) => self.durability_loss(&held, amount),
            None => amount,
        };
        let max = self
            .held()
            .and_then(|s| self.res.items().get(&s.item))
            .map_or(0, |i| i.max_damage);
        let cell = self.held_mut();
        if let Some(stack) = cell.as_mut()
            && max > 0
        {
            stack.damage += amount;
            if stack.damage >= max {
                let broken = stack.item.clone();
                *cell = None;
                self.stats.add("broken", &broken, 1);
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
    /// Block tool use: distinguish success, failure and fall-through for hand feedback.
    fn use_tool(
        &mut self,
        stack: &ItemStack,
        allow_block_actions: bool,
    ) -> Result<horizon_client::first_person::UseResult> {
        use horizon_client::first_person::UseResult;
        let item = stack.item.trim_start_matches("minecraft:").to_owned();
        let at = |p: [i32; 3]| {
            [
                f64::from(p[0]) + 0.5,
                f64::from(p[1]) + 0.5,
                f64::from(p[2]) + 0.5,
            ]
        };
        if allow_block_actions
            && let Some(hit) = self.pick()
            && self.use_on_cauldron(hit.pos)?
        {
            return Ok(UseResult::Success { swing: true });
        }
        // BucketItem has no useOn: a guest's UseItemOn leaves it to UseItem.
        let bucket = horizon_client::items::bucket_contents(&stack.item);
        if matches!(self.guest_use, Some(GuestUse::OnBlock(_)))
            && (item == "bucket" || bucket.is_some())
        {
            return Ok(UseResult::Pass);
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
                    return Ok(UseResult::Success { swing: true });
                }
            }
            let Some(hit) = self.pick_fluid() else {
                return Ok(UseResult::Pass);
            };
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            let f = self.res.facts().get(state).clone();
            if f.fluid == Fluid::None || f.fluid_level != 8 || f.fluid_falling {
                return Ok(UseResult::Pass);
            }
            let sound = match f.fluid {
                Fluid::Water => "minecraft:item.bucket.fill",
                _ => "minecraft:item.bucket.fill_lava",
            };
            let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
            let Some(filled) =
                self.with_level(|b, level, ticks| b.bucket_pickup(level, ticks, pos))
            else {
                return Ok(UseResult::Pass);
            };
            self.process_level_events()?;
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
                if self.held().is_none() {
                    *self.held_mut() = Some(filled);
                } else if let Some(rest) = self.give(filled)
                    && let Some(eye) = self.eye_position()
                {
                    self.spawn_item(rest, eye, [0.0, 0.1, 0.0], 40);
                }
            }
            return Ok(UseResult::Success { swing: true });
        }
        // BucketItem.use has its own ray, even when the crosshair hits an entity.
        if !allow_block_actions && bucket.is_none() {
            return Ok(UseResult::Pass);
        }
        let Some(hit) = self.pick() else {
            return Ok(UseResult::Pass);
        };
        let clicked = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if let Some((content, sound, mob)) = bucket {
            if !self.may_build() {
                return Ok(UseResult::Fail);
            }
            let target = if self.res.facts().get(clicked).replaceable {
                hit.pos
            } else {
                [
                    hit.pos[0] + hit.face[0],
                    hit.pos[1] + hit.face[1],
                    hit.pos[2] + hit.face[2],
                ]
            };
            let pos = BlockPos::new(target[0], target[1], target[2]);
            let emptied = self
                .with_level(|b, level, ticks| b.bucket_empty(level, ticks, pos, content, sound));
            self.process_level_events()?;
            if !emptied {
                return Ok(UseResult::Fail);
            }
            if let Some(kind) = mob {
                if let Some(id) = self.spawn_mob_at(kind, (target[0], target[1], target[2]), true) {
                    if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
                        horizon_client::entity_nbt::load_bucket_data(m, stack);
                    }
                    self.mob_placed(id)?;
                }
                self.post_block_game_event(
                    "minecraft:entity_place",
                    target,
                    self.sculk_actor(),
                    None,
                );
            }
            self.trigger_block("minecraft:placed_block", target, stack);
            if !self.creative {
                *self.held_mut() = Some(ItemStack::new("minecraft:bucket", 1));
            }
            return Ok(UseResult::Success { swing: true });
        }
        if self.use_block_transformer(stack, hit.pos, hit.face)? {
            return Ok(UseResult::Success { swing: true });
        }
        if item == "honeycomb" {
            let registry = self.pack.assets.registry;
            let old = registry.state(clicked).cloned();
            if let Some(waxed) = old.and_then(|old| self.res.transformers().waxed(registry, &old))
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
                return Ok(UseResult::Success { swing: true });
            }
        }
        if item == "flint_and_steel" || item == "fire_charge" {
            if !self.may_build() {
                return Ok(UseResult::Fail);
            }
            let name = self.res.facts().get(clicked).name.clone();
            if self.can_light(clicked, &name) {
                let registry = self.pack.assets.registry;
                let mut state = registry
                    .state(clicked)
                    .cloned()
                    .ok_or("missing lit block state")?;
                state.properties.insert("lit".into(), "true".into());
                self.level_set(hit.pos, registry.resolve(&state)?)?;
                self.audio.play(
                    if item == "fire_charge" {
                        "minecraft:item.firecharge.use"
                    } else {
                        "minecraft:item.flintandsteel.use"
                    },
                    Some(at(hit.pos)),
                    1.0,
                    horizon_client::items::fire_starter_pitch(
                        item == "fire_charge",
                        &mut self.world.random,
                    ),
                );
                if item == "fire_charge" {
                    self.consume_held();
                } else {
                    self.damage_held(1);
                }
                return Ok(UseResult::Success { swing: true });
            }
            let target = [
                hit.pos[0] + hit.face[0],
                hit.pos[1] + hit.face[1],
                hit.pos[2] + hit.face[2],
            ];
            if self.can_place_used_fire(target) {
                if !self.try_light_portal(target)? {
                    self.place_used_fire(target)?;
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
                return Ok(UseResult::Success { swing: true });
            }
            return Ok(UseResult::Fail);
        }
        Ok(UseResult::Pass)
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
        self.next_entity += 1;
        self.orbs.push(ExperienceOrb {
            id: self.next_entity,
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
    fn can_place_used_fire(&mut self, target: [i32; 3]) -> bool {
        let pos = BlockPos::new(target[0], target[1], target[2]);
        let direction = match ((self.yaw / 90.0 + 0.5).floor() as i32).rem_euclid(4) {
            0 => Direction::South,
            1 => Direction::West,
            2 => Direction::North,
            _ => Direction::East,
        };
        self.with_level(|b, level, ticks| b.fire_can_be_placed_at(level, ticks, pos, direction))
    }
    fn place_used_fire(&mut self, target: [i32; 3]) -> Result<()> {
        let pos = BlockPos::new(target[0], target[1], target[2]);
        let state = self.with_level(|b, level, ticks| b.fire_state_for(level, ticks, pos));
        self.level_set(target, state)
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
            // SpawnerBlock.spawnAfterBreak: 15 + nextInt(15) + nextInt(15).
            "spawner" => {
                let r = &mut self.world.random;
                let a = r.next_int_bound(15).unwrap_or(0);
                return (15 + a + r.next_int_bound(15).unwrap_or(0)) as u32;
            }
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
        // ExperienceOrb.followNearbyPlayer: each orb goes for the nearest
        // living player within 8 blocks, the host's own or a guest.
        let mut followed: Vec<([f64; 3], [f64; 3])> = player
            .map(|p| {
                let feet = [p.position.x, p.position.y, p.position.z];
                let eye = f64::from(p.dimensions.eye_height);
                (feet, [feet[0], feet[1] + eye / 2.0, feet[2]])
            })
            .into_iter()
            .collect();
        if let Some(host) = self.lan.as_ref() {
            followed.extend(
                host.hub
                    .guests
                    .iter()
                    .filter(|g| g.placed && g.loaded && !g.dead && g.health > 0.0)
                    .filter(|g| !g.spectator())
                    .map(|g| {
                        let p = g.position;
                        (p, [p[0], p[1] + g.eye_height() / 2.0, p[2]])
                    }),
            );
        }
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
        orbs.retain_mut(|o| {
            let distance = |feet: &[f64; 3]| -> f64 {
                (feet[0] - o.position.x).powi(2)
                    + (feet[1] - o.position.y).powi(2)
                    + (feet[2] - o.position.z).powi(2)
            };
            let target = followed
                .iter()
                .filter(|(feet, _)| distance(feet) < 64.0)
                .min_by(|a, b| distance(&a.0).total_cmp(&distance(&b.0)))
                .map(|(_, target)| *target);
            o.tick(&self.world, self.res.facts(), self.collision, target)
        });
        let mut gained = 0;
        let mut taken = Vec::new();
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
                    taken.push(o.clone());
                    return false;
                }
                true
            });
        }
        self.orbs = orbs;
        for o in taken {
            self.add_pickup(pickups::Taken::Orb(Box::new(o)), pickups::Collector::Player);
        }
        if gained > 0 {
            gained = self.mending(gained);
        }
        if gained > 0 {
            let at = self.eye_position();
            let pitch =
                (self.world.random.next_float() - self.world.random.next_float()) * 0.35 + 0.9;
            self.audio
                .play("minecraft:entity.experience_orb.pickup", at, 0.1, pitch);
            self.give_orb_points(gained);
        }
    }
    /// `Player.giveExperiencePoints`, and `giveExperienceLevels`' level-up
    /// sound every fifth level (at most every 100 ticks).
    pub(super) fn give_orb_points(&mut self, gained: u32) {
        let at = self.eye_position();
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
    /// BowItem.releaseUsing: power from charge time, arrows at power * 3.
    fn release_bow(&mut self, ticks: u32) {
        let f = ticks as f32 / 20.0;
        let power = ((f * f + f * 2.0) / 3.0).min(1.0);
        if power < 0.1 {
            return;
        }
        let arrow_slot = self.projectile_slot("minecraft:bow");
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
        // ProjectileWeaponItem.useAmmo: creative and Infinity draw without
        // using the ammo, so the arrow is CREATIVE_ONLY; else setOwner(player)
        // lets it be picked up.
        let ammo = arrow_slot
            .and_then(|i| self.stack_in(i).cloned())
            .unwrap_or_else(|| ItemStack::new("minecraft:arrow", 1));
        if let Some(a) = arrow.arrow.as_mut() {
            a.set_item(&horizon_client::arrows::drawn_ammo(
                &ammo,
                self.creative || infinity,
            ));
            a.set_owner(Some(horizon_client::mobs::Who::Player));
        }
        // Flame: the arrow is spawned burning (EnchantmentHelper.onProjectileSpawned).
        if let Some(seconds) = self.bow_projectile_ignite()
            && let Some(a) = arrow.arrow.as_mut()
        {
            a.ignite_for_seconds(seconds);
        }
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
        {
            let cell = self.stack_in_mut(i);
            if let Some(s) = cell.as_mut() {
                s.count -= 1;
                if s.count == 0 {
                    *cell = None;
                }
            }
        }
        self.damage_held(1);
    }
    /// FirstPersonHandsAndItemsRenderer: the right arm or the held items of
    /// both hands in view space, converted to camera-relative world space.
    fn hand_geometry(&mut self, alpha: f64) -> Vec<Vertex> {
        let mut out = Vec::new();
        // GameRenderer.renderItemInHand: F1 hides the hand with the HUD.
        if self.spectator()
            || self.ui.hide_gui
            || self.sleeping.is_some()
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
        // FirstPersonHandsAndItemsRenderer: the swing goes to the hand that
        // swings; each hand has its inverse arm height (the swap scale is 1
        // for every item but spears).
        let swing = self.swing.animation(a);
        let attacks = Hand::BOTH.map(|h| {
            if self.swing.attack_hand() == h {
                swing
            } else {
                0.0
            }
        });
        let inverse_heights = self.heights.interpolated(a).map(|h| 1.0 - h);
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
        let shown = self.hands_to_render();
        let used_hand = self.used_hand();
        let spinning = self
            .player
            .as_ref()
            .is_some_and(|p| p.auto_spin_attack_ticks > 0);
        let items = self.visible_hand_items();
        let off_hand_empty = items[1].is_none();
        for (i, (hand, item)) in Hand::BOTH.into_iter().zip(items).enumerate() {
            if !shown[i] {
                continue;
            }
            let (attack, inverse_height) = (attacks[i], inverse_heights[i]);
            let sqrt_attack = attack.sqrt();
            // The main hand on its arm, the off hand on the other one.
            let left = self.settings.main_hand_left != (hand == Hand::Off);
            let Some(stack) = item else {
                // Only the main hand's empty arm shows.
                if hand == Hand::Off {
                    continue;
                }
                let Some(arm) = self.res.player_arm().clone() else {
                    continue;
                };
                let Some(r) = self
                    .gui
                    .atlas
                    .sprites
                    .get("entity/player/wide/steve")
                    .copied()
                else {
                    continue;
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
                continue;
            };
            if let Some(id) = maps::map_id(&stack) {
                use horizon_client::first_person::{MapHold, one_handed_map_pose};
                match MapHold::of(hand, off_hand_empty) {
                    MapHold::TwoHanded => self.two_handed_map_geometry(
                        to_world,
                        attack,
                        inverse_height,
                        id,
                        packed,
                        &mut out,
                    ),
                    MapHold::OneHanded => self.held_map(
                        to_world * one_handed_map_pose(left, inverse_height, attack),
                        id,
                        packed,
                        &mut out,
                    ),
                }
                continue;
            }
            let mut owner = self.player_owner(Some(&stack));
            // The other hand may hold an equal stack: only the used
            // hand's item is in use, only the main hand's selected.
            owner.using_this_item &= used_hand == Some(hand);
            if hand == Hand::Off {
                owner.is_selected = false;
                owner.is_carried = false;
            }
            // The hand renderer's copy of the main hand stack.
            owner.fishing_cast_with_this_item =
                self.main_hand_rod() && self.cast_rod() == Some(&stack);
            let using = owner.using_this_item && owner.use_item_remaining_ticks > 0;
            let ticks_used =
                owner.use_duration as f32 - (owner.use_item_remaining_ticks as f32 - a + 1.0);
            use horizon_client::first_person::ItemPose;
            let pose = match &*stack.item {
                "minecraft:bow" if using => ItemPose::Bow { ticks_used },
                "minecraft:crossbow" => {
                    let charged = stack.data().is_some_and(|d| !d.charged.is_empty());
                    ItemPose::Crossbow {
                        ticks_used: (using && !charged).then_some(ticks_used),
                        charge_duration: owner.crossbow_charge_duration as f32,
                        // Only the main hand turns a charged crossbow aside.
                        charged: charged && hand == Hand::Main,
                    }
                }
                "minecraft:trident" if using => ItemPose::Trident { ticks_used },
                _ if using => {
                    let components = self
                        .pack
                        .item_renderer
                        .models
                        .components(&horizon_client::item_render::stack_view(&stack));
                    let animation = components
                        .get("minecraft:consumable")
                        .map(horizon_client::first_person::consumable_animation);
                    if matches!(
                        animation,
                        Some(
                            horizon_world::consumable::UseAnimation::Eat
                                | horizon_world::consumable::UseAnimation::Drink
                        )
                    ) {
                        ItemPose::Eat {
                            remaining: owner.use_item_remaining_ticks as f32,
                            duration: owner.use_duration as f32,
                            partial: a,
                        }
                    } else {
                        ItemPose::Using
                    }
                }
                _ if spinning => ItemPose::SpinAttack,
                _ => ItemPose::Swing,
            };
            let m = to_world
                * horizon_client::first_person::item_pose(left, inverse_height, attack, pose);
            // Seeded by the player id plus the context ordinal.
            let (context, seed) = if left {
                (
                    horizon_assets::item_model::DisplayContext::FirstPersonLeftHand,
                    3,
                )
            } else {
                (
                    horizon_assets::item_model::DisplayContext::FirstPersonRightHand,
                    4,
                )
            };
            let state = self.resolve_item(&stack, context, Some(&owner), seed);
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
        out
    }
    /// The renderer for the window, with the current block sprites, GUI
    /// atlas and world border texture.
    fn new_renderer(&self, window: Arc<Window>) -> Result<Renderer> {
        self.renderer_for(window, &self.pack, &self.gui)
    }
    fn renderer_for(
        &self,
        window: Arc<Window>,
        pack: &PackResources,
        gui: &UiResources,
    ) -> Result<Renderer> {
        build_renderer(
            window,
            self.options.offscreen,
            pack,
            gui,
            self.renderer.as_ref(),
            None,
            self.settings.horizon.api,
        )
    }
    fn apply_settings(&mut self) {
        if let Some(r) = &self.renderer {
            self.settings
                .clamp_gui_scale([r.config.width as i32, r.config.height as i32]);
        }
        self.streaming.radius = self.settings.render_distance;
        // Options.biomeBlendRadius, ambientOcclusion and cutoutLeaves:
        // LevelExtractor.allChanged when they change.
        let (s, st) = (&self.settings, &self.streaming);
        let remesh = (s.biome_blend, s.smooth_lighting, s.cutout_leaves)
            != (st.biome_blend, st.smooth_lighting, st.cutout_leaves);
        self.streaming.biome_blend = self.settings.biome_blend;
        self.streaming.smooth_lighting = self.settings.smooth_lighting;
        self.streaming.cutout_leaves = self.settings.cutout_leaves;
        if remesh {
            self.all_sections_changed();
        }
        self.audio.master = self.settings.master_volume;
        horizon_client::hud::GUI_SCALE.store(
            self.settings.gui_scale,
            std::sync::atomic::Ordering::Relaxed,
        );
        self.apply_ui_settings();
    }
    /// `LevelExtractor.allChanged`: every loaded section is rebuilt.
    fn all_sections_changed(&mut self) {
        for &(x, z) in self.world.chunks.keys() {
            for sy in 0..horizon_client::chunk::HEIGHT / 16 {
                self.world.dirty_sections.insert((x, sy, z));
            }
        }
    }
    /// Whether precipitation at a column is snow (Biome.coldEnoughToSnow:
    /// base temperature lowered above y 80) or none (no precipitation).
    fn precipitation(&self, x: i32, y: i32, z: i32) -> Option<bool> {
        let chunk = self.world.chunks.get(&(x >> 4, z >> 4))?;
        let b = self.res.biomes().get(chunk.biome(x, y, z));
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
    /// `getHeight(Heightmap.Types.MOTION_BLOCKING, x, z)`: one above the
    /// highest block in `blocks_motion_in_heightmap` or holding a fluid.
    fn rain_height(&self, x: i32, z: i32) -> i32 {
        use horizon_world::{heightmap::HeightmapType, worldgen::terrain::TerrainBlockStates};
        let properties = &self.pack.assets.properties;
        self.world.motion_blocking(x, z, |s| {
            HeightmapType::MotionBlocking.is_opaque(properties.heightmap_state(s))
        })
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
            let top = self.rain_height(x, z);
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
    fn weather_geometry(
        &mut self,
        camera: glam::DVec3,
        alpha: f64,
    ) -> Vec<horizon_client::weather::Vertex> {
        use horizon_client::weather;
        let (intensity, _) = self.world.weather.levels(alpha as f32);
        if intensity <= 0.0 {
            return Vec::new();
        }
        let atlas = &self.gui.atlas;
        let (Some(rain_tex), Some(snow_tex)) = (
            atlas.sprites.get("environment/rain").copied(),
            atlas.sprites.get("environment/snow").copied(),
        ) else {
            return Vec::new();
        };
        let (cx, cy, cz) = (
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        );
        // The 32x32 direction table supports at most a 15-block radius.
        let radius = self.settings.weather_radius.clamp(0, 15);
        let eye = camera.to_array();
        let mut rain = Vec::new();
        let mut snow_vertices = Vec::new();
        // Saturation keeps extreme camera coordinates from overflowing the loop.
        for z in cz.saturating_sub(radius)..=cz.saturating_add(radius) {
            for x in cx.saturating_sub(radius)..=cx.saturating_add(radius) {
                let top = self.rain_height(x, z);
                let y0 = cy.saturating_sub(radius).max(top);
                let y1 = cy.saturating_add(radius).max(top);
                if y1 == y0 {
                    continue;
                }
                let Some(snow) = self.precipitation(x, cy, z) else {
                    continue;
                };
                let parameters = self.weather_parameters.get(x, z);
                let light = self.world.light(x, top.max(cy), z);
                let coords = i32::from(light & 15) << 4 | i32::from(light >> 4) << 20;
                // Only light brightening is needed here; UV phases stay zero.
                let lc = if snow {
                    let (block, sky) = (coords >> 4 & 15, coords >> 20 & 15);
                    ((block * 3 + 15) / 4) << 4 | ((sky * 3 + 15) / 4) << 20
                } else {
                    coords
                };
                let column = weather::Column {
                    x,
                    z,
                    bottom_y: y0,
                    top_y: y1,
                    u_offset: 0.0,
                    v_offset: 0.0,
                    light_coords: lc,
                };
                let r = if snow { snow_tex } else { rain_tex };
                let vertices = weather::vertices(
                    &column,
                    parameters,
                    snow,
                    eye,
                    radius,
                    intensity,
                    [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0],
                );
                if snow {
                    snow_vertices.extend(vertices);
                } else {
                    rain.extend(vertices);
                }
            }
        }
        // Vanilla draws every rain column before the snow ones.
        rain.extend(snow_vertices);
        rain
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
            .pack
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
        let dist = ((p.position.x - f64::from(head[0]) - 0.5).powi(2)
            + (p.position.z - f64::from(head[2]) - 0.5).powi(2))
        .sqrt();
        if dist > 3.0 {
            self.message = Some((
                "You may not rest now; the bed is too far away".into(),
                Instant::now(),
            ));
            return;
        }
        // In reach (bedInRange), day or night: setRespawnPosition(
        // RespawnConfig(the head, the player's yaw), not forced). It replaces
        // an anchor or a spawnpoint; respawning checks the bed is still there
        // and finds where to stand up (findRespawnAndUseSpawnBlock).
        // Straw beds never set it (`straw_bed_rule`).
        if self.bed_sets_spawn(state) {
            self.respawn_config = Some(respawn_anchor::RespawnConfig::wrapped(
                self.dim, head, p.yaw, false,
            ));
            // The fallback becomes the world spawn: a guest's is none of its
            // own, so it follows `setworldspawn`.
            self.spawn = if self.lan_acting.is_some() {
                None
            } else {
                self.level
                    .world_spawn
                    .map(|[x, y, z]| {
                        Vec3::new(f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5)
                    })
                    .or(self.spawn)
            };
        }
        if !night {
            self.message = Some((
                "You can sleep only at night or during thunderstorms".into(),
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
        self.stop_mining();
        self.left = false;
        self.right = false;
        self.using = None;
        self.sleeping = Some(head);
        self.sleep_timer = 0;
        self.stats.custom("sleep_in_bed", 1);
        self.trigger_player("minecraft:slept_in_bed");
        self.stats.reset("custom", "minecraft:time_since_rest");
    }
    fn wake_up(&mut self) {
        if self.remote.is_some() {
            // InBedChatScreen.sendWakeUp: the server wakes the player.
            self.remote_stop_sleeping();
            return;
        }
        self.sleeping = None;
        self.sleep_timer = 100;
    }
    fn tick_sleep(&mut self) {
        self.sleep_timer = tick_sleep_timer(self.sleep_timer, self.sleeping.is_some());
        if let Some(head) = self.sleeping {
            if self.keys.contains(&KeyCode::ShiftLeft)
                || self
                    .player
                    .as_ref()
                    .is_none_or(|p| p.health.dead_or_dying())
            {
                self.wake_up();
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
        }
        // ServerLevel.tick: once enough of the players have slept 100 ticks,
        // skip to morning, clear the weather and wake everyone.
        if self.enough_slept() {
            let t = self.world.day_time;
            self.world.day_time = t + 24000 - t.rem_euclid(24000);
            self.world.weather.raining = false;
            self.world.weather.thundering = false;
            self.world.weather.rain_time = 0;
            self.world.weather.thunder_time = 0;
            if self.sleeping.is_some() {
                self.wake_up();
            }
            self.wake_guests();
        }
    }
    fn creative_origin(batch: &HudBatch) -> (f32, f32) {
        batch.centered_origin(Creative::WIDTH, Creative::HEIGHT)
    }
    /// Vanilla tooltip: dark box with a purple gradient border.
    fn tooltip(&self, batch: &mut HudBatch, text: &str, x: f32, y: f32) {
        let atlas = &self.gui.atlas;
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
            &self.gui.atlas,
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
                batch.sprite(&self.gui.atlas, &tab_sprite(t, false), ox + tx, oy + ty);
            }
        }
        let background = if tab.search {
            "gui/container/creative_inventory/tab_item_search"
        } else if tab.inventory {
            "gui/container/creative_inventory/tab_inventory"
        } else {
            "gui/container/creative_inventory/tab_items"
        };
        if let Some(mut r) = self.gui.atlas.sprites.get(background).copied() {
            r.u1 = r.u0 + (r.u1 - r.u0) * Creative::WIDTH / r.w as f32;
            r.v1 = r.v0 + (r.v1 - r.v0) * Creative::HEIGHT / r.h as f32;
            batch.rect_uv(ox, oy, Creative::WIDTH, Creative::HEIGHT, r, u32::MAX);
        }
        let (tx, ty) = Creative::tab_position(&tab);
        batch.sprite(&self.gui.atlas, &tab_sprite(&tab, true), ox + tx, oy + ty);
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
        let atlas = &self.gui.atlas;
        if !tab.inventory {
            batch.text(
                atlas,
                &tab.title,
                ox + 8.0,
                oy + 6.0,
                rgba(64, 64, 64, 255),
                false,
            );
        }
        if tab.search {
            // The search box: an unbordered EditBox of 80 at (82, 6), white.
            let search = &self.creative_screen.search;
            horizon_client::ui::append_field(
                batch,
                atlas,
                search,
                (ox + 82.0, oy + 6.0, 80.0),
                (u32::MAX, true),
                clipboard::CREATIVE_SEARCH_CHARS,
                self.text_selection
                    .covers(clipboard::CREATIVE_SEARCH, search),
                true,
                self.creative_screen.search_focused.elapsed().as_millis(),
            );
        }
        let mut hovered: Option<ItemStack> = None;
        if tab.inventory {
            self.draw_player_preview(
                batch,
                ox + 73.0,
                oy + 6.0,
                ox + 105.0,
                oy + 49.0,
                20.0,
                0.0625,
            );
            self.draw_inventory_slots(batch, ox, oy);
            if Self::creative_trash_hovered(batch, self.cursor) {
                let (x, y) = Creative::TRASH_POSITION;
                batch.fill(
                    &self.gui.atlas,
                    ox + x,
                    oy + y,
                    16.0,
                    16.0,
                    rgba(255, 255, 255, 128),
                );
            }
            if let Some(slot) = self.slot_at(batch, self.cursor.0, self.cursor.1) {
                let (x, y) = self.slot_position(slot);
                batch.fill(
                    &self.gui.atlas,
                    ox + x,
                    oy + y,
                    16.0,
                    16.0,
                    rgba(255, 255, 255, 128),
                );
            }
        } else {
            let items = self.creative_screen.items(&self.pack.names);
            let first = self.creative_screen.first_row(&self.pack.names);
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
                        batch.fill(&self.gui.atlas, x, y, 16.0, 16.0, rgba(255, 255, 255, 128));
                        hovered = Some(stack.clone());
                    }
                }
            }
            let drag = self.drag_view();
            for c in 0..9 {
                let (x, y) = (ox + 9.0 + c as f32 * 18.0, oy + 112.0);
                let stack = self.inventory[c].clone();
                self.draw_slot_item(batch, drag.as_ref(), c, stack.clone(), x, y);
                if self.cursor.0 >= x
                    && self.cursor.0 < x + 16.0
                    && self.cursor.1 >= y
                    && self.cursor.1 < y + 16.0
                {
                    batch.fill(&self.gui.atlas, x, y, 16.0, 16.0, rgba(255, 255, 255, 128));
                    hovered = stack;
                }
            }
            let scrollable = self.creative_screen.rows(&self.pack.names) > 5;
            let scroller = if scrollable {
                "gui/sprites/container/creative_inventory/scroller"
            } else {
                "gui/sprites/container/creative_inventory/scroller_disabled"
            };
            batch.sprite(
                &self.gui.atlas,
                scroller,
                ox + Creative::SCROLL_X,
                oy + Creative::SCROLL_Y + self.creative_screen.scroller_offset(),
            );
        }
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
        self.draw_carried(batch);
        if tab.inventory {
            if Self::creative_trash_hovered(batch, self.cursor) {
                self.tooltip(batch, "Destroy Item", self.cursor.0, self.cursor.1);
            } else {
                self.draw_slot_tooltip(batch);
            }
        } else if self.carried.is_none()
            && let Some(item) = hovered
        {
            let lines = self.stack_tooltip(&item);
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
        self.draw_inventory_effects(batch, (ox, oy), Creative::WIDTH as i32);
    }
    fn creative_trash_hovered(batch: &HudBatch, cursor: (f32, f32)) -> bool {
        let (ox, oy) = Self::creative_origin(batch);
        let (x, y) = Creative::TRASH_POSITION;
        cursor.0 >= ox + x
            && cursor.0 < ox + x + 16.0
            && cursor.1 >= oy + y
            && cursor.1 < oy + y + 16.0
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
                select_creative_tab(&mut self.creative_screen, &mut self.inv_clicks, i);
                self.inv_drag = None;
                self.last_inv_click = None;
                return;
            }
        }
        if self.creative_inventory_open() {
            if Self::creative_trash_hovered(&batch, self.cursor) {
                if self.has_shift_down() {
                    let before = self.player_equipment();
                    self.inventory.fill(None);
                    self.armor.fill(None);
                    self.offhand = None;
                    self.craft_grid[..4].fill(None);
                    self.player_equipment_changed(&before);
                    self.apply_equipment();
                } else {
                    self.carried = None;
                }
                self.inv_drag = None;
                self.inv_clicks = Default::default();
            } else if self.slot_at(&batch, cx, cy).is_some() || self.clicked_outside(&batch) {
                self.inventory_press(right);
            }
            return;
        }
        // Holding the scrollbar scrolls as the mouse moves (mouseDragged).
        if !right
            && self
                .creative_screen
                .press_scrollbar(cx - ox, cy - oy, &self.pack.names)
        {
            return;
        }
        if self.slot_at(&batch, cx, cy).is_some() || self.clicked_outside(&batch) {
            self.inventory_press(right);
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
        level.respawn_elsewhere = self.respawn_config.map(|c| c.saved());
        level.rotation = [self.yaw, self.pitch];
        level.xp_level = self.experience.level;
        level.xp_progress = self.experience.progress;
        level.xp_total = self.experience.total;
        if let Some(p) = &self.player {
            level.position = Some([p.position.x, p.position.y, p.position.z]);
            level.health = p.health.health();
            level.absorption = p.health.absorption();
            let extra = level
                .player_extra
                .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
            if let horizon_world::nbt::Tag::Compound(map) = extra {
                map.insert(
                    "Fire".into(),
                    horizon_world::nbt::Tag::Short(
                        p.fire_ticks.clamp(i32::from(i16::MIN), i32::from(i16::MAX)) as i16,
                    ),
                );
                map.insert("Air".into(), horizon_world::nbt::Tag::Short(p.air as i16));
            }
            level.fall_distance = p.fall_distance;

            level.food = p.health.food.food;
            level.saturation = p.health.food.saturation;
            level.flying = p.flying;
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
        level.active_effects = self.potions.effects.saved();
    }
    fn save_game(&mut self) -> Result<()> {
        self.save_game_with(true)
    }
    /// The autosave doesn't `wait` for the chunks to be on disk: every
    /// generated chunk is saved, hundreds at a time, encoded and written on
    /// the region writer meanwhile.
    fn save_game_with(&mut self, wait: bool) -> Result<()> {
        if self.remote.is_some() {
            return Ok(());
        }
        self.save_level(wait)?;
        self.save_parked_levels(wait)?;
        self.save_game_metadata()
    }
    fn save_game_metadata(&mut self) -> Result<()> {
        self.save_maps();
        self.save_random_sequences();
        self.save_fight();
        self.save_wandering_trader();
        self.save_portal_arrivals();
        self.save_stats();
        self.save_advancements();
        self.save_scoreboard();
        self.save_bossbars();
        self.save_schedule();
        self.save_warden_tracker();
        self.sync_level();
        self.store_owner();
        self.level.write(&self.world_root)?;
        self.save_world_state();
        self.message = Some(("Saved the game".into(), Instant::now()));
        Ok(())
    }
    /// Save the level in place: its changed chunks, its entities and its raids.
    fn save_level(&mut self, wait: bool) -> Result<()> {
        self.streaming.save(&mut self.world, wait);
        let keys: Vec<_> = self.world.modified.drain().collect();
        let chunks = self.with_entities(keys.iter().filter_map(|k| self.world.chunks.get(k)));
        self.save
            .save_later(&chunks, self.pack.assets.registry, self.res.biomes())?;
        // Prepare entities while the chunk writer runs, then use one barrier.
        self.save_entities(false)?;
        self.save_raids();
        if wait {
            self.save.flush()?;
        }
        Ok(())
    }
    fn render(&mut self) -> Result<bool> {
        if let Some(loader) = &self.world_loader {
            if let Some(renderer) = &mut self.renderer {
                let mut hud = HudBatch::new([renderer.config.width, renderer.config.height]);
                horizon_client::level_loading::draw_progress(
                    &mut hud,
                    &self.gui.atlas,
                    if loader.cancelled {
                        "Cancelling world"
                    } else {
                        "Opening world"
                    },
                    0.0,
                );
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        if self.pending_entry.is_some() {
            if let (Some(renderer), Some(loader)) = (&mut self.renderer, &self.rest_loader) {
                let mut hud = HudBatch::new([renderer.config.width, renderer.config.height]);
                horizon_client::level_loading::draw_progress(
                    &mut hud,
                    &self.gui.atlas,
                    loader.progress.line(),
                    loader.progress.fraction(),
                );
                return renderer.render(&loading_frame(&hud));
            }
            return Ok(false);
        }
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return Ok(false);
        };
        let alpha = self.clock.fraction();
        // Camera.getCameraEntityPartialTicks: the view, the hand and the
        // player's own model stay smooth while the level is frozen.
        let camera_alpha = self.clock.camera_fraction();
        let view = self.camera_view(camera_alpha as f32);
        let (camera, underwater, view_yaw, view_pitch) = match view {
            Some((c, yaw, pitch)) => {
                let s =
                    self.world
                        .block(c.x.floor() as i32, c.y.floor() as i32, c.z.floor() as i32);
                (c, self.res.facts().get(s).fluid == Fluid::Water, yaw, pitch)
            }
            None => (
                glam::DVec3::new(0.5, 100.0, 0.5),
                false,
                self.yaw,
                self.pitch,
            ),
        };
        self.audio
            .set_listener([camera.x, camera.y, camera.z], view_yaw, view_pitch);
        let block = [
            camera.x.floor() as i32,
            camera.y.floor() as i32,
            camera.z.floor() as i32,
        ];
        // The camera inside an opaque block sees nothing through it: disable.
        let cave_culling = self.settings.horizon.cave_culling
            && !self
                .res
                .facts()
                .get(self.world.block(block[0], block[1], block[2]))
                .solid_render;
        // The cave culling search runs on its worker while the frame is prepared.
        if cave_culling
            && self.title.is_none()
            && let Some(renderer) = self.renderer.as_mut()
        {
            horizon_client::spans::time("occlusion", || {
                renderer.start_occlusion(camera, (self.streaming.radius * 16) as f32)
            });
        }
        let biome = self
            .world
            .chunks
            .get(&(block[0] >> 4, block[2] >> 4))
            .map(|c| c.biome(block[0], block[1], block[2]))
            .unwrap_or(0);
        let environment = self.dim_environment();
        // EnvironmentAttributeProbe: the spatially interpolated biome colours,
        // Gaussian-weighted over the noise biomes around the camera (plains
        // where no chunk is loaded, as ClientLevel.getUncachedNoiseBiome).
        let biomes = self.res.biomes();
        let weights = {
            let chunks = &self.world.chunks;
            let plains = biomes.id("minecraft:plains");
            horizon_client::biome::gaussian_weights(camera.to_array(), |qx, qy, qz| {
                chunks
                    .get(&(qx >> 2, qz >> 2))
                    .map_or(plains, |c| c.biome(qx * 4, qy * 4, qz * 4))
            })
        };
        let blend = |pick: fn(&horizon_client::biome::BiomeInfo) -> Option<u32>, base: [f32; 3]| {
            horizon_client::biome::blend_rgb(&weights, |b| {
                pick(biomes.get(b)).map_or(base, horizon_client::biome::rgb)
            })
        };
        let biome_sky = blend(
            |b| b.sky,
            environment.base_rgb("minecraft:visual/sky_color"),
        );
        let biome_fog = blend(
            |b| b.fog,
            environment.base_rgb("minecraft:visual/fog_color"),
        );
        // visual/water_fog_color: no dimension sets it, its default is #050533.
        let water_fog = blend(|b| Some(b.water_fog), [0.0; 3]);
        let mut sky = environment.sample(self.world.day_time, Some(biome_sky), Some(biome_fog));
        let (rain, thunder) = if self.dim == Dim::Overworld {
            self.world.weather.levels(alpha as f32)
        } else {
            (0.0, 0.0)
        };
        horizon_client::environment::apply_weather(&mut sky, rain, thunder);
        self.end_flash_sky(&mut sky, alpha);
        if self.flash > 0 && !self.settings.file.bool("hideLightningFlashes") {
            // Lightning flash brightens the sky (LevelRenderer skyFlashTime).
            sky.sky_color = [0.9, 0.9, 1.0];
            sky.sky_light_factor = 1.0;
        }
        // FishingHookRenderer's line, then DebugRenderer: chunk borders and
        // hitboxes (F3+G, F3+B).
        let mut lines = self.fishing_line(alpha, size);
        let mut on_top = Vec::new();
        if self.show_hitboxes || self.show_chunk_borders {
            let g = self.debug_gizmos(alpha as f32);
            lines.extend(g.lines);
            on_top = g.on_top;
        }
        let view = horizon_client::camera::view_matrix(view_yaw, view_pitch);
        let at = camera.to_array();
        // GameRenderer.render3dHud: the debug crosshair, first person with the HUD shown.
        let mut crosshair = horizon_client::gizmos::Gizmos::default();
        if self.debug_crosshair() && self.camera_type.first_person() && !self.ui.hide_gui {
            let scale = HudBatch::new(size).scale;
            crosshair.debug_crosshair(at, view_pitch, view_yaw, scale);
        }
        let debug_lines = horizon_client::gizmos::camera_relative(&lines, at, view);
        let debug_lines_on_top = horizon_client::gizmos::camera_relative(&on_top, at, view);
        let debug_crosshair = horizon_client::gizmos::camera_relative(&crosshair.on_top, at, view);
        // LevelExtractor.extractBlockOutline: model translucency controls draw order.
        let mut outline_after_terrain = false;
        let outline = if self.screen == Screen::Playing && !self.spectator() {
            self.pick()
                .map(|h| {
                    let state = self.world.block(h.pos[0], h.pos[1], h.pos[2]);
                    let f = self.res.facts().get(state);
                    outline_after_terrain = self
                        .pack
                        .render
                        .model_catalog
                        .collect(state, BlockPos::new(h.pos[0], h.pos[1], h.pos[2]))
                        .is_ok_and(|models| {
                            models
                                .iter()
                                .any(|(_, model)| model.faces.iter().any(|f| f.translucent))
                        });
                    let o = if f.outline_offset {
                        self.pack
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
        // LevelLoadingScreen (Reason.OTHER) draws the title's panorama.
        let title = self.title.is_some()
            || self.loading_reason() == Some(horizon_client::level_loading::Reason::Other);
        let (yaw, pitch) = if title {
            // PanoramaRenderer: the title cube spins at 0.1 degrees per tick, pitched 10 degrees.
            (
                self.panorama_rotation.update(
                    self.started.elapsed().as_secs_f32(),
                    self.settings.file.double("panoramaScrollSpeed") as f32,
                ),
                10.0,
            )
        } else {
            (view_yaw, view_pitch)
        };
        let fov_degrees = if title {
            85.0
        } else {
            self.fov_degrees(camera_alpha as f32, camera.to_array())
        };
        self.locator_camera =
            (!title).then_some((camera.to_array(), view_yaw, view_pitch, fov_degrees));
        let bob = if title {
            glam::Mat4::IDENTITY
        } else {
            self.bob_matrix(camera_alpha as f32)
        };
        let terrain_distance = self.terrain_distance();
        let distance = (terrain_distance * 16) as f32;
        let depth_far =
            horizon_client::camera::depth_far(terrain_distance, self.settings.cloud_range);
        self.view_frustum = self
            .renderer
            .as_ref()
            .map(|r| r.camera_frustum(fov_degrees, yaw, pitch, bob, depth_far));
        // Keep the buffer local while builders need mutable access to the game.
        let mut entities = std::mem::take(&mut self.entity_vertices);
        entities.clear();
        self.see_through_vertices.clear();
        if !title {
            entities = horizon_client::spans::time("entity_geometry", || {
                self.entity_geometry(camera, alpha, entities)
            });
        }
        let bolts = if title {
            Vec::new()
        } else {
            self.bolt_instances()
        };
        let mut additive = Vec::new();
        if !title && self.dim == Dim::End {
            let mut dragon = Vec::new();
            self.dragon_geometry(camera, alpha, &mut dragon, &mut additive);
            self.dragon_fireball_geometry(camera, alpha, &mut dragon);
            entities.extend(dragon);
        }
        let (celestial, stars) = if title {
            (Vec::new(), None)
        } else if self.dim == Dim::Overworld
            && self.potions.effects.get("darkness").is_none()
            && self.potions.effects.get("blindness").is_none()
        {
            // Camera.doesMobEffectBlockSky: no sun, moon or stars.
            self.celestial_geometry(&sky)
        } else if self.dim == Dim::End && self.settings.horizon.sky {
            (self.end_flash_geometry(), None)
        } else {
            (Vec::new(), None)
        };
        let hand = if title || self.scoping() {
            Vec::new()
        } else {
            self.hand_geometry(camera_alpha)
        };
        let screen = if title {
            Vec::new()
        } else {
            self.screen_quads(alpha as f32, size)
        };
        if !title {
            self.end_geometry(camera, &mut entities);
        }
        let weather = if !title && self.dim == Dim::Overworld && self.settings.horizon.weather {
            horizon_client::spans::time("weather", || self.weather_geometry(camera, alpha))
        } else {
            Vec::new()
        };
        let border = (!title)
            .then(|| self.border_draw(camera.to_array(), alpha as f32))
            .flatten();
        let see_through = std::mem::take(&mut self.see_through_vertices);
        if title {
            self.dynamic_lights.clear();
        } else {
            self.gather_dynamic_lights(camera, alpha);
        }
        let pack_inputs = if title {
            Default::default()
        } else {
            self.shader_pack_inputs(camera, alpha as f32)
        };
        let frame = Frame {
            weather: &weather,
            weather_time: horizon_client::weather::Time::new(self.world.game_time, alpha as f32),
            nether_lighting: self.dim == Dim::Nether,
            camera,
            yaw,
            pitch,
            fov_degrees,
            bob,
            sky,
            distance,
            detail_distance: (self.streaming.radius * 16) as f32,
            depth_far,
            underwater,
            title,
            skybox: self.dim == Dim::Overworld && self.settings.horizon.sky,
            end_sky: self.dim == Dim::End && self.settings.horizon.sky,
            end_sky_sprite: self
                .gui
                .atlas
                .sprites
                .get("environment/end_sky")
                .map(|r| [r.u0, r.v0, r.u1, r.v1]),
            environment_fog: (self.dim == Dim::Nether).then_some((sky.fog_start, sky.fog_end)),
            cave_culling,
            water_fog,
            fog: Some({
                let mut fog = self.frame_fog(camera, &sky, biome, water_fog);
                // Horizon's fog setting: no distance fog (fluids keep theirs).
                if !self.settings.horizon.fog
                    && !underwater
                    && self.darkness_fog().is_none()
                    && !self.boss_overlay.should_create_world_fog()
                    && self.potions.effects.get("blindness").is_none()
                    && fog.environmental_end >= (self.streaming.radius * 16) as f32
                {
                    (fog.render_distance_start, fog.render_distance_end) = (1e9, 2e9);
                    (fog.environmental_start, fog.environmental_end) = (1e9, 2e9);
                }
                fog
            }),
            dark_disc: self.dark_disc(camera),
            outline,
            outline_after_terrain,
            debug_lines: &debug_lines,
            debug_lines_on_top: &debug_lines_on_top,
            debug_crosshair: &debug_crosshair,
            hud: &hud,
            clouds: (!title
                && self.dim == Dim::Overworld
                && horizon_client::clouds::visible(&self.settings.clouds, sky.cloud_color[3]))
            .then(|| self.gui.atlas.sprites.get("horizon/white"))
            .flatten()
            .map(|white| horizon_client::clouds::Input {
                texture: &self.pack.clouds,
                uv: [(white.u0 + white.u1) / 2.0, (white.v0 + white.v1) / 2.0],
                camera,
                time: self.world.game_time,
                partial_tick: alpha,
                range: self.settings.cloud_range,
                fast: self.settings.clouds == "fast",
                color: sky.cloud_color,
            }),
            entities: &entities,
            dynamic_lights: &self.dynamic_lights,
            lightning: horizon_client::gfx::lightning::Input {
                bolts: &bolts,
                uv: [
                    self.gui.atlas.font.white().u0,
                    self.gui.atlas.font.white().v0,
                ],
            },
            additive: &additive,
            see_through: &see_through,
            celestial: if self.settings.horizon.sky || title {
                &celestial
            } else {
                &[]
            },
            stars: stars.filter(|_| self.settings.horizon.sky || title),
            hand: &hand,
            hand_fov_degrees: self.hud_fov_degrees(camera_alpha as f32, camera.to_array()),
            screen: &screen,
            pack: pack_inputs,
            border,
            game_time: self.world.game_time as f32,
            shader_game_time: ((self.world.game_time % 24000) as f32 + alpha as f32) / 24000.0,
            crumbling_sprite: self
                .gui
                .atlas
                .sprites
                .get("block/destroy_stage_0")
                .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0]),
            end_portal_sprites: ["environment/end_sky", "entity/end_portal/end_portal"].map(|n| {
                self.gui
                    .atlas
                    .sprites
                    .get(n)
                    .map_or([0.0; 4], |r| [r.u0, r.v0, r.u1 - r.u0, r.v1 - r.v0])
            }),
            ticks: self.ticks,
            darkness: self.darkness_lighting(alpha as f32),
            night_vision: self.night_vision_intensity(),
            block_flicker: self.fog.block_light_flicker,
            boss_darken: self
                .boss_overlay
                .darken_amount(self.clock.fraction() as f32),
            menu_blur: self.menu_blur(),
        };
        let result = match self.renderer.as_mut() {
            Some(renderer) => horizon_client::spans::time("renderer", || renderer.render(&frame)),
            None => Err("renderer".into()),
        };
        // Restore storage even when rendering fails (for example, a lost surface).
        self.entity_vertices = entities;
        self.see_through_vertices = see_through;
        result
    }
    /// The LevelLoadingScreen shown while the player waits for its chunks:
    /// opening a world, or after a portal or a respawn into another dimension.
    fn loading_reason(&self) -> Option<horizon_client::level_loading::Reason> {
        use horizon_client::level_loading::Reason;
        if self.title.is_some() || self.remote.is_some() || self.player.is_some() {
            return None;
        }
        Some(
            if self.travelling.is_none() || self.respawn_travel.is_some() {
                Reason::Other
            } else if self.end_travel {
                Reason::EndPortal
            } else {
                Reason::NetherPortal
            },
        )
    }
    /// `LevelLoadTracker.serverProgress`: the player's 7x7 chunks loaded.
    fn level_load_progress(&self) -> f32 {
        let (cx, cz) = self.streaming.center;
        let ready = (-3..=3)
            .flat_map(|dz| (-3..=3).map(move |dx| (cx + dx, cz + dz)))
            .filter(|&(x, z)| self.streaming.is_lit(x, z))
            .count();
        horizon_client::level_loading::progress(ready)
    }
    /// `LevelLoadingScreen.extractBackground` and `extractRenderState`.
    fn draw_level_loading(
        &self,
        batch: &mut HudBatch,
        reason: horizon_client::level_loading::Reason,
    ) {
        use horizon_client::level_loading::{self, Reason, STATUS_RADIUS};
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        match reason {
            // The panorama and its blur are drawn by the renderer.
            Reason::Other => {
                horizon_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false)
            }
            Reason::NetherPortal => {
                let overlay = hud_overlays::portal_overlay(w as i32, h as i32, 1.0);
                hud_layout::paint(batch, atlas, &[overlay]);
            }
            Reason::EndPortal => batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255)),
        }
        let (cx, cz) = self.streaming.center;
        let status = |x: i32, z: i32| {
            self.streaming.latest_status(
                &self.world,
                cx + x - STATUS_RADIUS,
                cz + z - STATUS_RADIUS,
            )
        };
        level_loading::draw(batch, atlas, Some(&status), Some(self.loading_progress));
    }
    /// `Screen.extractBlurredBackground` over the panorama or the world and
    /// HUD, when `menuBackgroundBlurriness` is at least 1.
    fn menu_blur(&self) -> Option<f32> {
        let loading = self.loading_reason() == Some(horizon_client::level_loading::Reason::Other);
        let blurred = loading
            || self.title.as_ref().is_some_and(|t| {
                matches!(self.screen, Screen::Options | Screen::Win)
                    || !matches!(t.mode, horizon_client::title::Mode::Main)
            })
            || (self.title.is_none()
                && self.player.is_some()
                && (self.screen.blurs_background(self.pause_menu)
                    || (self.screen == Screen::Win && self.win_screen_pauses())));
        let radius = self.settings.menu_blur as f32;
        (blurred && radius >= 1.0).then_some(radius)
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
            horizon_client::camera::depth_far(self.terrain_distance(), self.settings.cloud_range);
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
        if self.sleeping.is_some()
            && self.screen == Screen::Playing
            && !matches!(
                key,
                KeyCode::F1 | KeyCode::F2 | KeyCode::F11 | KeyCode::KeyT | KeyCode::Slash
            )
        {
            if matches!(key, KeyCode::Escape | KeyCode::ShiftLeft) {
                self.wake_up();
            }
            return Ok(());
        }
        if self.sort_key_press(key) {
            return Ok(());
        }
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
                Screen::GameModeSwitcher => self.close_gamemode_switcher(false),
                Screen::Dead => {}
                Screen::Stats | Screen::Friends => self.screen = Screen::Paused,
                Screen::Advancements => self.close_advancements(),
                Screen::WorldOptions => self.world_options_escape(),
                Screen::CommandBlock => self.close_command_block(),
                Screen::Chat => self.close_chat(),
                Screen::Win => self.close_win_screen()?,
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
                if (self.screen == Screen::Inventory
                    || (self.screen == Screen::Creative && !self.creative_screen.tab().search))
                    && self.inventory_hotkey(key) => {}
            // The search tab's number keys (scripted presses; the window's
            // keys reach search_tab_hotbar_key before the search box).
            _ if self.screen == Screen::Creative
                && self.creative_screen.tab().search
                && self.search_tab_hotbar_key(key) => {}
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
            // key.swapOffhand: on a server, ServerboundPlayerActionPacket SWAP_ITEM_WITH_OFFHAND
            // (the server swaps the hands and sends both slots back; spectators can't).
            KeyCode::KeyF if self.screen == Screen::Playing && self.remote.is_some() => {
                if !self.spectator() {
                    self.remote_swap_hands();
                }
            }
            KeyCode::KeyF if self.screen == Screen::Playing => {
                // handlePlayerAction SWAP_ITEM_WITH_OFFHAND: setItemInHand for
                // the off hand, then the main hand.
                std::mem::swap(&mut self.offhand, &mut self.inventory[self.selected]);
                let (offhand, main) = (self.offhand.clone(), self.inventory[self.selected].clone());
                self.player_on_equip("offhand", main.as_ref(), offhand.as_ref());
                self.player_on_equip("mainhand", offhand.as_ref(), main.as_ref());
                self.using = None;
            }
            KeyCode::KeyQ if self.screen == Screen::Playing => {
                let all = drop_whole_stack(&self.physical_keys);
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
            | KeyCode::Digit9
                if self.screen == Screen::Playing =>
            {
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
        if self.sleeping.is_some() {
            return Ok(());
        }
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
                // handleKeybinds consumes use clicks without acting while an
                // item is in use.
                if pressed && self.using.is_none() {
                    self.use_with_hand()?;
                    self.use_delay = 4;
                }
            }
            MouseButton::Middle if pressed => {
                // Minecraft.pickBlockOrEntity -> the server's handlePickItemFromBlock:
                // the block's clone item, with its block entity's data on Ctrl in
                // creative, picked from the inventory (`pick_item`).
                if let Some(hit) = self.pick() {
                    let include_data = self.creative && self.has_control_down();
                    if let Some(stack) = self.picked_stack(hit.pos, include_data) {
                        let infinite = self.creative;
                        horizon_client::pick_item::pick(
                            &mut self.inventory[..36],
                            &mut self.selected,
                            &stack,
                            infinite,
                        );
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }
    /// LOD Distance as far terrain uses it (`Graphics::lod_distance_on`).
    fn lod_distance(&self) -> u32 {
        match &self.renderer {
            Some(renderer) => self.settings.horizon.lod_distance_on(&renderer.info),
            None => self.settings.horizon.lod_distance.min(1024),
        }
    }
    /// Only enabled LOD worlds extend the renderer's range, never simulation.
    fn terrain_distance(&self) -> i32 {
        if self.lod.is_some() {
            self.streaming.radius.max(self.lod_distance() as i32)
        } else {
            self.streaming.radius
        }
    }
    fn clear_lod(&mut self) {
        self.lod = None;
        self.lod_source_attempted = false;
        self.lod_observe.clear();
        self.lod_covered_at = None;
        if let Some(renderer) = &mut self.renderer {
            renderer.clear_lods();
        }
    }
    fn prepare_lod(&mut self) {
        let enabled = self.settings.horizon.lod_distance > 0
            && self.renderer.is_some()
            && self.player.is_some()
            && self.title.is_none()
            && (self.remote.is_none() || self.integrated.is_some());
        if !enabled {
            if self.lod.is_some() || self.lod_source_attempted {
                self.clear_lod();
            }
            return;
        }
        if self.lod_source_attempted {
            return;
        }
        self.lod_source_attempted = true;
        let source = match &self.integrated {
            Some(server) => server.call(|game| game.streaming.lod_source()),
            None => Some(self.streaming.lod_source()),
        };
        if let Some(source) = source {
            self.lod = Some(horizon_client::lod::Lod::new(
                source,
                self.pack.context.clone(),
            ));
            self.lod_observe.extend(self.world.chunks.keys().copied());
        }
    }
    fn update_world(&mut self) {
        if let Err(e) = self.poll_remote() {
            log::warn!("Horizon: {e}");
        }
        self.prepare_lod();
        if self.lod.is_some() {
            for &((x, _, z), _, _) in &self.world.light_edits {
                self.lod_observe.insert((x >> 4, z >> 4));
            }
        }
        if let Some(p) = &self.player {
            self.streaming.center = (
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            );
        }
        // The loading screen's chunk map fills ring by ring, as vanilla's.
        self.streaming.loading =
            self.options.execution_role.is_client() && self.loading_reason().is_some();
        let (uploads, unloaded) =
            horizon_client::spans::time("streaming", || self.streaming.update(&mut self.world));
        if let Some(lod) = &mut self.lod {
            self.lod_observe
                .extend(uploads.iter().map(|(key, _)| (key.0, key.2)));
            for chunk in &unloaded {
                self.lod_observe.remove(&(chunk.x, chunk.z));
            }
            // Unloaded columns first (their summaries are all that remains),
            // then a few resident ones (an edit copies a chunk the worker
            // still holds). The worker summarizes them.
            horizon_client::spans::time("lod_observe", || {
                for chunk in &unloaded {
                    lod.observe(chunk.clone(), self.res.facts());
                }
                while lod.observing() < horizon_client::lod::OBSERVE_AHEAD
                    && let Some(&key) = self.lod_observe.iter().next()
                {
                    self.lod_observe.remove(&key);
                    if let Some(chunk) = self.world.chunks.get(&key) {
                        lod.observe(chunk.clone(), self.res.facts());
                    }
                }
            });
        }
        let dirty: Vec<_> = unloaded
            .iter()
            .filter(|c| self.world.modified.remove(&(c.x, c.z)))
            .cloned()
            .collect();
        let modified = horizon_client::spans::time("with_entities", || self.with_entities(&dirty));
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
                    .save_later(&modified, self.pack.assets.registry, self.res.biomes())
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
                // mesh of a waiting section replaces it, and sections of
                // chunks no longer loaded are dropped.
                let pending = &mut self.pending_uploads;
                let fresh: horizon_client::fast_hash::FxHashSet<(i32, i32, i32)> =
                    uploads.iter().map(|(k, _)| *k).collect();
                let chunks = &self.world.chunks;
                pending.retain(|(k, _)| !fresh.contains(k) && chunks.contains_key(&(k.0, k.2)));
                pending.extend(
                    uploads
                        .into_iter()
                        .filter(|(k, _)| chunks.contains_key(&(k.0, k.2))),
                );
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
                    let size = ((mesh.solid.len() + mesh.translucent.len())
                        * std::mem::size_of::<horizon_client::mesher::TerrainVertex>())
                        as u64;
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
            if let (Some(lod), Some(player)) = (&mut self.lod, &self.player) {
                let lod_distance = self.settings.horizon.lod_distance_on(&renderer.info);
                lod.set_near_loading(self.streaming.busy());
                lod.set_reduced_detail(
                    self.settings.horizon.far_detail
                        == horizon_client::graphics::FarDetail::Reduced,
                );
                // The covered columns change only with the complete ones and
                // the render area: rebuilt then, not every frame.
                let at = (
                    renderer.lod_detail_revision(),
                    self.streaming.center,
                    self.streaming.radius,
                );
                let covered = (self.lod_covered_at != Some(at)).then(|| {
                    self.lod_covered_at = Some(at);
                    horizon_client::spans::time("lod_covered_set", || {
                        renderer
                            .lod_detail_columns()
                            .filter(|&(x, z)| {
                                horizon_client::streaming::in_view_distance(at.1, at.2, x, z)
                            })
                            .collect()
                    })
                });
                let meshes = horizon_client::spans::time("lod_update", || {
                    lod.update(
                        [player.position.x, player.position.y, player.position.z],
                        self.yaw,
                        self.streaming.radius,
                        lod_distance,
                        covered,
                    )
                });
                renderer.queue_lods(meshes, lod.removed(), lod.take_cutout());
                // A finished job can hold hundreds of tiles: spread them
                // over frames rather than stall one.
                horizon_client::spans::time("lod_upload", || {
                    renderer.upload_lods(std::time::Duration::from_millis(2))
                });
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
        let size = self.options.window_size.map(f64::from);
        self.fullscreen = horizon_client::window::FullscreenState::new(fullscreen);
        let attributes = Window::default_attributes()
            .with_title("Horizon")
            .with_window_icon(horizon_client::window::icon());
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "horizon", "horizon",
        );
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None))),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
                return;
            }
        };
        horizon_client::telemetry::note_display(&window);
        match self.new_renderer(window.clone()) {
            Ok(r) => {
                self.renderer = Some(r);
                self.world_renderer_ready = true;
            }
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        self.window = Some(window);
        self.apply_settings();
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
        // Fullscreen is a window shortcut, before screens consume keyboard input.
        if let WindowEvent::KeyboardInput { event, .. } = &event
            && event.state == ElementState::Pressed
            && let PhysicalKey::Code(key) = event.physical_key
            && horizon_client::keybinds::key_name(key)
                == Some(self.settings.keys.get("key.fullscreen"))
            && !(self.screen == Screen::Options && self.options_waiting_for_key())
        {
            self.last_input = Instant::now();
            if !event.repeat {
                self.toggle_fullscreen();
            }
            return;
        }
        // KeyboardHandler.keyPress: the debug keys come before the game's.
        if let WindowEvent::KeyboardInput {
            event: key_event, ..
        } = &event
            && let PhysicalKey::Code(key) = key_event.physical_key
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
        if (self.pending_entry.is_some() || self.world_loader.is_some())
            && matches!(&event, WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.physical_key == PhysicalKey::Code(KeyCode::Escape))
        {
            self.pending_entry = None;
            if let Some(loader) = self.world_loader.take() {
                self.world_loader = Some(if loader.cancelled {
                    loader
                } else {
                    loader.cancel()
                });
            }
            self.perf.cancel_world_opening();
            return;
        }
        if (self.pending_entry.is_some() || self.world_loader.is_some())
            && !matches!(
                &event,
                WindowEvent::CloseRequested
                    | WindowEvent::Resized(_)
                    | WindowEvent::RedrawRequested
            )
        {
            return;
        }
        if self.pending_save.is_some() {
            match event {
                WindowEvent::CloseRequested => {
                    self.pending_save.as_mut().unwrap().quit = true;
                }
                WindowEvent::Focused(focused) => {
                    self.focused = focused;
                    if !focused {
                        self.capture(false);
                    }
                }
                WindowEvent::Resized(size) => {
                    if let Some(r) = &mut self.renderer {
                        r.resize(size.width, size.height);
                    }
                }
                WindowEvent::RedrawRequested => {
                    if let Err(error) = self.render_closing_save() {
                        self.error = Some(error);
                        event_loop.exit();
                    }
                }
                _ => {}
            }
            return;
        }
        let result: Result<()> = (|| {
            match event {
                WindowEvent::CloseRequested => {
                    self.begin_closing_save(true)?;
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
                        }
                        if matches!(self.screen, Screen::Inventory | Screen::Creative) {
                            self.inventory_drag_move();
                        }
                        if let Some(title) = &mut self.title {
                            title.drag(self.cursor, (b.width, b.height));
                            title.drag_field(&self.gui.atlas, self.cursor.0);
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
                    if let Some(title) = &mut self.title
                        && self.screen != Screen::Win
                    {
                        let size = self
                            .window
                            .as_ref()
                            .map(|w| w.inner_size())
                            .unwrap_or_default();
                        let batch = HudBatch::new([size.width, size.height]);
                        title.scroll_worlds(d, self.cursor, (batch.width, batch.height));
                    }
                    if self.screen == Screen::Playing
                        && self.sleeping.is_none()
                        && self.player.is_some()
                        && !self.extras_scroll(d)
                        && let Some((wheel, (_, vertical_steps))) =
                            self.mouse
                                .on_scroll(raw_x, raw_y, discrete, wheel_sensitivity)
                    {
                        if self.spectator() {
                            if let Some(player) = &mut self.player {
                                player.flying_speed = horizon_client::mouse::spectator_flying_speed(
                                    player.flying_speed,
                                    vertical_steps,
                                );
                            }
                        } else {
                            self.selected =
                                horizon_client::mouse::ScrollWheelHandler::next_selection(
                                    f64::from(wheel),
                                    self.selected as i32,
                                    9,
                                ) as usize;
                        }
                    }
                    if self.screen == Screen::Chat && d != 0.0 {
                        self.chat_scroll(d);
                    }
                    if self.screen == Screen::Options && d != 0.0 {
                        self.options_scroll(d);
                    }
                    if self.screen == Screen::Creative && d != 0.0 {
                        self.creative_screen.wheel(f64::from(d), &self.pack.names);
                    }
                    if self.screen == Screen::Inventory
                        && d != 0.0
                        && !self.tweak_scroll(d, self.inventory_target().0)
                    {
                        self.station_scroll(d);
                    }
                    if self.screen == Screen::Stats && d != 0.0 {
                        self.stats_scroll(d);
                    }
                    if self.screen == Screen::WorldOptions && d != 0.0 {
                        self.world_options_scroll(d);
                    }
                    if self.screen == Screen::Advancements {
                        self.advancements_scroll(scaled(raw_x) as f32, d);
                    }
                }
                WindowEvent::MouseInput { state, button, .. }
                    if self.server_packs.prompt.is_some() =>
                {
                    if state == ElementState::Pressed && button == MouseButton::Left {
                        self.pack_prompt_click()?;
                    }
                }
                WindowEvent::MouseInput { .. } if self.screen == Screen::Win => {}
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
                    state: ElementState::Released,
                    button: MouseButton::Left,
                    ..
                } if self.title.is_some() && self.screen != Screen::Options => {
                    if let Some(title) = &mut self.title {
                        title.release();
                        title.release_field();
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
                    let b = HudBatch::new(size);
                    let atlas = &self.gui.atlas;
                    let shift = self.has_shift_down();
                    let (action, sound) = self.title.as_mut().map_or((Action::None, false), |t| {
                        let clicked = t.click(self.cursor.0, self.cursor.1, (b.width, b.height));
                        t.press_field(atlas, self.cursor, (b.width, b.height), shift);
                        clicked
                    });
                    self.save_title_options();
                    if sound {
                        self.audio.play_button_click();
                    }
                    match action {
                        Action::Quit => event_loop.exit(),
                        Action::Options => self.open_options(true),
                        Action::Accessibility => self
                            .open_options_at(true, horizon_client::options_ui::Page::Accessibility),
                        Action::Language => {
                            self.open_options_at(true, horizon_client::options_ui::Page::Language)
                        }
                        Action::Play { folder } => {
                            self.open_world(&folder, None, None, false, None)?
                        }
                        Action::Create {
                            folder,
                            name,
                            seed,
                            creative,
                            options,
                        } => {
                            self.open_world(&folder, Some(&name), seed, creative, Some(&options))?
                        }
                        Action::Connect { address } => {
                            self.connect_server(&address, &self.options.player_name())?
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
                    if matches!(self.screen, Screen::Inventory | Screen::Creative)
                        && let Some(key) = self
                            .settings
                            .keys
                            .container_mouse_key(button, self.creative)
                    {
                        if pressed && self.carried.is_none() {
                            self.inventory_hotkey(key);
                        }
                        // Neither edge of a bound button is an ordinary slot click.
                        return Ok(());
                    }
                    if pressed && self.sort_mouse_press(button) {
                        return Ok(());
                    }
                    match self.screen {
                        Screen::Playing => {
                            if pressed && !self.captured {
                                self.capture(true);
                                return Ok(());
                            }
                            self.extras_mouse(button, pressed);
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
                        Screen::Inventory | Screen::Creative
                            if !matches!(button, MouseButton::Left | MouseButton::Right) =>
                        {
                            // AbstractContainerScreen: another button clicks
                            // a slot only as the pick block key (its hotbar
                            // keys were handled above).
                            if self.settings.keys.get("key.pickItem")
                                == horizon_client::keybinds::mouse_name(button)
                            {
                                if pressed {
                                    self.inventory_pick_press();
                                } else {
                                    self.inventory_pick_release();
                                }
                                if let Some(key) = self.container {
                                    self.container_changed(key);
                                }
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
                        Screen::Creative => {
                            if button == MouseButton::Left {
                                self.creative_screen.release();
                            }
                            self.inventory_mouse_release(button == MouseButton::Right);
                        }
                        Screen::Paused
                            if pressed && button == MouseButton::Left && self.pause_menu =>
                        {
                            self.pause_click()?
                        }
                        Screen::WorldOptions if pressed && button == MouseButton::Left => {
                            self.world_options_click()
                        }
                        Screen::Friends if pressed && button == MouseButton::Left => {
                            self.friends_screen_click()
                        }
                        Screen::CommandBlock if pressed && button == MouseButton::Left => {
                            self.command_block_click()
                        }
                        Screen::Dead if pressed && button == MouseButton::Left => {
                            self.death_click()?
                        }
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
                // ConfirmScreen.keyPressed: Escape answers no.
                WindowEvent::KeyboardInput { event, .. } if self.server_packs.prompt.is_some() => {
                    if event.state == ElementState::Pressed
                        && event.physical_key == PhysicalKey::Code(KeyCode::Escape)
                    {
                        self.answer_pack_prompt(false)?;
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
                    if let PhysicalKey::Code(key) = event.physical_key
                        && self.search_tab_hotbar_key(key)
                    {
                        // The hovered slot took the key (`ignoreTextInput`).
                    } else if let Some(shortcut) = self.edit_shortcut(&event) {
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
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::WorldOptions
                        && !matches!(event.physical_key, PhysicalKey::Code(KeyCode::Escape)) =>
                {
                    self.world_options_key(&event)
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::Win => {
                    self.win_key(&event)?
                }
                WindowEvent::KeyboardInput { event, .. } if self.screen == Screen::CommandBlock => {
                    self.command_block_key(&event)
                }
                // Options consumes presses; releases still reach the bound
                // gameplay mapping below, including sneak/sprint toggles.
                WindowEvent::KeyboardInput { event, .. }
                    if self.screen == Screen::Options && event.state == ElementState::Pressed =>
                {
                    if let Some(shortcut) = self.edit_shortcut(&event)
                        && self.options_shortcut(shortcut)
                    {
                        return Ok(());
                    }
                    if let PhysicalKey::Code(key) = event.physical_key {
                        self.options_key(key, event.text.as_deref())?;
                    }
                }
                WindowEvent::KeyboardInput { event, .. } if self.title.is_some() => {
                    if event.state == ElementState::Pressed {
                        if let Some(shortcut) = self.edit_shortcut(&event) {
                            self.title_shortcut(shortcut);
                            return Ok(());
                        }
                        match event.physical_key {
                            PhysicalKey::Code(KeyCode::Enter | KeyCode::NumpadEnter) => {
                                if let Some(t) = self.title.as_mut() {
                                    t.confirm_edit();
                                }
                            }
                            PhysicalKey::Code(
                                key @ (KeyCode::Backspace
                                | KeyCode::Delete
                                | KeyCode::ArrowLeft
                                | KeyCode::ArrowRight
                                | KeyCode::Home
                                | KeyCode::End),
                            ) if self.title.as_mut().is_some_and(|t| t.has_caret_field()) => {
                                let (ctrl, shift) =
                                    (self.has_control_down(), self.has_shift_down());
                                if let Some(t) = self.title.as_mut() {
                                    t.field_key(key, ctrl, shift);
                                }
                            }
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
                                self.save_title_options();
                            }
                            _ => self.title_edit(event.text.as_deref(), false),
                        }
                    }
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    let PhysicalKey::Code(key) = event.physical_key else {
                        return Ok(());
                    };
                    self.extras_key(key, event.state == ElementState::Pressed, event.repeat);
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
                    self.mapped_key(key, event.state == ElementState::Pressed, event.repeat)?;
                }
                WindowEvent::RedrawRequested => {
                    self.observe_fullscreen();
                    let now = Instant::now();
                    let frame_cpu = self
                        .options
                        .spikes
                        .and_then(|_| horizon_client::spans::thread_cpu());
                    let elapsed = now.duration_since(self.previous);
                    self.frame_ticks = elapsed.as_secs_f32() * 20.0;
                    self.previous = now;
                    // Arrow-key camera controls belong to gameplay; chat and
                    // other screens use the same keys for their own navigation.
                    if self.screen == Screen::Playing
                        && self.title.is_none()
                        && self.sign_edit.is_none()
                        && self.book.is_none()
                    {
                        let turn = |k| f32::from(u8::from(self.keys.contains(&k)));
                        let secs = elapsed.as_secs_f32().min(0.1);
                        self.yaw +=
                            (turn(KeyCode::ArrowRight) - turn(KeyCode::ArrowLeft)) * 90.0 * secs;
                        self.pitch = (self.pitch
                            + (turn(KeyCode::ArrowDown) - turn(KeyCode::ArrowUp)) * 90.0 * secs)
                            .clamp(-90.0, 90.0);
                    }
                    if self.title.is_none() {
                        horizon_client::spans::time("update_world", || self.update_world());
                        horizon_client::spans::time("spawn_player", || self.spawn_player())?;
                    }
                    let updated = Instant::now();
                    self.track_toggle_screens();
                    // For the "I'm playing" ping: an atomic, sent on when it changes.
                    horizon_client::heartbeat::set_lan(self.is_published());
                    self.update_discord_presence();
                    let steps = self.clock.advance(elapsed);
                    // The world ticking in the frame: once its ticks took a
                    // tick's length, the rest are dropped (`FixedStep::tick_fits`).
                    let in_frame = self.remote.is_none();
                    let mut spent = Duration::ZERO;
                    for step in 0..steps {
                        if in_frame && !self.clock.tick_fits(spent) {
                            if let Some(warning) = self.clock.drop_ticks(steps - step) {
                                log::warn!("{warning}");
                            }
                            break;
                        }
                        let started = Instant::now();
                        if self.title.is_none() && !self.pause {
                            let tick = Instant::now();
                            horizon_client::spans::time("tick", || self.tick())?;
                            // In single player the tick is the integrated server's.
                            if self.remote.is_none() {
                                self.perf.tick(tick.elapsed().as_secs_f32() * 1000.0);
                                self.debug_charts.log_tick(tick, tick.elapsed());
                            }
                            self.tick_ambience();
                            self.tick_fov();
                        }
                        self.tick_win_screen()?;
                        self.tick_music();
                        self.tick_sound_engine();
                        self.clock.record(started.elapsed());
                        spent += started.elapsed();
                        if let Some(report) = self.clock.take_sprint_report() {
                            self.sprint_report(&report);
                        }
                    }
                    self.extras_frame(elapsed);
                    self.mods_frame(elapsed);
                    self.extras_retry()?;
                    self.apply_mouse_look();
                    self.pad_look(elapsed.as_secs_f32());
                    self.advance_flight();
                    let ticked = Instant::now();
                    let rendered = self.render()?;
                    if let Some(threshold) = self.options.spikes
                        && let Some(r) = self.renderer.as_ref()
                    {
                        let cpu = now.elapsed().saturating_sub(r.last_wait).as_secs_f32() * 1000.0;
                        let spans = horizon_client::spans::take();
                        if cpu > threshold {
                            // The render thread's own CPU time in the frame:
                            // far below the wall time, the frame waited.
                            let worked = horizon_client::spans::thread_cpu()
                                .zip(frame_cpu)
                                .map_or_else(String::new, |(end, start)| {
                                    format!(", cpu {:.1} ms", (end - start).as_secs_f32() * 1000.0)
                                });
                            log::info!(
                                "Horizon: slow frame {cpu:.1} ms{worked} ({steps} ticks): {}",
                                horizon_client::spans::describe(&spans)
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
                    if let Some(server) = &self.integrated {
                        server.set_paused(self.pause);
                        server.set_distances(
                            self.settings.render_distance,
                            self.settings.simulation_distance,
                        );
                    }
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
                        self.debug_charts.frame_times.log(elapsed.as_nanos() as i64);
                        if self.frames == 0 {
                            // The launcher times Play to this line (launcher/src/perf.rs).
                            log::info!(
                                "Horizon: first frame after {} ms",
                                self.started.elapsed().as_millis()
                            );
                        }
                        if self.title.is_some() {
                            self.perf.title_shown();
                        } else if self.screen == Screen::Playing && self.player.is_some() {
                            self.perf.frame(elapsed.as_secs_f32() * 1000.0);
                            self.world_frames += 1;
                        }
                        self.frames += 1;
                        self.fps_frames += 1;
                        horizon_client::crash::debug_crash_tick();
                        if self.fps_at.elapsed() >= Duration::from_secs(1) {
                            self.fps = self.fps_frames as f32 / self.fps_at.elapsed().as_secs_f32();
                            self.fps_frames = 0;
                            self.fps_at = Instant::now();
                        }
                        if self.options.frames.is_some_and(|f| self.frames >= f)
                            || self
                                .options
                                .world_frames
                                .is_some_and(|f| self.world_frames >= f)
                        {
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
        if self.pending_save.is_none()
            && self.captured
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
        if self.pending_save.is_some() {
            match self.poll_closing_save() {
                Ok(true) => event_loop.exit(),
                Ok(false) => {
                    if let Some(window) = &self.window {
                        window.request_redraw();
                    }
                    // Yield between polls even when disk I/O is slow.
                    event_loop.set_control_flow(ControlFlow::WaitUntil(
                        Instant::now() + Duration::from_millis(4),
                    ));
                }
                Err(error) => {
                    self.error = Some(error);
                    event_loop.exit();
                }
            }
            return;
        }
        if let Err(error) = self.resume_world_entry() {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        self.tick_server_packs();
        self.poll_control(event_loop);
        if self.pending_save.is_some() {
            return;
        }
        self.poll_pads(event_loop);
        if self.options.frames.is_some() && self.started.elapsed() > self.options.timeout {
            self.error = Some("frame-limited run timed out".into());
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window {
            if self.options.offscreen {
                // Offscreen frames don't wait for the window to be drawn: an
                // occluded window (a locked macOS session) gets no redraws.
                let id = w.id();
                self.window_event(event_loop, id, WindowEvent::RedrawRequested);
            } else {
                w.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
    /// The application is ending. On macOS, Quit (Cmd+Q) ends it through
    /// applicationWillTerminate without a CloseRequested; vanilla's GLFW
    /// window turns it into a window close, so Minecraft.stop saves the world.
    fn exiting(&mut self, _event_loop: &ActiveEventLoop) {
        self.pad_close_output();
        if let Err(e) = self.save_for_exit() {
            log::warn!("Horizon: failed to save the world on exit: {e}");
        }
        self.finish_perf();
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
impl<'a> Game<'a> {
    /// A game with no world open, over the loaded resources: the window's,
    /// or an integrated server's on its own thread.
    fn new(
        res: &'a Arc<Resources>,
        pack: PackResources,
        gui: UiResources,
        options: Options,
        settings: Settings,
        audio: Audio,
        prepared: Option<GameCpuResources>,
    ) -> Result<Self> {
        let root = options.resources.clone();
        let registry = pack.assets.registry;
        // Until a world is opened the streamer is idle.
        let mut streaming = Streaming::new(
            Arc::new(TestTerrain::new(registry, res.biomes())?),
            pack.context.clone(),
            1,
        );
        streaming.paused = true;
        let level = Level::default();
        let save = Arc::new(SaveDir::new(
            std::env::current_dir()?
                .join("saves")
                .join(".horizon-title"),
        ));
        let prepared = prepared
            .map(Ok)
            .unwrap_or_else(|| GameCpuResources::load(&root))?;
        let world = World::new(res.facts().air, 0);
        let creative = options.creative;
        let inventory = vec![None; 36];
        let now = Instant::now();
        let look = options.look;
        let particles = horizon_client::particles::ParticleEngine::new(
            &root,
            &gui.atlas.sprites,
            &gui.atlas.translucent,
        );
        Ok(Game {
            res: res.clone(),
            rest_loader: None,
            pending_entry: None,
            world_loader: None,
            pending_pack_reload: false,
            server_packs: Default::default(),
            world_renderer_ready: false,
            collision: res.collision(),
            motion: res.motion(),
            pack,
            gui,
            fog: Default::default(),
            screen_fx: Default::default(),
            weather_parameters: Default::default(),
            panorama_rotation: Default::default(),
            world,
            streaming,
            lod: None,
            lod_source_attempted: false,
            lod_observe: Default::default(),
            lod_covered_at: None,
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
            craft_table: None,
            inv_drag: None,
            tweaks: Default::default(),
            inv_clicks: Default::default(),
            last_inv_click: None,
            last_quick_moved: None,
            container: None,
            station: None,
            potions: Default::default(),
            scratch_slot: None,
            mining: None,
            destroying: HashMap::new(),
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
            entity_vertices: Vec::new(),
            dynamic_lights: Vec::new(),
            luminance: None,
            see_through_vertices: Vec::new(),
            health_hud: hud_layout::HealthHud::default(),
            started: now,
            previous: now,
            last_input: now,
            pad: Default::default(),
            frames: 0,
            world_frames: 0,
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
            cart_sounds: HashSet::new(),
            riding_cart_sounds: None,
            dropped: Vec::new(),
            next_entity: 0,
            particles,
            pickups: Default::default(),
            sign_edit: None,
            book: None,
            lectern_pulses: Vec::new(),
            loading_progress: 0.0,
            spawn_origin: None,
            spawn_spiral: Default::default(),
            player_spawn: None,
            mobs: Vec::new(),
            creatures: Default::default(),
            sculk: Default::default(),
            raiding: Default::default(),
            attack_ticks: 100,
            attack_item: None,
            last_hurt_by_player: HashMap::new(),
            died: HashSet::new(),
            be_anims: {
                let mut anims = horizon_client::block_entity_anim::BlockEntityAnims::default();
                anims.classify(registry);
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
            hand: Hand::Main,
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
            dragging: false,
            avatar: Default::default(),
            flash: 0,
            lightning_seen: [0; 2],
            sleeping: None,
            sleep_timer: 0,
            dim: Dim::Overworld,
            world_root: PathBuf::new(),
            travelling: None,
            server_trips: Vec::new(),
            level_teleports: Vec::new(),
            levels: Default::default(),
            level_pass: Default::default(),
            portals: Default::default(),
            end_travel: false,
            respawn_config: None,
            respawn_travel: None,
            end_portal_travel: false,
            win: None,
            end: Default::default(),
            block_ticks: Ticks::new(0),
            tick_chunks: Default::default(),
            tickets: Default::default(),
            random_tick_sections: Default::default(),
            level_events: Vec::new(),
            falling: Vec::new(),
            paintings: Vec::new(),
            painting_variants: prepared.painting_variants,
            item_frames: Vec::new(),
            frame_model: prepared.frame_model,
            glow_frame_model: prepared.glow_frame_model,
            map_frame_model: prepared.map_frame_model,
            glow_map_frame_model: prepared.glow_map_frame_model,
            armor_stands: Vec::new(),
            others: Vec::new(),
            armor_stand_assets: prepared.armor_stand_assets,
            waypoint_styles: prepared.waypoint_styles,
            waypoints: Default::default(),
            locator_camera: None,
            experience_display: Default::default(),
            boats: Vec::new(),
            boat_models: prepared.boat_models,
            crystal_model: prepared.crystal_model,
            carts: Vec::new(),
            jukebox_sounds: HashMap::new(),
            bolts: Vec::new(),
            maps: maps::Maps::default(),
            fight: dragon::Fight::default(),
            dragon: None,
            crystals: Vec::new(),
            dragon_fireballs: Vec::new(),
            breath_clouds: Vec::new(),
            dragon_model: prepared.dragon_model,
            map_patches: Vec::new(),
            text_displays: Default::default(),
            text_display_vertices: 0,
            skin_downloads: RefCell::new(horizon_client::skins::Downloader::new(
                std::env::current_dir()
                    .unwrap_or_default()
                    .join("assets/skins"),
            )),
            skin_slots: Vec::new(),
            trim_slots: Vec::new(),
            cart_models: prepared.cart_models,
            riding: None,
            ender_items: vec![None; 27],
            ender_open: None,
            pending: Default::default(),
            spill: Default::default(),
            recipe_ui: Default::default(),
            mount_menu: Default::default(),
            hook: None,
            remote_hooks: Vec::new(),
            tridents: Vec::new(),
            rockets: Vec::new(),
            ominous_spawners: Vec::new(),
            creaking: Default::default(),
            spawners: Default::default(),
            trident_model: prepared.trident_model,
            arrow_model: prepared.arrow_model,
            tnts: Vec::new(),
            hoppers: HashMap::new(),
            item_columns: None,
            creative_screen: prepared.creative_screen,
            control: None,
            agent_control_on: false,
            agent_last: None,
            frame_stats: Default::default(),
            perf: Default::default(),
            pending_screenshot: None,
            pending_bench: None,
            flight: None,
            ui_wait: Default::default(),
            events: Default::default(),
            agent: agent::Agent::default(),
            view_frustum: None,
            entity_culler: None,
            pending_uploads: Vec::new(),
            frame_graph: Default::default(),
            extras: Default::default(),
            mods: Default::default(),
            sky_darken_memo: Default::default(),
            persist: Default::default(),
            remote: None,
            lan: None,
            integrated: None,
            serving: None,
            world_options: None,
            options_to_world_options: false,
            command_block_screen: None,
            friends: None,
            debug_keys: Default::default(),
            debug_charts: Default::default(),
            show_hitboxes: false,
            show_chunk_borders: false,
            pause_menu: true,
            previous_mode: None,
            gamemode_switcher: None,
            clipboard: Default::default(),
            text_selection: Default::default(),
            saved_for_exit: false,
            pending_save: None,
            forced_hit: None,
            guest_use: None,
            lan_events: Vec::new(),
            lan_block_events: Vec::new(),
            lan_entity_events: Vec::new(),
            lid_openers: HashMap::new(),
            lan_acting: None,
            guest_packets: Vec::new(),
            lan_acting_name: None,
            lan_acting_uuid: None,
            chat_capture: None,
            admin_feedback: Vec::new(),
            console_broadcast: None,
            cmds: Default::default(),
            ui: Default::default(),
            combat: Default::default(),
            mount: Default::default(),
            stats: Default::default(),
            score: Default::default(),
            fns: Default::default(),
            bosses: Default::default(),
            boss_overlay: Default::default(),
            exec: Default::default(),
            rand_sequences: Default::default(),
            border: Default::default(),
            border_renderer: Default::default(),
            border_state: Default::default(),
            tab_list: Default::default(),
            subtitles: Default::default(),
            adv: Default::default(),
            restore_player_state: true,
            stats_ui: Default::default(),
            adv_ui: Default::default(),
            camera_type: Default::default(),
            fangs: Default::default(),
            player_model: None,
        })
    }
}

/// CPU-only startup data, owned by one loader generation. No file reads or
/// model baking are needed when the event loop installs this bundle.
struct GameCpuResources {
    painting_variants: paintings::PaintingVariants,
    frame_model: frames::FrameModel,
    glow_frame_model: frames::FrameModel,
    map_frame_model: frames::FrameModel,
    glow_map_frame_model: frames::FrameModel,
    armor_stand_assets: armor_stands::ArmorStandAssets,
    waypoint_styles: horizon_client::locator_bar::WaypointStyles,
    boat_models: boats::BoatModels,
    cart_models: minecarts::CartModels,
    crystal_model: Option<horizon_client::mobs::Part>,
    dragon_model: Option<horizon_client::mobs::Part>,
    arrow_model: Option<horizon_client::mobs::Part>,
    trident_model: Option<horizon_client::mobs::Part>,
    creative_screen: Creative,
}
impl GameCpuResources {
    fn load(root: &std::path::Path) -> Result<Self> {
        let document = horizon_client::mobs::EntityModelDocument::load(root).ok();
        let model = |key| document.as_ref().and_then(|document| document.model(key));
        Ok(Self {
            painting_variants: paintings::PaintingVariants::load(root),
            frame_model: frames::bake_frame(root, "item_frame"),
            glow_frame_model: frames::bake_frame(root, "glow_item_frame"),
            map_frame_model: frames::bake_frame(root, "item_frame_map"),
            glow_map_frame_model: frames::bake_frame(root, "glow_item_frame_map"),
            armor_stand_assets: armor_stands::ArmorStandAssets::load(root, document.as_ref()),
            waypoint_styles: horizon_client::locator_bar::WaypointStyles::load(root),
            boat_models: boats::BoatModels::load(document.as_ref()),
            cart_models: minecarts::CartModels::load(document.as_ref()),
            crystal_model: model("minecraft:end_crystal#main"),
            dragon_model: model("minecraft:ender_dragon#main"),
            arrow_model: model("minecraft:arrow#main"),
            trident_model: model("minecraft:trident#main"),
            creative_screen: Creative::load(root)?,
        })
    }
}

/// The data the background loader produces for `Game::new`.
struct Loaded {
    game_cpu: Option<GameCpuResources>,
    renderer_cpu: Option<horizon_client::gfx::PreparedTextures>,
    root: PathBuf,
    pack_error: Option<String>,
    resources: Arc<Resources>,
    rest_loader: Option<WorldRestLoader>,
    pack: PackResources,
    /// The menu bundle, for the synchronous launch; the loading screen receives
    /// it earlier through its own channel.
    gui: Option<UiResources>,
}

/// What the background loader publishes: the menu bundle as soon as it is
/// ready, then the core bundle with a handle to the rest worker.
enum LoaderMsg {
    Ui(UiResources, PathBuf),
    Done(std::result::Result<Loaded, String>),
}

enum PendingWorldEntry {
    Local {
        folder: String,
        name: Option<String>,
        seed: Option<i64>,
        creative: bool,
        create: Option<Box<horizon_client::title::WorldOptions>>,
    },
    Server {
        address: String,
        name: String,
        /// A transfer's carried-over server cookies (`TransferState`).
        cookies: Option<horizon_client::net::Cookies>,
    },
}

struct WorldRestLoader {
    progress: Progress,
    handle: JoinHandle<std::result::Result<(), String>>,
}

impl WorldRestLoader {
    fn ready(&self) -> Result<bool> {
        // No time limit: the title stays usable while this loads, and a slow
        // disk or a debug build can take minutes. A failure ends the thread
        // and is reported by `finish`.
        Ok(self.handle.is_finished())
    }

    fn finish(self) -> Result<()> {
        self.handle
            .join()
            .map_err(|_| "world resource loader panicked")?
            .map_err(Into::into)
    }
}

impl Game<'_> {
    fn defer_world_entry(&mut self, entry: PendingWorldEntry) -> Result<bool> {
        self.poll_world_rest()?;
        if self.rest_loader.is_none() && self.world_loader.is_none() {
            return Ok(false);
        }
        self.perf.world_opening();
        if self.title.is_none() {
            self.title = Some(Title::new(
                std::env::current_dir()?.join("saves"),
                &self.options.resources,
            ));
        }
        self.pending_entry = Some(entry);
        log::info!("Horizon: world entry waiting for world resources");
        Ok(true)
    }

    fn resume_world_entry(&mut self) -> Result<()> {
        self.poll_world_rest()?;
        if let Some(loader) = &self.world_loader {
            if !loader.ready() {
                return Ok(());
            }
            let loader = self
                .world_loader
                .take()
                .ok_or("missing world initialization worker")?;
            let cancelled = loader.cancelled;
            let prepared = loader.finish();
            if !cancelled {
                self.finish_open_world(prepared?)?;
            }
        }
        if self.rest_loader.is_some() {
            return Ok(());
        }
        if std::mem::take(&mut self.pending_pack_reload) {
            self.reload_packs();
        }
        match self.pending_entry.take() {
            Some(PendingWorldEntry::Local {
                folder,
                name,
                seed,
                creative,
                create,
            }) => {
                self.open_world(&folder, name.as_deref(), seed, creative, create.as_deref())?;
            }
            Some(PendingWorldEntry::Server {
                address,
                name,
                cookies,
            }) => {
                self.connect_server_with_cookies(&address, &name, cookies)?;
            }
            None => {}
        }
        Ok(())
    }

    fn poll_world_rest(&mut self) -> Result<()> {
        if let Some(loader) = &self.rest_loader
            && loader.ready()?
        {
            self.rest_loader
                .take()
                .ok_or("missing world loader")?
                .finish()?;
        }
        Ok(())
    }

    fn join_world_rest(&mut self) -> Result<()> {
        if self.rest_loader.is_some() {
            let waited = Instant::now();
            log::info!("Horizon: waiting for world resources");
            while self.rest_loader.is_some() {
                self.poll_world_rest()?;
                if self.rest_loader.is_some() {
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            log::info!(
                "Horizon: world resources joined after {} ms",
                waited.elapsed().as_millis()
            );
        }
        if self.res.rest.get().is_none() {
            return Err("world resources are unavailable".into());
        }
        Ok(())
    }
}

/// `key.hotbar.1` to `key.hotbar.9` in a menu.
const HOTBAR_DIGITS: [KeyCode; 9] = [
    KeyCode::Digit1,
    KeyCode::Digit2,
    KeyCode::Digit3,
    KeyCode::Digit4,
    KeyCode::Digit5,
    KeyCode::Digit6,
    KeyCode::Digit7,
    KeyCode::Digit8,
    KeyCode::Digit9,
];
/// The other keys `InputConstants.Key.getNumericKeyValue` gives a digit.
const NUMERIC_KEYS: [KeyCode; 11] = [
    KeyCode::Digit0,
    KeyCode::Numpad0,
    KeyCode::Numpad1,
    KeyCode::Numpad2,
    KeyCode::Numpad3,
    KeyCode::Numpad4,
    KeyCode::Numpad5,
    KeyCode::Numpad6,
    KeyCode::Numpad7,
    KeyCode::Numpad8,
    KeyCode::Numpad9,
];

const STAGES: u32 = 10;
const STAGE_LINES: [&str; 10] = [
    "Loading menu",
    "Loading assets",
    "Loading block facts",
    "Loading biomes",
    "Loading pack tables",
    "Loading collision",
    "Loading items",
    "Loading recipes",
    "Loading world resources",
    "World resources ready",
];

/// The background loader's checkpoint, read by the loading screen each frame.
#[derive(Clone)]
struct Progress(Arc<AtomicU32>);
impl Progress {
    fn new() -> Self {
        Self(Arc::new(AtomicU32::new(0)))
    }
    fn set(&self, stage: u32) {
        self.0.store(stage, Ordering::Relaxed);
    }
    fn fraction(&self) -> f32 {
        self.0.load(Ordering::Relaxed) as f32 / (STAGES - 1) as f32
    }
    fn line(&self) -> &'static str {
        STAGE_LINES[(self.0.load(Ordering::Relaxed) as usize).min(STAGE_LINES.len() - 1)]
    }
}

/// Harness and dedicated runs keep the fully-synchronous startup so their
/// output stays byte-identical; only an ordinary launch shows the window first.
fn synchronous_startup(options: &Options) -> bool {
    options.frames.is_some()
        || options.world_frames.is_some()
        || options.benchmark.is_some()
        || options.control.is_some()
        || !options.execution_role.is_client()
        || std::env::var_os("HORIZON_DISPENSE_FIXTURE").is_some()
        || std::env::var_os("HORIZON_SCULK_FIXTURE").is_some()
        || std::env::var_os("HORIZON_COMMAND_SCRIPT").is_some()
        || std::env::var_os("HORIZON_NET_REPORT").is_some()
}

/// The synchronous launch loads both phases; an ordinary launch returns the
/// core bundle while the rest worker keeps loading.
fn load_resources(
    mut root: PathBuf,
    vanilla_root: &std::path::Path,
    settings: &Settings,
    started: Instant,
    menu: bool,
    progress: &Progress,
    publish: &mut dyn FnMut(UiResources, &std::path::Path),
) -> Result<Loaded> {
    let requested_root = root.clone();
    let threads = settings.horizon.worker_threads();
    // Stage A: the menu bundle (atlas and panorama). With a loading screen it is
    // published at once so the title screen can appear while the world bundle
    // loads; the synchronous launch instead carries it in `Loaded`.
    let mut loaded_gui = None;
    let mut ui_ready = false;
    let mut pack_error = None;
    let ui = match UiResources::load(&root, vanilla_root) {
        Ok(ui) => Some(ui),
        // Minecraft.rollbackResourcePacks: the packs' textures don't fit, start
        // with the game's own resources.
        Err(e) if root != vanilla_root => {
            log::warn!("Horizon: resource packs failed to load ({e}); using the default resources");
            pack_error = Some(e.to_string());
            root = vanilla_root.to_path_buf();
            None
        }
        Err(_) => None,
    };
    if let Some(ui) = ui {
        if menu {
            horizon_client::telemetry::mark_ui_ready();
            log::info!(
                "Horizon: UI bundle ready after {} ms",
                started.elapsed().as_millis()
            );
            publish(ui, &root);
        } else {
            loaded_gui = Some(ui);
        }
        ui_ready = true;
    }
    progress.set(0);
    let (assets, render) = match Assets::load(&root).and_then(|assets| {
        let render = assets.reload(&root)?;
        Ok((assets, render))
    }) {
        Ok(bundle) => bundle,
        // Minecraft.rollbackResourcePacks: the packs break loading, start
        // with the game's own resources.
        Err(e) if root != vanilla_root => {
            log::warn!("Horizon: resource packs failed to load ({e}); using the default resources");
            pack_error = Some(e.to_string());
            root = vanilla_root.to_path_buf();
            let assets = Assets::load(vanilla_root)?;
            let render = assets.reload(vanilla_root)?;
            (assets, render)
        }
        Err(e) => {
            return Err(format!(
                "{e}\nPrepare resources with the launcher, or: horizon-prepare --prepare"
            )
            .into());
        }
    };
    let assets = Arc::new(assets);
    let render = Arc::new(render);
    // The menu bundle must come from the same root as the world bundle.
    if !ui_ready || root != requested_root {
        let ui = UiResources::load(&root, vanilla_root)?;
        if menu {
            horizon_client::telemetry::mark_ui_ready();
            log::info!(
                "Horizon: UI bundle ready after {} ms",
                started.elapsed().as_millis()
            );
            publish(ui, &root);
        } else {
            loaded_gui = Some(ui);
        }
    }
    log::info!("load assets: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(1);
    let behaviour_json: serde_json::Value = serde_json::from_reader(std::io::BufReader::new(
        std::fs::File::open(root.join("block-behaviours.json"))?,
    ))?;
    // The block facts, the biomes and the environment are independent reads:
    // run them together, within the worker budget the settings allow.
    let registry = assets.registry;
    let load_facts = || {
        BlockFacts::load_with_projection(&root, registry, &behaviour_json)
            .map_err(|e| e.to_string())
    };
    let load_biomes = || Biomes::load(&root).map_err(|e| e.to_string());
    let load_environment = || Environment::load(&root).map_err(|e| e.to_string());
    let (facts, biomes, environment) = if threads > 1 {
        std::thread::scope(|scope| {
            let facts = scope.spawn(load_facts);
            let biomes = scope.spawn(load_biomes);
            let environment = scope.spawn(load_environment);
            (
                facts.join().unwrap(),
                biomes.join().unwrap(),
                environment.join().unwrap(),
            )
        })
    } else {
        (load_facts(), load_biomes(), load_environment())
    };
    let (facts, gameplay_projection) =
        facts.map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    let biomes = Arc::new(biomes.map_err(|e| -> Box<dyn std::error::Error> { e.into() })?);
    let environment = environment.map_err(|e| -> Box<dyn std::error::Error> { e.into() })?;
    log::info!("load facts: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(2);
    log::info!("load biomes+env: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(3);
    let light = Arc::new(LightTables::load(&root, assets.registry, &facts)?);
    let pack = PackResources::load(&root, assets.clone(), render, &facts, biomes.clone(), light)?;
    log::info!("load pack tables: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(4);
    if !menu {
        horizon_client::ui::load_metadata(&root);
        horizon_client::lang::set(horizon_client::lang::Language::load(
            &[&root],
            &settings.language,
        ));
    }
    let registry_owner = assets.clone();
    let collision = collisions::load(&root, registry_owner.registry)?;
    let motion = MotionResources::load_with_facts(&root, &registry_owner, &facts)?;
    log::info!(
        "load collision+motion: {:.2}s",
        started.elapsed().as_secs_f32()
    );
    progress.set(5);
    let resources = Resources {
        core: WorldCore {
            facts,
            biomes: biomes.clone(),
            environment,
            collision,
            motion,
            threads,
        },
        rest: std::sync::OnceLock::new(),
    };
    let resources = Arc::new(resources);
    if menu {
        horizon_client::telemetry::mark_core_ready();
        log::info!(
            "Horizon: core-ready after {} ms",
            started.elapsed().as_millis()
        );
    }
    // Finish fallible CPU preparation before starting the tail, so an error
    // cannot detach a tail worker that races the next pack-loader generation.
    let prepared_at = Instant::now();
    let game_cpu = menu.then(|| GameCpuResources::load(&root)).transpose()?;
    let renderer_cpu = menu.then(|| prepare_renderer_textures(&pack));
    if menu {
        log::info!(
            "Horizon: handoff CPU resources prepared on worker in {} ms",
            prepared_at.elapsed().as_millis()
        );
    }
    let rest_loader = if menu {
        let resources = resources.clone();
        let root = root.clone();
        let rest_progress = progress.clone();
        let progress = progress.clone();
        let handle = std::thread::Builder::new()
            .name("world-rest-loader".into())
            .spawn(move || {
                let rest = load_world_rest(
                    &root,
                    &resources,
                    &assets,
                    &behaviour_json,
                    &gameplay_projection,
                    &progress,
                    started,
                )
                .map_err(|error| error.to_string())?;
                resources
                    .rest
                    .set(rest)
                    .map_err(|_| "world resources already loaded".to_owned())?;
                progress.set(9);
                horizon_client::telemetry::mark_rest_ready();
                log::info!(
                    "Horizon: rest-ready after {} ms",
                    started.elapsed().as_millis()
                );
                Ok(())
            })?;
        Some(WorldRestLoader {
            progress: rest_progress,
            handle,
        })
    } else {
        let rest = load_world_rest(
            &root,
            &resources,
            &assets,
            &behaviour_json,
            &gameplay_projection,
            progress,
            started,
        )?;
        resources
            .rest
            .set(rest)
            .map_err(|_| "world resources already loaded")?;
        None
    };
    Ok(Loaded {
        game_cpu,
        renderer_cpu,
        root,
        pack_error,
        resources,
        rest_loader,
        pack,
        gui: loaded_gui,
    })
}

fn load_world_rest(
    root: &std::path::Path,
    resources: &Resources,
    assets: &Assets,
    behaviour_json: &serde_json::Value,
    gameplay_projection: &[horizon_client::blocks::GameplayProjection],
    progress: &Progress,
    started: Instant,
) -> Result<WorldRest> {
    let facts = resources.facts();
    let biomes = resources.biomes();
    let threads = resources.threads();
    let registry_owner = assets;
    progress.set(6);
    let mob_data = horizon_client::mobs::MobData::load(root, facts);
    horizon_client::mobs::trial_spawner::load_registry(root);

    let items = Items::load(root, assets.registry, facts)?;
    log::info!("load items: {:.2}s", started.elapsed().as_secs_f32());
    progress.set(7);
    let recipes = Recipes::load(root, &items)?;
    let transformers = horizon_client::block_transform::Transformers::load(root, assets.registry)?;
    log::info!(
        "Horizon: resources loaded in {:.1}s; {} block states, {} items, {} biomes; {} worker threads",
        started.elapsed().as_secs_f32(),
        facts.states.len(),
        items.items.len(),
        biomes.list.len(),
        threads
    );
    progress.set(8);
    let stations = horizon_client::stations::StationData::load(root, &items);
    let animate =
        horizon_client::animate_tick::AnimateTables::load(root, registry_owner.registry, biomes)?;
    let fog = horizon_client::fog::FogTables::load(root, biomes);
    Ok(WorldRest {
        stations,
        items,
        recipes,
        transformers,
        providers: Providers::load(root),
        block_entity_render: horizon_client::block_entity_render::BlockEntityRenderer::load(
            root,
            registry_owner.registry,
        )?,
        animate,
        fog,
        music: horizon_client::music::MusicTables::load(root),
        ambient: horizon_client::ambience::AmbientTables::load(root),
        nether_environment: Environment::load_dimension(root, "the_nether", false)?,
        end_environment: Environment::load_dimension(root, "the_end", false)?,
        behaviours: Behaviours::load_with_gameplay(
            registry_owner.registry,
            root,
            behaviour_json,
            gameplay_projection,
        )?,
        mob_types: MobTypes::load(root)?
            .types
            .into_iter()
            .map(|(k, v)| (k, Arc::new(v)))
            .collect(),
        mob_data,
        player_type: perspective::player_type(root),
        player_arm: horizon_client::mobs::load_model(root, "minecraft:player#main")
            .and_then(|m| m.children.into_iter().find(|c| c.name == "right_arm")),
        spin_attack: horizon_client::mobs::load_model(root, "minecraft:spin_attack#main"),
    })
}

/// CPU mip chains for the block sprites and world border.
fn prepare_renderer_textures(pack: &PackResources) -> horizon_client::gfx::PreparedTextures {
    let assets = &pack.render;
    horizon_client::gfx::PreparedTextures::new(
        &assets.pixels,
        assets.sprite_size,
        assets.texture_layers,
        &assets.sprite_mips,
        (&pack.forcefield.2, pack.forcefield.0),
    )
}

/// Upload pack textures and create the renderer on the window's device.
fn build_renderer(
    window: Arc<Window>,
    offscreen: bool,
    pack: &PackResources,
    gui: &UiResources,
    previous: Option<&Renderer>,
    prepared: Option<horizon_client::gfx::PreparedTextures>,
    api: horizon_client::graphics::GraphicsApi,
) -> Result<Renderer> {
    let assets = &pack.render;
    let atlas = &gui.atlas;
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        offscreen,
        &assets.pixels,
        assets.sprite_size,
        assets.sprite_minimum_size,
        assets.texture_layers,
        &assets.sprite_mips,
        &assets.animations,
        (&atlas.pixels, atlas.width, atlas.height),
        (&pack.forcefield.2, pack.forcefield.0),
        &gui.panorama,
        prepared,
        previous,
        api,
    ))?;
    renderer.hud.fire_animations = atlas
        .fire_animations
        .iter()
        .map(|(rect, animation)| (*rect, animation.clone(), usize::MAX))
        .collect();
    Ok(renderer)
}

/// The loading screen's renderer: the real HUD atlas and font over one opaque
/// white block-sprite layer, an empty border texture and the title panorama
/// once the menu bundle is ready.
fn loading_renderer(
    window: Arc<Window>,
    offscreen: bool,
    gui: Option<&UiResources>,
    previous: Option<&Renderer>,
    api: horizon_client::graphics::GraphicsApi,
) -> Result<Renderer> {
    static FORCEFIELD: [u8; 4] = [0; 4];
    static EMPTY_HUD: [u8; 4] = [0, 0, 0, 255];
    let empty_panorama = horizon_client::gfx::panorama::Faces {
        size: 1,
        pixels: [0u8; 4].repeat(6),
    };
    let panorama = gui.map_or(&empty_panorama, |gui| gui.panorama.as_ref());
    let atlas = gui.map(|gui| &gui.atlas);
    let (pixels, width, height) = match atlas {
        Some(atlas) => (atlas.pixels.as_slice(), atlas.width, atlas.height),
        None => (EMPTY_HUD.as_slice(), 1, 1),
    };
    let mut renderer = pollster::block_on(Renderer::new(
        window,
        offscreen,
        &[255, 255, 255, 255],
        1,
        1,
        1,
        &[],
        &[],
        (pixels, width, height),
        (&FORCEFIELD[..], 1),
        panorama,
        None,
        previous,
        api,
    ))?;
    if let Some(atlas) = atlas {
        renderer.hud.fire_animations = atlas
            .fire_animations
            .iter()
            .map(|(rect, animation)| (*rect, animation.clone(), usize::MAX))
            .collect();
    }
    Ok(renderer)
}

/// A title-shaped frame: black clear and panorama, only the HUD is drawn.
fn loading_frame(hud: &HudBatch) -> Frame<'_> {
    Frame {
        weather: &[],
        weather_time: horizon_client::weather::Time::new(0, 0.0),
        camera: glam::DVec3::new(0.5, 100.0, 0.5),
        yaw: 0.0,
        pitch: 10.0,
        fov_degrees: 85.0,
        bob: glam::Mat4::IDENTITY,
        sky: horizon_client::environment::Sky::default(),
        distance: 12.0,
        detail_distance: 12.0,
        depth_far: 1000.0,
        underwater: false,
        cave_culling: false,
        title: true,
        skybox: false,
        end_sky: false,
        end_sky_sprite: None,
        environment_fog: None,
        water_fog: [0.0; 3],
        fog: None,
        dark_disc: false,
        outline: Vec::new(),
        outline_after_terrain: false,
        debug_lines: &[],
        debug_lines_on_top: &[],
        debug_crosshair: &[],
        hud,
        nether_lighting: false,
        entities: &[],
        dynamic_lights: &[],
        clouds: None,
        lightning: Default::default(),
        additive: &[],
        see_through: &[],
        celestial: &[],
        stars: None,
        hand: &[],
        hand_fov_degrees: 70.0,
        screen: &[],
        pack: Default::default(),
        border: None,
        game_time: 0.0,
        shader_game_time: 0.0,
        end_portal_sprites: [[0.0; 4]; 2],
        crumbling_sprite: [0.0; 4],
        ticks: 0,
        darkness: (0.0, 0.0),
        night_vision: 0.0,
        block_flicker: 0.0,
        boss_darken: 0.0,
        menu_blur: None,
    }
}

/// The post-load setup once the resources are ready: `Game::new` and the
/// options, packs, mods, control socket, benchmark and world selection the
/// old `run` did after loading. `None` means a harness or dedicated run
/// finished here instead.
#[allow(clippy::too_many_arguments)]
fn start_game<'a>(
    res: &'a Arc<Resources>,
    pack: PackResources,
    gui: UiResources,
    options: Options,
    pack_error: Option<String>,
    audio: Audio,
    vanilla_root: PathBuf,
    started: Instant,
    rest_loader: Option<WorldRestLoader>,
    settings: Option<Settings>,
    prepared: Option<GameCpuResources>,
) -> Result<Option<Game<'a>>> {
    let handoff = prepared.is_some();
    let mut settings = match settings {
        Some(settings) => settings,
        None => Settings::load(&std::env::current_dir()?.join("options.txt")),
    };
    if let Some(d) = options.distance_override {
        settings.render_distance = d;
    }
    let mut game = Game::new(res, pack, gui, options, settings, audio, prepared)?;
    game.rest_loader = rest_loader;
    game.ui.vanilla_root = vanilla_root;
    if let Some(error) = pack_error {
        // Minecraft.clearResourcePacksOnError: the selection is cleared and
        // saved, so the next launch starts without the packs.
        game.settings.resource_packs.clear();
        game.settings.save(&Game::options_path());
        game.open_options_at(true, horizon_client::options_ui::Page::Packs);
        game.ui.options.resource_error = Some(error);
    }
    if game.options.benchmark.is_some() {
        // The same conditions on every machine: no VSync, no frame cap, no
        // pause or idle limit without focus. In memory only, never saved.
        game.settings.vsync = false;
        game.settings.max_fps = 260;
        game.settings.inactivity_fps_limit = "minimized".into();
        game.options.no_pause = true;
    }
    game.apply_settings();
    // Benchmarks measure the game alone.
    if game.options.benchmark.is_none() {
        game.mods.dev = game.options.dev_mods.clone();
        game.start_mods();
    }
    game.start_agent_control()?;
    if let Some(scenario) = game.options.benchmark.clone() {
        let control = game
            .control
            .get_or_insert_with(horizon_client::control::Server::local);
        horizon_client::benchmark::start(
            control.sender(),
            horizon_client::benchmark::Setup {
                scenario,
                started,
                app: horizon_bench::App {
                    name: "horizon-client".into(),
                    version: horizon_client::crash::version(),
                    commit: option_env!("HORIZON_COMMIT").map(str::to_owned),
                    mods: Vec::new(),
                },
                simulation_distance: game.settings.simulation_distance as u32,
                vsync: false,
                fps_cap: None,
                offscreen: game.options.offscreen,
                game_dir: std::env::current_dir()?,
            },
        );
    }
    if let Ok(spec) = std::env::var("HORIZON_DISPENSE_FIXTURE") {
        game.dispense_fixture(&spec)?;
        return Ok(None);
    }
    if let Ok(spec) = std::env::var("HORIZON_SCULK_FIXTURE") {
        game.sculk_fixture(&spec)?;
        return Ok(None);
    }
    if game.options.execution_role == execution_role::ExecutionRole::DedicatedServer {
        game.run_dedicated()?;
        return Ok(None);
    }
    match game.options.world.clone() {
        _ if game.options.server.is_some() => {
            let (address, name) = (
                game.options.server.clone().unwrap_or_default(),
                game.options.player_name(),
            );
            game.connect_server(&address, &name)?;
            if let Some(report) = std::env::var_os("HORIZON_NET_REPORT") {
                game.run_net_harness(report.as_ref())?;
                return Ok(None);
            }
        }
        Some(folder) => {
            let (seed, creative) = (game.options.seed, game.options.creative);
            if std::env::var_os("HORIZON_COMMAND_SCRIPT").is_some() {
                // The command harness runs before entering the window loop.
                game.open_world(&folder, None, Some(seed), creative, None)?;
            } else {
                // Start after the window is handed over, so --world uses the
                // same responsive initialization path as the title screen.
                game.title = Some(Title::new(
                    std::env::current_dir()?.join("saves"),
                    &game.options.resources,
                ));
                game.pending_entry = Some(PendingWorldEntry::Local {
                    folder,
                    name: None,
                    seed: Some(seed),
                    creative,
                    create: None,
                });
            }
        }
        None if handoff => {}
        None => {
            let mut title = Title::new(
                std::env::current_dir()?.join("saves"),
                &game.options.resources,
            );
            title.last_server = game.settings.file.string("lastServer");
            game.title = Some(title);
        }
    }
    if let (Some(script), Some(output)) = (
        std::env::var_os("HORIZON_COMMAND_SCRIPT"),
        std::env::var_os("HORIZON_COMMAND_OUTPUT"),
    ) {
        game.run_command_harness(script.as_ref(), output.as_ref())?;
        return Ok(None);
    }
    Ok(Some(game))
}

fn write_report(game: &Game<'_>) -> Result<()> {
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
    Ok(())
}

fn run_event_loop(mut game: Game<'_>) -> Result<()> {
    EventLoop::new()?.run_app(&mut game)?;
    horizon_client::discord_presence::stop();
    write_report(&game)?;
    if let Some(e) = game.error.take() {
        return Err(e);
    }
    Ok(())
}

/// The loading screen's state while the worker loads the resources.
struct Loading {
    window: Option<Arc<Window>>,
    renderer: Option<Renderer>,
    gui: Option<UiResources>,
    settings: Settings,
    fullscreen: horizon_client::window::FullscreenState,
    options: Option<Options>,
    vanilla_root: PathBuf,
    pack_error: Option<String>,
    started: Instant,
    panorama_rotation: horizon_client::gfx::panorama::Rotation,
    progress: Progress,
    receiver: Receiver<LoaderMsg>,
    loaded: Option<std::result::Result<Loaded, String>>,
    handle: Option<JoinHandle<()>>,
    shown: bool,
    menu: startup_menu::StartupMenu,
    error: Option<Box<dyn std::error::Error>>,
}
impl Loading {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        // The Window constructor: created fullscreen when options.txt says so.
        let fullscreen = self.settings.fullscreen;
        let size = self
            .options
            .as_ref()
            .map_or([1280, 720], |o| o.window_size)
            .map(f64::from);
        self.fullscreen = horizon_client::window::FullscreenState::new(fullscreen);
        let attributes = Window::default_attributes()
            .with_title("Horizon")
            .with_window_icon(horizon_client::window::icon());
        // Wayland names the window by its app id, which the horizon.desktop entry matches.
        #[cfg(target_os = "linux")]
        let attributes = winit::platform::wayland::WindowAttributesExtWayland::with_name(
            attributes, "horizon", "horizon",
        );
        let window = match event_loop.create_window(
            attributes
                .with_inner_size(winit::dpi::LogicalSize::new(size[0], size[1]))
                .with_fullscreen(fullscreen.then_some(winit::window::Fullscreen::Borderless(None))),
        ) {
            Ok(w) => Arc::new(w),
            Err(e) => {
                self.error = Some(e.into());
                event_loop.exit();
                return;
            }
        };
        horizon_client::telemetry::note_display(&window);
        let offscreen = self.options.as_ref().is_some_and(|o| o.offscreen);
        match loading_renderer(
            window.clone(),
            offscreen,
            self.gui.as_ref(),
            None,
            self.settings.horizon.api,
        ) {
            Ok(r) => self.renderer = Some(r),
            Err(e) => {
                self.error = Some(e);
                event_loop.exit();
                return;
            }
        }
        if let Some(r) = &self.renderer {
            self.settings
                .clamp_gui_scale([r.config.width as i32, r.config.height as i32]);
        }
        horizon_client::hud::GUI_SCALE.store(self.settings.gui_scale, Ordering::Relaxed);
        self.window = Some(window);
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if self.window.as_ref().is_none_or(|w| w.id() != id) {
            return;
        }
        if let Err(error) = self.menu_event(event_loop, event) {
            self.error = Some(error);
            event_loop.exit();
        }
    }
    /// The menu bundle arrived: rebuild the loading screen with the real atlas
    /// and title panorama.
    fn set_ui(&mut self, gui: UiResources, root: PathBuf) {
        log::info!(
            "Horizon: loading the title artwork after {} ms",
            self.started.elapsed().as_millis()
        );
        self.menu.set_root(root, &self.settings);
        self.gui = Some(gui);
        horizon_client::ui_theme::reset_atlas();
        self.apply_menu_settings();
        let Some(window) = self.window.clone() else {
            return;
        };
        let offscreen = self.options.as_ref().is_some_and(|o| o.offscreen);
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        match loading_renderer(
            window,
            offscreen,
            self.gui.as_ref(),
            self.renderer.as_ref(),
            self.settings.horizon.api,
        ) {
            Ok(r) => {
                self.settings
                    .clamp_gui_scale([r.config.width as i32, r.config.height as i32]);
                horizon_client::hud::GUI_SCALE.store(self.settings.gui_scale, Ordering::Relaxed);
                self.renderer = Some(r);
            }
            Err(e) => self.error = Some(e),
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(error) = self.poll_menu_resources() {
            self.error = Some(error);
            event_loop.exit();
            return;
        }
        if let Some(w) = &self.window {
            if self.options.as_ref().is_some_and(|o| o.offscreen) {
                self.draw();
            } else {
                w.request_redraw();
            }
        }
        event_loop.set_control_flow(ControlFlow::Poll);
    }
    fn draw(&mut self) {
        let Some(size) = self
            .renderer
            .as_ref()
            .map(|r| [r.config.width, r.config.height])
        else {
            return;
        };
        let mut batch = HudBatch::new(size);
        self.draw_menu(&mut batch);
        let mut frame = loading_frame(&batch);
        frame.yaw = self.panorama_rotation.update(
            self.started.elapsed().as_secs_f32(),
            self.settings.file.double("panoramaScrollSpeed") as f32,
        );
        if let Some(renderer) = &mut self.renderer {
            match renderer.render(&frame) {
                Ok(true) if self.gui.is_some() && self.menu.pending.is_none() => {
                    self.menu.perf.title_shown()
                }
                Ok(_) => {}
                Err(e) => log::warn!("Horizon: loading screen frame failed: {e}"),
            }
        }
        if !self.shown {
            self.shown = true;
            log::info!(
                "Horizon: loading screen shown after {} ms",
                self.started.elapsed().as_millis()
            );
        }
    }
    fn finish(&mut self, loaded: Loaded) -> Result<Option<Game<'static>>> {
        let handoff_at = Instant::now();
        let mut options = self.options.take().ok_or("loading already finished")?;
        options.resources = loaded.root.clone();
        // StartupMenu owns world selection until it can hand it to Game.
        options.world = None;
        options.server = None;
        let gui = self.gui.take().ok_or("missing menu bundle")?;
        let window = self.window.clone().ok_or("window closed")?;
        // Keep the loader's device and swap in the real textures in one step.
        if let Some(renderer) = &mut self.renderer {
            renderer.release_surface();
        }
        let renderer_at = Instant::now();
        let renderer = build_renderer(
            window.clone(),
            options.offscreen,
            &loaded.pack,
            &gui,
            self.renderer.as_ref(),
            loaded.renderer_cpu,
            self.settings.horizon.api,
        )?;
        log::info!(
            "Horizon: world renderer ready after {} ms",
            self.started.elapsed().as_millis()
        );
        let renderer_ms = renderer_at.elapsed().as_millis();
        // Audio is created on the main thread, after the core is ready.
        let audio = if !options.execution_role.is_client() {
            Audio::silent(&options.resources)
        } else {
            Audio::new(&options.resources)
        };
        let pack_error = loaded.pack_error.or_else(|| self.pack_error.take());
        // The game borrows its shared resources for the process's lifetime,
        // like the single copy an integrated server would share.
        let res: &'static Arc<Resources> = Box::leak(Box::new(loaded.resources));
        let game_at = Instant::now();
        let Some(mut game) = start_game(
            res,
            loaded.pack,
            gui,
            options,
            pack_error,
            audio,
            self.vanilla_root.clone(),
            self.started,
            loaded.rest_loader,
            Some(self.settings.clone()),
            loaded.game_cpu,
        )?
        else {
            return Ok(None);
        };
        let game_ms = game_at.elapsed().as_millis();
        game.window = Some(window);
        game.renderer = Some(renderer);
        game.world_renderer_ready = true;
        game.fullscreen = self.fullscreen;
        game.panorama_rotation = self.panorama_rotation;
        self.menu.handoff(&mut game);
        game.apply_settings();
        game.previous = Instant::now();
        log::info!(
            "Horizon: startup handoff {} ms (renderer {} ms, start_game {} ms)",
            handoff_at.elapsed().as_millis(),
            renderer_ms,
            game_ms
        );
        Ok(Some(game))
    }
}

/// The application: the loading screen until the resources are ready, then
/// the game itself, on the same event loop and window.
enum App {
    Loading(Box<Loading>),
    Ready(Box<Game<'static>>),
}
impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        match self {
            App::Loading(loading) => loading.resumed(event_loop),
            App::Ready(game) => game.resumed(event_loop),
        }
    }
    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        match self {
            App::Loading(loading) => loading.window_event(event_loop, id, event),
            App::Ready(game) => game.window_event(event_loop, id, event),
        }
    }
    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        if let App::Ready(game) = self {
            game.device_event(event_loop, id, event);
        }
    }
    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        enum Step {
            Waiting,
            Ready(Box<Game<'static>>),
            Done,
            Failed(Box<dyn std::error::Error>),
        }
        let step = match self {
            App::Ready(game) => {
                game.about_to_wait(event_loop);
                return;
            }
            App::Loading(loading) => {
                loading.about_to_wait(event_loop);
                if loading.error.is_some() {
                    event_loop.exit();
                    return;
                }
                match if loading.menu.reload_packs {
                    None
                } else {
                    loading.loaded.take()
                } {
                    None => Step::Waiting,
                    Some(Err(e)) => Step::Failed(e.into()),
                    Some(Ok(loaded)) => match loading.finish(loaded) {
                        Ok(Some(game)) => Step::Ready(Box::new(game)),
                        Ok(None) => Step::Done,
                        Err(e) => Step::Failed(e),
                    },
                }
            }
        };
        match step {
            Step::Waiting => {}
            Step::Ready(game) => *self = App::Ready(game),
            Step::Done => event_loop.exit(),
            Step::Failed(e) => {
                if let App::Loading(loading) = self {
                    loading.error = Some(e);
                }
                event_loop.exit();
            }
        }
    }
    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        if let App::Ready(game) = self {
            game.exiting(event_loop);
        }
    }
}

/// The ordinary launch: load the resources on a worker while the window shows
/// the loading screen, then swap in the game on the main thread.
fn run_loading(
    options: Options,
    settings: Settings,
    vanilla_root: PathBuf,
    pack_error: Option<String>,
    started: Instant,
) -> Result<()> {
    let progress = Progress::new();
    let (receiver, loader) =
        startup_menu::spawn_loader(&options, &settings, &vanilla_root, started, &progress)?;
    let menu = startup_menu::StartupMenu::new(&options, pack_error.as_deref())?;
    let loading = Loading {
        window: None,
        renderer: None,
        gui: None,
        settings,
        fullscreen: horizon_client::window::FullscreenState::new(false),
        options: Some(options),
        vanilla_root,
        pack_error,
        started,
        panorama_rotation: Default::default(),
        progress,
        receiver,
        loaded: None,
        handle: Some(loader),
        shown: false,
        menu,
        error: None,
    };
    let mut app = App::Loading(Box::new(loading));
    EventLoop::new()?.run_app(&mut app)?;
    horizon_client::discord_presence::stop();
    match app {
        App::Ready(mut game) => {
            write_report(&game)?;
            if let Some(e) = game.error.take() {
                return Err(e);
            }
        }
        App::Loading(loading) => {
            if let Some(e) = loading.error {
                return Err(e);
            }
        }
    }
    Ok(())
}

pub fn run() -> Result<()> {
    let Some(mut options) = Options::read()? else {
        return Ok(());
    };
    if options.execution_role == execution_role::ExecutionRole::DedicatedServer
        && !dedicated::preflight(&mut options)?
    {
        return Ok(());
    }
    // Before any worker starts: on Linux they go behind the server thread
    // and the render thread.
    horizon_threads::put_helpers_behind();
    if options.execution_role.is_client() {
        // The render thread, like the server thread, ahead of the workers on
        // macOS and Windows. While terrain loads, the worker pools (two with
        // single player's server thread) and other programs outnumber the
        // cores, and a frame that gave its core up (in a GPU submit) waited
        // 10 to 200 ms to get one back.
        horizon_threads::favour_current();
    }
    // Someone is playing: the anonymous ping behind the launcher's "N playing now". Not for a
    // server, a benchmark or a run that quits by itself.
    if options.execution_role.is_client()
        && options.benchmark.is_none()
        && !options.offscreen
        && options.frames.is_none()
        && options.world_frames.is_none()
    {
        horizon_client::heartbeat::start();
    }
    let started = Instant::now();
    horizon_client::spans::enable(options.spikes.is_some());
    let vanilla_root = options.resources.clone();
    let early_settings = Settings::load(&std::env::current_dir()?.join("options.txt"));
    // "Playing Horizon" on Discord: never for a server; an automated run only talks to a test's fake Discord.
    if options.dedicated.is_none() {
        let automated = options.benchmark.is_some()
            || options.offscreen
            || options.frames.is_some()
            || options.world_frames.is_some();
        horizon_client::discord_presence::start(early_settings.extras.discord_presence, automated);
    }
    // Snapshots of derived tables live in the game folder, never beside the
    // shared resources.
    Behaviours::set_cache_dir(std::env::current_dir()?.join(".horizon-cache"));
    if !synchronous_startup(&options) {
        return run_loading(options, early_settings, vanilla_root, None, started);
    }
    let mut pack_error = None;
    options.resources = packs::startup_root(&vanilla_root, &early_settings.resource_packs, &[])
        .unwrap_or_else(|e| {
            log::warn!("Horizon: resource packs not applied: {e}");
            pack_error = Some(e.to_string());
            vanilla_root.clone()
        });
    options.resources = mods::content_root(
        &options.resources,
        &early_settings.mods.enabled,
        &options.dev_mods,
    );
    let loaded = load_resources(
        options.resources.clone(),
        &vanilla_root,
        &early_settings,
        started,
        false,
        &Progress::new(),
        &mut |_, _| {},
    )?;
    let gui = loaded.gui.ok_or("missing menu bundle")?;
    options.resources = loaded.root.clone();
    let audio = if !options.execution_role.is_client() {
        Audio::silent(&options.resources)
    } else {
        Audio::new(&options.resources)
    };
    let resources = loaded.resources;
    let game = start_game(
        &resources,
        loaded.pack,
        gui,
        options,
        loaded.pack_error.or(pack_error),
        audio,
        vanilla_root,
        started,
        loaded.rest_loader,
        None,
        None,
    )?;
    if let Some(game) = game {
        run_event_loop(game)?;
    }
    Ok(())
}

// Player.tick: cap the sleeping fade at 100, then fade out over 10 waking ticks.
fn tick_sleep_timer(timer: u32, sleeping: bool) -> u32 {
    if sleeping {
        (timer + 1).min(100)
    } else if (1..109).contains(&timer) {
        timer + 1
    } else {
        0
    }
}

fn reset_world_progress(adv: &mut advancements::AdvState, stats: &mut stats::Stats) {
    *adv = Default::default();
    *stats = Default::default();
}

#[cfg(test)]
mod startup_cpu_tests {
    use super::*;

    #[test]
    fn worker_prepares_owned_models_and_inventory_before_handoff() {
        let root = std::env::temp_dir().join(format!("horizon-startup-cpu-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("entity-models.json"),
            r#"{
            "minecraft:end_crystal#main": {}, "minecraft:ender_dragon#main": {},
            "minecraft:arrow#main": {}, "minecraft:trident#main": {},
            "minecraft:armor_stand#main": {}
        }"#,
        )
        .unwrap();
        std::fs::write(
            root.join("creative-tabs.json"),
            r#"[{
            "id":"minecraft:building_blocks", "items":["minecraft:stone"]
        }]"#,
        )
        .unwrap();
        let worker_root = root.clone();
        let prepared = std::thread::spawn(move || {
            GameCpuResources::load(&worker_root).map_err(|error| error.to_string())
        })
        .join()
        .unwrap()
        .unwrap();
        std::fs::remove_dir_all(&root).unwrap();
        assert!(prepared.crystal_model.is_some());
        assert!(prepared.dragon_model.is_some());
        assert!(prepared.arrow_model.is_some());
        assert!(prepared.trident_model.is_some());
        assert!(prepared.armor_stand_assets.main.is_some());
        assert_eq!(
            prepared.creative_screen.tabs[0].items[0].item.as_ref(),
            "minecraft:stone"
        );
        // No documents from the old generation remain available to reload.
        assert!(GameCpuResources::load(&root).is_err());
    }
}

#[cfg(test)]
mod runtime_reset_tests {
    use super::*;
    #[test]
    fn world_entry_invalidates_loaded_progress() {
        let mut adv = advancements::AdvState::default();
        let mut stats = stats::Stats::default();
        adv.loaded = true;
        stats.loaded = true;
        adv.progress
            .criteria
            .insert("minecraft:story/root".into(), Default::default());
        stats.add("mined", "minecraft:stone", 4);
        reset_world_progress(&mut adv, &mut stats);
        assert!(!adv.loaded && !stats.loaded);
        assert!(adv.progress.criteria.is_empty());
        assert!(stats.values.is_empty());
    }
}

#[cfg(test)]
mod entity_model_loading_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::container_screen::{Button, Clicks, Release, Target};
    use horizon_client::keymapping::{KeyAction, SPRINT, ToggleKeys};

    #[test]
    fn sleep_fade_lasts_ten_ticks_after_waking() {
        let alpha = |timer| match hud_overlays::sleep_overlay(320, 240, timer).first() {
            Some(hud_layout::GuiDraw::Fill { color, .. }) => color >> 24,
            None => 0,
            _ => panic!("sleep overlay must be a full-screen fill"),
        };
        assert_eq!(tick_sleep_timer(0, false), 0);
        let mut timer = 0;
        for _ in 0..50 {
            timer = tick_sleep_timer(timer, true);
        }
        assert_eq!(alpha(timer as i32), 110);
        for _ in 0..100 {
            timer = tick_sleep_timer(timer, true);
        }
        assert_eq!(timer, 100);
        assert_eq!(alpha(timer as i32), 220);
        for _ in 0..5 {
            timer = tick_sleep_timer(timer, false);
        }
        assert_eq!(alpha(timer as i32), 110);
        for _ in 0..5 {
            timer = tick_sleep_timer(timer, false);
        }
        assert_eq!(timer, 0);
        assert!(hud_overlays::sleep_overlay(320, 240, timer as i32).is_empty());
    }

    #[test]
    fn world_rest_poll_does_not_wait_for_a_blocked_worker() {
        let (release, wait) = std::sync::mpsc::channel();
        let loader = WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(move || {
                wait.recv().unwrap();
                Ok(())
            }),
        };
        assert!(!loader.ready().unwrap());
        release.send(()).unwrap();
        loader.finish().unwrap();
    }

    #[test]
    fn world_rest_poll_waits_for_a_slow_worker() {
        let (release, wait) = std::sync::mpsc::channel();
        let loader = WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(move || {
                wait.recv().unwrap();
                Ok(())
            }),
        };
        // However long it takes, a worker still loading is not an error.
        assert!(!loader.ready().unwrap());
        release.send(()).unwrap();
        while !loader.ready().unwrap() {
            std::thread::sleep(Duration::from_millis(1));
        }
        loader.finish().unwrap();
    }

    #[test]
    fn world_rest_worker_failure_reaches_the_client() {
        let loader = WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(|| Err("missing world table".to_owned())),
        };
        assert_eq!(
            loader.finish().unwrap_err().to_string(),
            "missing world table"
        );
    }

    #[test]
    fn world_rest_worker_panic_reaches_the_client() {
        let loader = WorldRestLoader {
            progress: Progress::new(),
            handle: std::thread::spawn(|| panic!("broken loader")),
        };
        assert_eq!(
            loader.finish().unwrap_err().to_string(),
            "world resource loader panicked"
        );
    }

    /// An integrated server thread shares the loaded resources with the client.
    #[test]
    fn shared_resources_can_cross_threads() {
        fn shared<T: Send + Sync>() {}
        shared::<Resources>();
        shared::<PackResources>();
    }

    #[test]
    fn selecting_creative_tabs_does_not_drop_the_carried_stack_on_release() {
        let mut creative = Creative::from_json(&serde_json::json!({
            "schema": 2,
            "tabs": [{"type": "CATEGORY"}, {"type": "INVENTORY"}]
        }))
        .unwrap();
        for button in [Button::Left, Button::Right] {
            let mut clicks = Clicks::default();
            for tab in [1, 1, 0, 1] {
                select_creative_tab(&mut creative, &mut clicks, tab);
                assert_eq!(creative.selected, tab);
                assert_eq!(creative.tab().inventory, tab == 1);
                assert_eq!(
                    clicks.release(
                        &Target::<usize>::Outside,
                        button,
                        false,
                        false,
                        false,
                        false
                    ),
                    Release::Nothing
                );
            }
        }
    }

    #[test]
    fn stack_drop_requires_physical_ctrl_even_with_sprint_latched() {
        for sprint in [false, true] {
            let mut toggles = ToggleKeys::default();
            if sprint {
                toggles.key(SPRINT, KeyAction::Press, false, true);
                toggles.key(SPRINT, KeyAction::Release, false, true);
            }
            assert_eq!(toggles.is_down(SPRINT), sprint);

            for held in 0..4 {
                let mut physical_keys = HashSet::from([KeyCode::ShiftLeft, KeyCode::KeyQ]);
                if held & 1 != 0 {
                    physical_keys.insert(KeyCode::ControlLeft);
                }
                if held & 2 != 0 {
                    physical_keys.insert(KeyCode::ControlRight);
                }
                assert_eq!(drop_whole_stack(&physical_keys), held != 0);
                physical_keys.remove(&KeyCode::ControlLeft);
                physical_keys.remove(&KeyCode::ControlRight);
                assert!(!drop_whole_stack(&physical_keys));
                assert_eq!(toggles.is_down(SPRINT), sprint);
            }
        }
    }
}
