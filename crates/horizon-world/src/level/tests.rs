//! Behaviour tests over an in-memory level. They need the prepared client
//! resources (`HORIZON_RESOURCES`, or `.local/horizon-client/26.3` at the
//! repository root) and are skipped when those are absent.
use super::*;
use crate::chunk::BlockState;
use std::{
    collections::{BTreeMap, HashMap},
    path::PathBuf,
    sync::OnceLock,
};

struct Fixture {
    registry: BlockRegistry,
    behaviours: Behaviours,
}

fn fixture() -> Option<&'static Fixture> {
    static FIXTURE: OnceLock<Option<Fixture>> = OnceLock::new();
    FIXTURE
        .get_or_init(|| {
            let root = std::env::var_os("HORIZON_RESOURCES")
                .map(PathBuf::from)
                .unwrap_or_else(|| {
                    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                        .join("../../.local/horizon-client/26.3")
                });
            if !root.join("block-behaviours.json").exists() {
                eprintln!("skipping level tests: no resources at {}", root.display());
                return None;
            }
            let registry = data::load_registry(&root).expect("registry");
            let behaviours = Behaviours::load(&registry, &root).expect("behaviours");
            Some(Fixture {
                registry,
                behaviours,
            })
        })
        .as_ref()
}

pub(crate) struct TestLevel {
    blocks: HashMap<BlockPos, BlockStateId>,
    air: BlockStateId,
    random: LegacyRandom,
    pub time: i64,
    pub events: Vec<LevelEvent>,
    pub block_light: HashMap<BlockPos, i32>,
}
impl TestLevel {
    fn new(b: &Behaviours) -> Self {
        Self {
            blocks: HashMap::new(),
            air: b.air(),
            random: LegacyRandom::new(42),
            time: 0,
            events: Vec::new(),
            block_light: HashMap::new(),
        }
    }
}
impl LevelAccess for TestLevel {
    fn block_state(&self, pos: BlockPos) -> BlockStateId {
        self.blocks.get(&pos).copied().unwrap_or(self.air)
    }
    fn set_raw(&mut self, pos: BlockPos, state: BlockStateId) -> Option<BlockStateId> {
        Some(self.blocks.insert(pos, state).unwrap_or(self.air))
    }
    fn is_loaded(&self, _pos: BlockPos) -> bool {
        true
    }
    fn random(&mut self) -> &mut LegacyRandom {
        &mut self.random
    }
    fn game_time(&self) -> i64 {
        self.time
    }
    fn block_light(&self, pos: BlockPos) -> i32 {
        self.block_light.get(&pos).copied().unwrap_or(0)
    }
    fn emit(&mut self, event: LevelEvent) {
        self.events.push(event);
    }
}

struct World<'f> {
    f: &'f Fixture,
    level: TestLevel,
    ticks: Ticks,
}
impl<'f> World<'f> {
    fn new(f: &'f Fixture) -> Self {
        let mut ticks = Ticks::new(0);
        for x in -4..4 {
            for z in -4..4 {
                ticks.add_empty_chunk(ChunkPos::new(x, z)).unwrap();
            }
        }
        Self {
            level: TestLevel::new(&f.behaviours),
            f,
            ticks,
        }
    }
    fn b(&self) -> &'f Behaviours {
        &self.f.behaviours
    }
    fn id(&self, name: &str, props: &[(&str, &str)]) -> BlockStateId {
        let state = BlockState {
            name: format!("minecraft:{name}"),
            properties: props
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect::<BTreeMap<_, _>>(),
        };
        self.f
            .registry
            .resolve(&state)
            .unwrap_or_else(|_| panic!("unknown state {name} {props:?}"))
    }
    fn set(&mut self, pos: BlockPos, state: BlockStateId) -> bool {
        self.f.behaviours.set_block(
            &mut self.level,
            &mut self.ticks,
            pos,
            state,
            flags::UPDATE_ALL,
        )
    }
    /// Raw storage write without updates (world generation).
    fn put(&mut self, pos: BlockPos, state: BlockStateId) {
        self.level.blocks.insert(pos, state);
    }
    fn get(&self, pos: BlockPos) -> BlockStateId {
        self.level.block_state(pos)
    }
    fn name(&self, pos: BlockPos) -> String {
        let state = self.f.registry.state(self.get(pos)).unwrap();
        let props: Vec<String> = state
            .properties
            .iter()
            .map(|(k, v)| format!("{k}={v}"))
            .collect();
        format!(
            "{}[{}]",
            state.name.trim_start_matches("minecraft:"),
            props.join(",")
        )
    }
    fn prop(&self, pos: BlockPos, key: &str) -> String {
        self.f
            .registry
            .state(self.get(pos))
            .unwrap()
            .properties
            .get(key)
            .cloned()
            .unwrap_or_default()
    }
    fn tick(&mut self) {
        self.level.time += 1;
        self.f
            .behaviours
            .tick_scheduled(&mut self.level, &mut self.ticks);
    }
    /// Scheduled ticks, block events and block entities, as `ServerLevel.tick`.
    fn full_tick(&mut self) {
        self.tick();
        self.f
            .behaviours
            .run_block_events(&mut self.level, &mut self.ticks);
        self.f
            .behaviours
            .tick_block_entities(&mut self.level, &mut self.ticks);
    }
    fn floor(&mut self, y: i32, radius: i32, block: &str) {
        let state = self.id(block, &[]);
        for x in -radius..=radius {
            for z in -radius..=radius {
                self.put(BlockPos::new(x, y, z), state);
            }
        }
    }
}

macro_rules! world {
    () => {{
        let Some(f) = fixture() else {
            return;
        };
        World::new(f)
    }};
}

#[test]
fn state_table_matches_registry() {
    let Some(f) = fixture() else {
        return;
    };
    let t = &f.behaviours.table;
    assert_eq!(t.state_count(), f.registry.len());
    let stairs = f.registry.default_state("minecraft:oak_stairs").unwrap();
    let key = f.behaviours.props.facing;
    let east = t.with_text(stairs, key, "east").unwrap();
    assert_eq!(f.registry.state(east).unwrap().properties["facing"], "east");
    assert_eq!(t.text(east, f.behaviours.props.half), "bottom");
    assert!(classes::Class::ALL.len() <= 320);
}

#[test]
fn fences_connect_and_disconnect() {
    let mut w = world!();
    let fence = w.id("oak_fence", &[]);
    let a = BlockPos::new(0, 1, 0);
    let c = BlockPos::new(1, 1, 0);
    w.set(a, fence);
    w.set(c, fence);
    assert_eq!(w.prop(a, "east"), "true");
    assert_eq!(w.prop(c, "west"), "true");
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 1, -1), stone);
    assert_eq!(w.prop(a, "north"), "true", "fences connect to sturdy faces");
    let air = w.b().air();
    w.set(c, air);
    assert_eq!(w.prop(a, "east"), "false");
}

#[test]
fn walls_and_panes_connect() {
    let mut w = world!();
    let wall = w.id("cobblestone_wall", &[]);
    let pane = w.id("glass_pane", &[]);
    w.set(BlockPos::new(0, 1, 0), wall);
    w.set(BlockPos::new(0, 1, 1), wall);
    w.set(BlockPos::new(0, 1, 2), wall);
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "north"), "low");
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "south"), "low");
    assert_eq!(
        w.prop(BlockPos::new(0, 1, 1), "up"),
        "false",
        "straight wall drops its post"
    );
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 2, 1), stone);
    assert_eq!(w.prop(BlockPos::new(0, 1, 1), "north"), "tall");
    w.set(BlockPos::new(5, 1, 0), pane);
    w.set(BlockPos::new(6, 1, 0), pane);
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "east"), "true");
}

#[test]
fn stairs_form_corners() {
    let mut w = world!();
    let north = w.id("oak_stairs", &[("facing", "north")]);
    let east = w.id("oak_stairs", &[("facing", "east")]);
    w.set(BlockPos::new(0, 1, 0), north);
    w.set(BlockPos::new(0, 1, -1), east);
    // The north-facing stair has an east-facing stair behind it.
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "shape"), "outer_right");
}

#[test]
fn torch_and_door_halves_break_with_support() {
    let mut w = world!();
    w.floor(0, 3, "stone");
    let torch = w.id("torch", &[]);
    w.set(BlockPos::new(1, 1, 1), torch);
    let door = w.id("oak_door", &[("half", "lower")]);
    let upper = w.id("oak_door", &[("half", "upper")]);
    w.set(BlockPos::new(0, 1, 0), door);
    w.set(BlockPos::new(0, 2, 0), upper);
    let air = w.b().air();
    w.set(BlockPos::new(1, 0, 1), air);
    assert_eq!(
        w.get(BlockPos::new(1, 1, 1)),
        air,
        "torch pops without support"
    );
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::DropResources { .. }))
    );
    w.set(BlockPos::new(0, 1, 0), air);
    assert_eq!(
        w.get(BlockPos::new(0, 2, 0)),
        air,
        "upper door half goes with the lower"
    );
}

#[test]
fn snowy_grass_and_chests_pair() {
    let mut w = world!();
    let grass = w.id("grass_block", &[]);
    w.set(BlockPos::new(0, 0, 0), grass);
    let snow = w.id("snow", &[]);
    w.set(BlockPos::new(0, 1, 0), snow);
    assert_eq!(w.prop(BlockPos::new(0, 0, 0), "snowy"), "true");
    // A left chest's partner sits clockwise of its facing; removing the
    // partner makes it single again.
    let left = w.id("chest", &[("facing", "north"), ("type", "left")]);
    let right = w.id("chest", &[("facing", "north"), ("type", "right")]);
    w.set(BlockPos::new(3, 1, 0), left);
    w.set(BlockPos::new(4, 1, 0), right);
    let air = w.b().air();
    w.set(BlockPos::new(4, 1, 0), air);
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "type"), "single");
}

#[test]
fn sand_falls_and_concrete_solidifies() {
    let mut w = world!();
    let sand = w.id("sand", &[]);
    w.set(BlockPos::new(0, 5, 0), sand);
    w.tick();
    w.tick();
    assert_eq!(w.get(BlockPos::new(0, 5, 0)), w.b().air());
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::FallingBlock { state, .. } if *state == sand))
    );
    w.floor(0, 2, "stone");
    let water = w.id("water", &[]);
    w.put(BlockPos::new(1, 1, 0), water);
    let powder = w.id("white_concrete_powder", &[]);
    let landing = w.b().falling_block_landed(
        &mut w.level,
        &mut w.ticks,
        BlockPos::new(0, 1, 0),
        powder,
        false,
    );
    assert_eq!(
        landing,
        falling::Landing::Placed(w.id("white_concrete", &[]))
    );
}

#[test]
fn water_spreads_on_a_floor() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), water);
    for _ in 0..60 {
        w.tick();
    }
    for d in 1..=7 {
        let pos = BlockPos::new(d, 1, 0);
        assert_eq!(
            w.prop(pos, "level"),
            d.to_string(),
            "flow at distance {d}: {}",
            w.name(pos)
        );
    }
    assert_eq!(w.get(BlockPos::new(8, 1, 0)), w.b().air());
    // Diagonals follow Manhattan distance.
    assert_eq!(w.prop(BlockPos::new(2, 1, 2), "level"), "4");
}

#[test]
fn water_falls_and_makes_sources() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 4, 0), water);
    for _ in 0..30 {
        w.tick();
    }
    assert_eq!(
        w.prop(BlockPos::new(0, 3, 0), "level"),
        "8",
        "falling water"
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "level"), "8");
    // Two sources make a third (no holes within slope distance, so the
    // sources spread in every direction).
    let mut w = world!();
    w.floor(0, 12, "stone");
    w.set(BlockPos::new(0, 1, 0), water);
    w.set(BlockPos::new(2, 1, 0), water);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), water);
    // On a small platform the water instead seeks the nearest edges.
    let mut w = world!();
    w.floor(0, 4, "stone");
    w.set(BlockPos::new(0, 1, 0), water);
    w.set(BlockPos::new(2, 1, 0), water);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), w.b().air());
    assert_eq!(w.prop(BlockPos::new(-1, 1, 0), "level"), "1");
}

#[test]
fn lava_meets_water() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let lava = w.id("lava", &[]);
    let water = w.id("water", &[]);
    w.set(BlockPos::new(0, 1, 0), lava);
    w.set(BlockPos::new(1, 1, 0), water);
    assert_eq!(w.get(BlockPos::new(0, 1, 0)), w.id("obsidian", &[]));
    let mut w = world!();
    w.floor(0, 4, "stone");
    w.set(BlockPos::new(0, 1, 0), lava);
    for _ in 0..40 {
        w.tick();
    }
    w.set(BlockPos::new(1, 2, 0), water);
    for _ in 0..40 {
        w.tick();
    }
    assert_eq!(
        w.get(BlockPos::new(1, 1, 0)),
        w.id("cobblestone", &[]),
        "{}",
        w.name(BlockPos::new(1, 1, 0))
    );
}

#[test]
fn waterlogged_blocks_schedule_water() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let slab = w.id("oak_slab", &[("waterlogged", "true")]);
    w.set(BlockPos::new(0, 1, 0), slab);
    // A neighbour shape update schedules the slab's water tick.
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(0, 1, 1), stone);
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "level"), "1");
}

#[test]
fn redstone_lever_wire_and_lamp() {
    let mut w = world!();
    w.floor(0, 20, "stone");
    let wire = w.id("redstone_wire", &[]);
    let lamp = w.id("redstone_lamp", &[]);
    let lever = w.id("lever", &[("face", "floor"), ("facing", "north")]);
    for x in 1..=5 {
        w.set(BlockPos::new(x, 1, 0), wire);
    }
    w.set(BlockPos::new(6, 1, 0), lamp);
    w.set(BlockPos::new(0, 1, 0), lever);
    let lever_on = w
        .b()
        .table
        .set_bool(w.get(BlockPos::new(0, 1, 0)), w.b().props.powered, true);
    w.set(BlockPos::new(0, 1, 0), lever_on);
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "power"), "15");
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "power"), "11");
    assert_eq!(w.prop(BlockPos::new(6, 1, 0), "lit"), "true");
    let lever_off = w.b().table.set_bool(lever_on, w.b().props.powered, false);
    w.set(BlockPos::new(0, 1, 0), lever_off);
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "power"), "0");
    for _ in 0..5 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(6, 1, 0), "lit"), "false");
}

#[test]
fn repeater_delays_and_torch_inverts() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let block = w.id("redstone_block", &[]);
    // A repeater takes its input from the side it faces.
    let repeater = w.id("repeater", &[("facing", "west")]);
    w.set(BlockPos::new(1, 1, 0), repeater);
    w.set(BlockPos::new(0, 1, 0), block);
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "powered"), "false");
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "powered"), "true");
    let torch = w.id("redstone_torch", &[]);
    let stone = w.id("stone", &[]);
    w.set(BlockPos::new(5, 1, 0), stone);
    w.set(BlockPos::new(5, 2, 0), torch);
    // A redstone block only weakly powers the stone: the torch stays lit.
    w.set(BlockPos::new(6, 1, 0), block);
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(5, 2, 0), "lit"), "true");
    // A lever on the stone strongly powers it.
    let air = w.b().air();
    w.set(BlockPos::new(6, 1, 0), air);
    let lever = w.id(
        "lever",
        &[("face", "wall"), ("facing", "east"), ("powered", "true")],
    );
    w.set(BlockPos::new(6, 1, 0), lever);
    // LeverBlock.pull notifies the attached block's neighbours.
    let lever_block = w.b().block(lever);
    w.b().update_neighbors_at(
        &mut w.level,
        &mut w.ticks,
        BlockPos::new(5, 1, 0),
        lever_block,
    );
    w.tick();
    w.tick();
    assert_eq!(w.prop(BlockPos::new(5, 2, 0), "lit"), "false");
}

#[test]
fn crops_grow_and_leaves_decay() {
    let mut w = world!();
    let farmland = w.id("farmland", &[("moisture", "7")]);
    let wheat = w.id("wheat", &[]);
    w.put(BlockPos::new(0, 0, 0), farmland);
    w.put(BlockPos::new(0, 1, 0), wheat);
    for _ in 0..400 {
        w.b()
            .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(0, 1, 0));
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "age"), "7");
    let leaves = w.id("oak_leaves", &[("distance", "7")]);
    w.put(BlockPos::new(5, 5, 5), leaves);
    w.b()
        .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(5, 5, 5));
    assert_eq!(w.get(BlockPos::new(5, 5, 5)), w.b().air());
    let log = w.id("oak_log", &[]);
    let leaf = w.id("oak_leaves", &[]);
    w.set(BlockPos::new(9, 5, 5), leaf);
    w.set(BlockPos::new(8, 5, 5), log);
    w.tick();
    assert_eq!(w.prop(BlockPos::new(9, 5, 5), "distance"), "1");
}

impl World<'_> {
    /// Right-click `item` on the `face` of `clicked`, hitting the face centre
    /// (offset vertically by `dy`), looking with `yaw`/`pitch`.
    fn place(
        &mut self,
        item: &str,
        clicked: BlockPos,
        face: Direction,
        yaw: f32,
        pitch: f32,
        dy: f64,
    ) -> PlaceOutcome {
        let [nx, ny, nz] = face.normal();
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(clicked.x) + 0.5 + f64::from(nx) * 0.5,
            f64::from(clicked.y) + 0.5 + f64::from(ny) * 0.5 + dy,
            f64::from(clicked.z) + 0.5 + f64::from(nz) * 0.5,
        );
        let ctx = PlaceContext {
            clicked_pos: clicked,
            face,
            hit,
            inside: false,
            yaw,
            pitch,
            sneaking: false,
            item: format!("minecraft:{item}"),
            can_use_game_master_blocks: false,
        };
        self.f
            .behaviours
            .place_block(&mut self.level, &mut self.ticks, &ctx)
    }
    fn use_block(&mut self, pos: BlockPos) -> InteractionResult {
        let player = PlayerContext {
            yaw: 0.0,
            pitch: 0.0,
            may_build: true,
            can_eat: true,
            game_master: false,
        };
        let hit = horizon_core::geometry::Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y) + 0.5,
            f64::from(pos.z),
        );
        self.f.behaviours.use_without_item(
            &mut self.level,
            &mut self.ticks,
            pos,
            Direction::North,
            hit,
            player,
        )
    }
}

#[test]
fn placement_orients_blocks() {
    let mut w = world!();
    w.floor(0, 6, "stone");
    let ground = BlockPos::new(0, 0, 0);
    // Looking north (yaw 180) at the top of the floor.
    w.place("oak_stairs", ground, Direction::Up, 180.0, 30.0, 0.0);
    assert_eq!(
        w.name(BlockPos::new(0, 1, 0)),
        "oak_stairs[facing=north,half=bottom,shape=straight,waterlogged=false]"
    );
    w.place(
        "oak_log",
        BlockPos::new(1, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    w.place(
        "oak_log",
        BlockPos::new(1, 1, 0),
        Direction::East,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(2, 1, 0), "axis"), "x");
    // Torch against the log's east face becomes a wall torch facing east.
    w.place(
        "torch",
        BlockPos::new(2, 1, 0),
        Direction::East,
        270.0,
        0.0,
        0.0,
    );
    assert_eq!(w.name(BlockPos::new(3, 1, 0)), "wall_torch[facing=east]");
    // Furnaces face the player; observers face where the player looks.
    w.place(
        "furnace",
        BlockPos::new(-2, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-2, 1, 0), "facing"), "north");
    w.place(
        "observer",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        0.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 1, 0), "facing"), "south");
}

#[test]
fn placement_merges_slabs_and_stacks() {
    let mut w = world!();
    w.floor(0, 4, "stone");
    let ground = BlockPos::new(0, 0, 0);
    w.place("oak_slab", ground, Direction::Up, 0.0, 60.0, 0.0);
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "type"), "bottom");
    w.place(
        "oak_slab",
        BlockPos::new(0, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.5,
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "type"), "double");
    // Clicking the upper half of a side face places a top slab.
    w.place(
        "oak_slab",
        BlockPos::new(0, 1, 0),
        Direction::East,
        0.0,
        0.0,
        0.25,
    );
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "type"), "top");
    w.place(
        "snow",
        BlockPos::new(3, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.place(
        "snow",
        BlockPos::new(3, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.375,
    );
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "layers"), "2");
    w.place(
        "candle",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.place(
        "candle",
        BlockPos::new(-3, 1, 0),
        Direction::Up,
        0.0,
        60.0,
        -0.4,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 1, 0), "candles"), "2");
}

#[test]
fn doors_beds_and_plants_place_both_halves() {
    let mut w = world!();
    w.floor(0, 6, "grass_block");
    let ground = BlockPos::new(0, 0, 0);
    w.place("oak_door", ground, Direction::Up, 180.0, 30.0, 0.0);
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "half"), "lower");
    assert_eq!(w.prop(BlockPos::new(0, 2, 0), "half"), "upper");
    assert_eq!(
        w.use_block(BlockPos::new(0, 1, 0)),
        InteractionResult::Success
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "open"), "true");
    assert_eq!(
        w.prop(BlockPos::new(0, 2, 0), "open"),
        "true",
        "the upper half follows"
    );
    w.place(
        "red_bed",
        BlockPos::new(3, 0, 0),
        Direction::Up,
        180.0,
        30.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "part"), "foot");
    assert_eq!(w.prop(BlockPos::new(3, 1, -1), "part"), "head");
    w.place(
        "sunflower",
        BlockPos::new(-3, 0, 0),
        Direction::Up,
        0.0,
        30.0,
        0.0,
    );
    assert_eq!(w.prop(BlockPos::new(-3, 2, 0), "half"), "upper");
    // Iron doors cannot be opened by hand.
    w.place(
        "iron_door",
        BlockPos::new(5, 0, 0),
        Direction::Up,
        180.0,
        30.0,
        0.0,
    );
    assert_eq!(w.use_block(BlockPos::new(5, 1, 0)), InteractionResult::Pass);
    // A placement into a solid block fails.
    assert_eq!(
        w.place(
            "stone",
            BlockPos::new(0, 0, 0),
            Direction::West,
            0.0,
            0.0,
            0.0
        ),
        PlaceOutcome::Failed
    );
}

#[test]
fn levers_power_wire_through_interaction() {
    let mut w = world!();
    w.floor(0, 8, "stone");
    w.place(
        "lever",
        BlockPos::new(0, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    for x in 1..=3 {
        w.place(
            "redstone",
            BlockPos::new(x, 0, 0),
            Direction::Up,
            0.0,
            60.0,
            0.0,
        );
    }
    assert_eq!(
        w.use_block(BlockPos::new(0, 1, 0)),
        InteractionResult::Success
    );
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "powered"), "true");
    assert_eq!(w.prop(BlockPos::new(3, 1, 0), "power"), "13");
    w.use_block(BlockPos::new(0, 1, 0));
    assert_eq!(w.prop(BlockPos::new(1, 1, 0), "power"), "0");
    // Buttons release after their pressed time.
    w.place(
        "stone_button",
        BlockPos::new(5, 0, 0),
        Direction::Up,
        0.0,
        60.0,
        0.0,
    );
    w.use_block(BlockPos::new(5, 1, 0));
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "powered"), "true");
    for _ in 0..20 {
        w.tick();
    }
    assert_eq!(w.prop(BlockPos::new(5, 1, 0), "powered"), "false");
}

#[test]
fn pistons_push_and_sticky_pistons_pull() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("sticky_piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    let dirt = w.id("dirt", &[]);
    let air = w.b().air();
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), dirt);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "true");
    assert_eq!(
        w.name(BlockPos::new(1, 1, 0)),
        "piston_head[facing=east,short=false,type=sticky]"
    );
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), dirt);
    assert!(w.ticks.moving_pistons.is_empty());
    w.set(BlockPos::new(-1, 1, 0), air);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "false");
    assert_eq!(
        w.get(BlockPos::new(1, 1, 0)),
        dirt,
        "the sticky piston pulled the block back"
    );
    assert_eq!(w.get(BlockPos::new(2, 1, 0)), air);
}

#[test]
fn pistons_pop_fragile_blocks_and_refuse_obsidian() {
    let mut w = world!();
    w.floor(0, 10, "stone");
    let piston = w.id("piston", &[("facing", "east")]);
    let block = w.id("redstone_block", &[]);
    let torch = w.id("torch", &[]);
    let obsidian = w.id("obsidian", &[]);
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), torch);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(
        w.name(BlockPos::new(1, 1, 0)),
        "piston_head[facing=east,short=false,type=normal]"
    );
    assert!(
        w.level
            .events
            .iter()
            .any(|e| matches!(e, LevelEvent::DropResources { state, .. } if *state == torch))
    );
    let mut w = world!();
    w.floor(0, 10, "stone");
    w.set(BlockPos::new(0, 1, 0), piston);
    w.set(BlockPos::new(1, 1, 0), obsidian);
    w.set(BlockPos::new(-1, 1, 0), block);
    for _ in 0..4 {
        w.full_tick();
    }
    assert_eq!(w.prop(BlockPos::new(0, 1, 0), "extended"), "false");
    assert_eq!(w.get(BlockPos::new(1, 1, 0)), obsidian);
}

#[test]
fn dripstone_fills_cauldrons() {
    let mut w = world!();
    let block = w.id("dripstone_block", &[]);
    let water = w.id("water", &[]);
    let tip = w.id(
        "pointed_dripstone",
        &[("vertical_direction", "down"), ("thickness", "tip")],
    );
    let cauldron = w.id("cauldron", &[]);
    w.put(BlockPos::new(0, 11, 0), water);
    w.put(BlockPos::new(0, 10, 0), block);
    w.put(BlockPos::new(0, 9, 0), tip);
    w.put(BlockPos::new(0, 5, 0), cauldron);
    for _ in 0..100 {
        w.b()
            .random_tick(&mut w.level, &mut w.ticks, BlockPos::new(0, 9, 0));
    }
    for _ in 0..80 {
        w.tick();
    }
    assert_eq!(w.name(BlockPos::new(0, 5, 0)), "water_cauldron[level=1]");
}

#[test]
fn dispenser_buckets_pick_up_and_empty() {
    let mut w = world!();
    let at = BlockPos::new(0, 10, 0);
    let water = w.id("water", &[("level", "0")]);
    w.put(at, water);
    let picked = w.b().bucket_pickup(&mut w.level, &mut w.ticks, at);
    assert_eq!(picked, Some("minecraft:water_bucket"));
    assert_eq!(w.get(at), w.b().air());
    // Flowing water gives nothing.
    let flowing = w.id("water", &[("level", "3")]);
    w.put(at, flowing);
    assert_eq!(w.b().bucket_pickup(&mut w.level, &mut w.ticks, at), None);
    // A waterlogged slab is drained and stays a slab.
    let slab = w.id("oak_slab", &[("waterlogged", "true")]);
    w.put(at, slab);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at),
        Some("minecraft:water_bucket")
    );
    assert_eq!(w.prop(at, "waterlogged"), "false");
    // Powder snow comes up as its bucket.
    let snow = w.id("powder_snow", &[]);
    w.put(at, snow);
    assert_eq!(
        w.b().bucket_pickup(&mut w.level, &mut w.ticks, at),
        Some("minecraft:powder_snow_bucket")
    );
    // Emptying water into air makes a source; stone refuses.
    let sound = "minecraft:item.bucket.empty";
    let content = BucketContent::Fluid(crate::fluid::FluidKind::Water);
    assert!(
        w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound)
    );
    assert_eq!(w.name(at), "water[level=0]");
    let stone = w.id("stone", &[]);
    w.put(at, stone);
    assert!(
        !w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound)
    );
    // Water fills a waterloggable slab instead of replacing it.
    let dry = w.id("oak_slab", &[]);
    w.put(at, dry);
    assert!(
        w.b()
            .bucket_empty(&mut w.level, &mut w.ticks, at, content, sound)
    );
    assert_eq!(w.prop(at, "waterlogged"), "true");
    // Powder snow only goes into empty space.
    let air = w.b().air();
    w.put(at, air);
    assert!(w.b().bucket_empty(
        &mut w.level,
        &mut w.ticks,
        at,
        BucketContent::Powder,
        "minecraft:item.bucket.empty_powder_snow"
    ));
    assert_eq!(w.name(at), "powder_snow[]");
}
