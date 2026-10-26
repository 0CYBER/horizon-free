//! Client-side ambient block effects: `ClientLevel.animateTick` (667 random
//! blocks within 16 and 32 blocks per tick calling `Block.animateTick` and
//! `FluidState.animateTick`, fluid drips and biome ambient particles) and the
//! campfire block-entity particle tick. Particle positions, odds and sounds
//! follow the vanilla block classes.
use crate::{
    biome::Biomes,
    blocks::{BlockFacts, Fluid},
    command::args::GameMode,
    items::ItemStack,
    particles::{ParticleEngine, ParticleOptions, ParticleType as T},
    world::World,
};
use horizon_core::{
    geometry::Axis,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    block_collision::BlockCollisionRegistry,
    registry::{BlockRegistry, BlockStateId},
    worldgen::tags::TagDefinitions,
};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Copy, Debug, PartialEq)]
enum Animate {
    None,
    Torch(T),
    WallTorch(T, [i32; 3]),
    RedstoneTorch,
    RedstoneWallTorch([i32; 3]),
    Furnace([i32; 3]),
    BlastFurnace([i32; 3]),
    Smoker,
    Campfire {
        spawn_particles: bool,
    },
    /// Leaves: optional falling-leaf particle and chance, tinted by foliage.
    Leaves(Option<(T, f32)>),
    NetherPortal,
    EnchantingTable,
    EndRod([i32; 3]),
    SporeBlossom,
    CryingObsidian,
    Mycelium,
    Candles(u8),
    CandleCake,
    BrewingStand,
    EnderChest,
    /// FallingBlock with its dust colour.
    Falling(u32),
    RedstoneOre,
    RedstoneWire {
        power: u8,
        sides: [u8; 4],
    },
    RespawnAnchor,
    BubbleColumn {
        drag: bool,
    },
    Fire,
    /// An active sculk sensor.
    SculkSensor,
    /// Potent sulfur that is not dry.
    PotentSulfur,
    DriedGhast {
        waterlogged: bool,
    },
    EndPortal,
}
fn step(facing: &str) -> [i32; 3] {
    match facing {
        "north" => [0, 0, -1],
        "south" => [0, 0, 1],
        "west" => [-1, 0, 0],
        "east" => [1, 0, 0],
        "up" => [0, 1, 0],
        _ => [0, -1, 0],
    }
}
fn classify(name: &str, prop: &dyn Fn(&str) -> Option<String>) -> Animate {
    let name = name.strip_prefix("minecraft:").unwrap_or(name);
    let lit = prop("lit").as_deref() == Some("true");
    let facing = || step(&prop("facing").unwrap_or_default());
    match name {
        "torch" => Animate::Torch(T::Flame),
        "soul_torch" => Animate::Torch(T::SoulFireFlame),
        "copper_torch" => Animate::Torch(T::CopperFireFlame),
        "wall_torch" => Animate::WallTorch(T::Flame, facing()),
        "soul_wall_torch" => Animate::WallTorch(T::SoulFireFlame, facing()),
        "copper_wall_torch" => Animate::WallTorch(T::CopperFireFlame, facing()),
        "redstone_torch" if lit => Animate::RedstoneTorch,
        "redstone_wall_torch" if lit => Animate::RedstoneWallTorch(facing()),
        "furnace" if lit => Animate::Furnace(facing()),
        "blast_furnace" if lit => Animate::BlastFurnace(facing()),
        "smoker" if lit => Animate::Smoker,
        "campfire" if lit => Animate::Campfire {
            spawn_particles: true,
        },
        "soul_campfire" if lit => Animate::Campfire {
            spawn_particles: false,
        },
        "oak_leaves" | "birch_leaves" | "jungle_leaves" | "acacia_leaves" | "dark_oak_leaves"
        | "mangrove_leaves" => Animate::Leaves(Some((T::TintedLeaves, 0.01))),
        "cherry_leaves" => Animate::Leaves(Some((T::CherryLeaves, 0.1))),
        "pale_oak_leaves" => Animate::Leaves(Some((T::PaleOakLeaves, 0.02))),
        "spruce_leaves" | "azalea_leaves" | "flowering_azalea_leaves" => Animate::Leaves(None),
        "nether_portal" => Animate::NetherPortal,
        "enchanting_table" => Animate::EnchantingTable,
        "end_rod" => Animate::EndRod(facing()),
        "spore_blossom" => Animate::SporeBlossom,
        "crying_obsidian" => Animate::CryingObsidian,
        "mycelium" => Animate::Mycelium,
        "brewing_stand" => Animate::BrewingStand,
        "ender_chest" => Animate::EnderChest,
        "sand" => Animate::Falling(0xDBD3A0),
        "red_sand" => Animate::Falling(0xA95821),
        "gravel" => Animate::Falling(0x807C7B),
        "redstone_ore" | "deepslate_redstone_ore" if lit => Animate::RedstoneOre,
        "redstone_wire" => {
            let power = prop("power").and_then(|p| p.parse().ok()).unwrap_or(0);
            let side = |d: &str| match prop(d).as_deref() {
                Some("up") => 2,
                Some("side") => 1,
                _ => 0,
            };
            if power == 0 {
                Animate::None
            } else {
                Animate::RedstoneWire {
                    power,
                    sides: [side("north"), side("east"), side("south"), side("west")],
                }
            }
        }
        "respawn_anchor" if prop("charges").is_some_and(|c| c != "0") => Animate::RespawnAnchor,
        "potent_sulfur" if prop("potent_sulfur_state").is_some_and(|s| s != "dry") => {
            Animate::PotentSulfur
        }
        "bubble_column" => Animate::BubbleColumn {
            drag: prop("drag").as_deref() == Some("true"),
        },
        "fire" | "soul_fire" => Animate::Fire,
        "dried_ghast" => Animate::DriedGhast {
            waterlogged: prop("waterlogged").as_deref() == Some("true"),
        },
        "end_portal" => Animate::EndPortal,
        "sculk_sensor" | "calibrated_sculk_sensor"
            if prop("sculk_sensor_phase").as_deref() == Some("active") =>
        {
            Animate::SculkSensor
        }
        _ if name.ends_with("candle_cake") && lit => Animate::CandleCake,
        _ if (name == "candle" || name.ends_with("_candle")) && lit => {
            Animate::Candles(prop("candles").and_then(|c| c.parse().ok()).unwrap_or(1))
        }
        _ => Animate::None,
    }
}
/// `RedstoneWireBlock.COLORS`.
fn redstone_color(power: u8) -> [f32; 3] {
    let f = f32::from(power) / 15.0;
    let r = f * 0.6 + if f > 0.0 { 0.4 } else { 0.3 };
    let g = (f * f * 0.7 - 0.5).clamp(0.0, 1.0);
    let b = (f * f * 0.6 - 0.7).clamp(0.0, 1.0);
    // DustParticleOptions stores the packed int colour.
    let q = |c: f32| ((c * 255.0) as u8) as f32 / 255.0;
    [q(r), q(g), q(b)]
}
/// `CandleBlock.PARTICLE_OFFSETS` by candle count (pixels).
fn candle_offsets(count: u8) -> &'static [[f64; 3]] {
    match count {
        1 => &[[8.0, 8.0, 8.0]],
        2 => &[[6.0, 7.0, 8.0], [10.0, 8.0, 7.0]],
        3 => &[[8.0, 5.0, 10.0], [6.0, 7.0, 8.0], [9.0, 8.0, 7.0]],
        _ => &[
            [7.0, 5.0, 9.0],
            [10.0, 7.0, 9.0],
            [6.0, 7.0, 6.0],
            [9.0, 8.0, 6.0],
        ],
    }
}

/// Per-state animate behaviour and the block tags it needs.
pub struct AnimateTables {
    kinds: Vec<Animate>,
    impermeable: Vec<bool>,
    power_provider: Vec<bool>,
    power_transmitter: Vec<bool>,
    /// `#triggers_ambient_dried_ghast_block_sounds` (soul sand, soul soil).
    dried_ghast_sounds: Vec<bool>,
    /// Biome id -> ambient particles and probabilities (`minecraft:visual/ambient_particles`).
    ambient: Vec<Vec<(T, f32)>>,
}
impl AnimateTables {
    pub fn load(root: &Path, registry: &BlockRegistry, biomes: &Biomes) -> crate::Result<Self> {
        let kinds = (0..registry.len() as u32)
            .map(|id| {
                registry.state(BlockStateId(id)).map_or(Animate::None, |s| {
                    classify(&s.name, &|k| s.properties.get(k).cloned())
                })
            })
            .collect();
        let bundle: BTreeMap<String, String> =
            serde_json::from_slice(&std::fs::read(root.join("block-tags.json"))?)?;
        let mut definitions = TagDefinitions::default();
        for (name, text) in bundle {
            definitions.apply_json(&name, "vanilla", &text)?;
        }
        let built =
            definitions.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
        let by_state = |tag: &str| -> Vec<bool> {
            let names = built.tags.get(tag);
            (0..registry.len() as u32)
                .map(|id| {
                    registry
                        .state(BlockStateId(id))
                        .is_some_and(|s| names.is_some_and(|n| n.contains(&s.name)))
                })
                .collect()
        };
        let mut ambient = vec![Vec::new(); biomes.list.len()];
        for (name, &id) in &biomes.index {
            let path = root.join(format!(
                "data/minecraft/worldgen/biome/{}.json",
                name.trim_start_matches("minecraft:")
            ));
            let Ok(bytes) = std::fs::read(path) else {
                continue;
            };
            let Ok(json) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
                continue;
            };
            let list = &json["attributes"]["minecraft:visual/ambient_particles"];
            let entries = list["argument"].as_array().or_else(|| list.as_array());
            for entry in entries.into_iter().flatten() {
                let ty = entry["particle"]["type"].as_str().and_then(T::from_name);
                let probability = entry["probability"].as_f64().unwrap_or(0.0) as f32;
                if let (Some(ty), Some(slot)) = (ty, ambient.get_mut(id as usize)) {
                    slot.push((ty, probability));
                }
            }
        }
        Ok(Self {
            kinds,
            impermeable: by_state("minecraft:impermeable"),
            power_provider: by_state("minecraft:enchantment_power_provider"),
            power_transmitter: by_state("minecraft:enchantment_power_transmitter"),
            dried_ghast_sounds: by_state("minecraft:triggers_ambient_dried_ghast_block_sounds"),
            ambient,
        })
    }
    fn kind(&self, state: BlockStateId) -> Animate {
        self.kinds
            .get(state.0 as usize)
            .copied()
            .unwrap_or(Animate::None)
    }
    /// Whether a state ticks campfire particles (`CampfireBlockEntity.particleTick`).
    pub fn is_lit_campfire(&self, state: u32) -> bool {
        matches!(self.kind(BlockStateId(state)), Animate::Campfire { .. })
    }
}

/// World access for ambient effects.
pub struct AnimateContext<'a> {
    pub world: &'a World,
    pub facts: &'a BlockFacts,
    pub collision: &'a BlockCollisionRegistry<'a>,
    pub biomes: &'a Biomes,
    pub registry: &'a BlockRegistry,
    /// `Level.isRainingAt`.
    pub raining_at: &'a dyn Fn([i32; 3]) -> bool,
    /// Creative main-hand marker block and a resolver for each placed state's sprite.
    pub marker: Option<(&'a str, &'a dyn Fn(BlockStateId) -> Option<ParticleOptions>)>,
}

/// `ClientLevel.getMarkerParticleTarget`: only the selected main-hand item counts.
pub fn marker_particle_target(
    mode: GameMode,
    main_hand: Option<&ItemStack>,
) -> Option<&'static str> {
    if mode != GameMode::Creative {
        return None;
    }
    let stack = main_hand.filter(|stack| stack.count > 0)?;
    match stack.item.as_ref() {
        "minecraft:barrier" => Some("minecraft:barrier"),
        "minecraft:light" => Some("minecraft:light"),
        _ => None,
    }
}
impl AnimateContext<'_> {
    fn state(&self, p: [i32; 3]) -> BlockStateId {
        self.world.block(p[0], p[1], p[2])
    }
    fn is_air(&self, p: [i32; 3]) -> bool {
        self.facts.is_air(self.state(p))
    }
    /// `isFaceSturdy` approximated by a full collision cube.
    fn sturdy(&self, p: [i32; 3]) -> bool {
        self.facts.get(self.state(p)).full_collision
    }
    fn shape_y(&self, p: [i32; 3]) -> Option<(f64, f64, [f64; 4])> {
        let shape = self
            .collision
            .cached_shapes()
            .cached_shape(self.state(p))
            .ok()??;
        if shape.is_empty() {
            return Some((0.0, 0.0, [0.0; 4]));
        }
        Some((
            shape.min(Axis::Y),
            shape.max(Axis::Y),
            [
                shape.min(Axis::X),
                shape.max(Axis::X),
                shape.min(Axis::Z),
                shape.max(Axis::Z),
            ],
        ))
    }
    fn foliage(&self, p: [i32; 3]) -> u32 {
        let biome = self
            .world
            .chunks
            .get(&(p[0] >> 4, p[2] >> 4))
            .map_or(0, |c| c.biome(p[0], p[1], p[2]));
        self.biomes.get(biome).foliage
    }
}

fn add(engine: &mut ParticleEngine, ty: T, p: [f64; 3], v: [f64; 3]) {
    engine.add(&ParticleOptions::simple(ty), p, v);
}
fn sound(engine: &mut ParticleEngine, name: &'static str, p: [f64; 3], volume: f32, pitch: f32) {
    engine.sounds.push((name, p, volume, pitch));
}
fn redstone(engine: &mut ParticleEngine, p: [f64; 3]) {
    engine.add(&ParticleOptions::dust([1.0, 0.0, 0.0], 1.0), p, [0.0; 3]);
}

/// `ClientLevel.animateTick` around the player's block position.
pub fn animate_tick(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    player: [i32; 3],
) {
    for _ in 0..667 {
        do_animate_tick(tables, engine, ctx, player, 16);
        do_animate_tick(tables, engine, ctx, player, 32);
    }
}
fn next_int(r: &mut LegacyRandom, bound: i32) -> i32 {
    r.next_int_bound(bound).unwrap_or(0)
}
fn do_animate_tick(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    center: [i32; 3],
    r: i32,
) {
    let random = engine.random();
    let x = center[0] + next_int(random, r) - next_int(random, r);
    let y = center[1] + next_int(random, r) - next_int(random, r);
    let z = center[2] + next_int(random, r) - next_int(random, r);
    let pos = [x, y, z];
    if !ctx.world.loaded(x, z) {
        return;
    }
    let state = ctx.state(pos);
    block_animate_tick(tables, engine, ctx, state, pos);
    let facts = ctx.facts.get(state);
    if facts.fluid != Fluid::None {
        fluid_animate_tick(
            engine,
            ctx,
            pos,
            facts.fluid,
            facts.fluid_level,
            facts.fluid_falling,
        );
        if next_int(engine.random(), 10) == 0 {
            let drip = if facts.fluid == Fluid::Water {
                T::DrippingWater
            } else {
                T::DrippingLava
            };
            let top_solid = ctx.sturdy(pos);
            try_spawn_drip(tables, engine, ctx, [x, y - 1, z], drip, top_solid);
        }
    }
    if let Some((block, options)) = ctx.marker
        && ctx.registry.state(state).is_some_and(|s| s.name == block)
        && let Some(options) = options(state)
    {
        engine.add(&options, pos.map(|v| f64::from(v) + 0.5), [0.0; 3]);
    }
    if !facts.full_collision {
        let biome = ctx
            .world
            .chunks
            .get(&(x >> 4, z >> 4))
            .map_or(0, |c| c.biome(x, y, z));
        for &(ty, probability) in tables.ambient.get(biome as usize).into_iter().flatten() {
            // AmbientParticle.canSpawn: random.nextFloat() <= probability.
            let random = engine.random();
            if random.next_float() <= probability {
                let p = [
                    f64::from(x) + random.next_double(),
                    f64::from(y) + random.next_double(),
                    f64::from(z) + random.next_double(),
                ];
                add(engine, ty, p, [0.0; 3]);
            }
        }
    }
}
/// `ClientLevel.trySpawnDripParticles`.
fn try_spawn_drip(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    pos: [i32; 3],
    drip: T,
    top_solid: bool,
) {
    let state = ctx.state(pos);
    if ctx.facts.get(state).fluid != Fluid::None {
        return;
    }
    let Some((bottom, top, bounds)) = ctx.shape_y(pos) else {
        return;
    };
    let spawn = |engine: &mut ParticleEngine, x1: f64, x2: f64, z1: f64, z2: f64, y: f64| {
        let r = engine.random();
        let (a, b) = (r.next_double(), r.next_double());
        add(
            engine,
            drip,
            [x1 + (x2 - x1) * a, y, z1 + (z2 - z1) * b],
            [0.0; 3],
        );
    };
    let (fx, fy, fz) = (f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    if top < 1.0 {
        if top_solid {
            spawn(engine, fx, fx + 1.0, fz, fz + 1.0, fy + 1.0 - 0.05);
        }
    } else if !tables
        .impermeable
        .get(state.0 as usize)
        .copied()
        .unwrap_or(false)
    {
        if bottom > 0.0 {
            spawn(
                engine,
                fx + bounds[0],
                fx + bounds[1],
                fz + bounds[2],
                fz + bounds[3],
                fy + bottom - 0.05,
            );
        } else {
            let below = [pos[0], pos[1] - 1, pos[2]];
            let below_top = ctx.shape_y(below).map_or(1.0, |s| s.1);
            if below_top < 1.0 && ctx.facts.get(ctx.state(below)).fluid == Fluid::None {
                spawn(
                    engine,
                    fx + bounds[0],
                    fx + bounds[1],
                    fz + bounds[2],
                    fz + bounds[3],
                    fy - 0.05,
                );
            }
        }
    }
}
/// `WaterFluid.animateTick` / `LavaFluid.animateTick`.
fn fluid_animate_tick(
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    pos: [i32; 3],
    fluid: Fluid,
    level: u8,
    falling: bool,
) {
    let (fx, fy, fz) = (f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    if fluid == Fluid::Water {
        let source = level >= 8 && !falling;
        let random = engine.random();
        if !source && !falling {
            if next_int(random, 64) == 0 {
                let volume = random.next_float() * 0.25 + 0.75;
                let pitch = random.next_float() + 0.5;
                sound(
                    engine,
                    "minecraft:block.water.ambient",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    volume,
                    pitch,
                );
            }
        } else if next_int(random, 10) == 0 {
            let p = [
                fx + random.next_double(),
                fy + random.next_double(),
                fz + random.next_double(),
            ];
            add(engine, T::Underwater, p, [0.0; 3]);
        }
        return;
    }
    let above = [pos[0], pos[1] + 1, pos[2]];
    if ctx.is_air(above) {
        let random = engine.random();
        if next_int(random, 100) == 0 {
            let p = [
                fx + random.next_double(),
                fy + 1.0,
                fz + random.next_double(),
            ];
            add(engine, T::Lava, p, [0.0; 3]);
            let random = engine.random();
            let (volume, pitch) = (
                0.2 + random.next_float() * 0.2,
                0.9 + random.next_float() * 0.15,
            );
            sound(engine, "minecraft:block.lava.pop", p, volume, pitch);
        }
        let random = engine.random();
        if next_int(random, 200) == 0 {
            let (volume, pitch) = (
                0.2 + random.next_float() * 0.2,
                0.9 + random.next_float() * 0.15,
            );
            sound(
                engine,
                "minecraft:block.lava.ambient",
                [fx, fy, fz],
                volume,
                pitch,
            );
        }
    }
}
#[allow(clippy::too_many_lines)]
fn block_animate_tick(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    state: BlockStateId,
    pos: [i32; 3],
) {
    let (fx, fy, fz) = (f64::from(pos[0]), f64::from(pos[1]), f64::from(pos[2]));
    match tables.kind(state) {
        Animate::None => {}
        Animate::Torch(flame) => {
            let p = [fx + 0.5, fy + 0.7, fz + 0.5];
            add(engine, T::Smoke, p, [0.0; 3]);
            add(engine, flame, p, [0.0; 3]);
        }
        Animate::WallTorch(flame, facing) => {
            let p = [
                fx + 0.5 - 0.27 * f64::from(facing[0]),
                fy + 0.7 + 0.22,
                fz + 0.5 - 0.27 * f64::from(facing[2]),
            ];
            add(engine, T::Smoke, p, [0.0; 3]);
            add(engine, flame, p, [0.0; 3]);
        }
        Animate::RedstoneTorch => {
            let r = engine.random();
            let p = [
                fx + 0.5 + (r.next_double() - 0.5) * 0.2,
                fy + 0.7 + (r.next_double() - 0.5) * 0.2,
                fz + 0.5 + (r.next_double() - 0.5) * 0.2,
            ];
            redstone(engine, p);
        }
        Animate::RedstoneWallTorch(facing) => {
            let r = engine.random();
            let p = [
                fx + 0.5 + (r.next_double() - 0.5) * 0.2 - 0.27 * f64::from(facing[0]),
                fy + 0.7 + (r.next_double() - 0.5) * 0.2 + 0.22,
                fz + 0.5 + (r.next_double() - 0.5) * 0.2 - 0.27 * f64::from(facing[2]),
            ];
            redstone(engine, p);
        }
        Animate::Furnace(facing) | Animate::BlastFurnace(facing) => {
            let blast = matches!(tables.kind(state), Animate::BlastFurnace(_));
            let (x, y, z) = (fx + 0.5, fy, fz + 0.5);
            let r = engine.random();
            if r.next_double() < 0.1 {
                let name = if blast {
                    "minecraft:block.blastfurnace.fire_crackle"
                } else {
                    "minecraft:block.furnace.fire_crackle"
                };
                sound(engine, name, [x, y, z], 1.0, 1.0);
            }
            let r = engine.random();
            let ss = r.next_double() * 0.6 - 0.3;
            let dx = if facing[0] != 0 {
                f64::from(facing[0]) * 0.52
            } else {
                ss
            };
            let dy = r.next_double() * if blast { 9.0 } else { 6.0 } / 16.0;
            let dz = if facing[2] != 0 {
                f64::from(facing[2]) * 0.52
            } else {
                ss
            };
            let p = [x + dx, y + dy, z + dz];
            add(engine, T::Smoke, p, [0.0; 3]);
            if !blast {
                add(engine, T::Flame, p, [0.0; 3]);
            }
        }
        Animate::Smoker => {
            let r = engine.random();
            if r.next_double() < 0.1 {
                sound(
                    engine,
                    "minecraft:block.smoker.smoke",
                    [fx + 0.5, fy, fz + 0.5],
                    1.0,
                    1.0,
                );
            }
            add(engine, T::Smoke, [fx + 0.5, fy + 1.1, fz + 0.5], [0.0; 3]);
        }
        Animate::Campfire { spawn_particles } => {
            let r = engine.random();
            if next_int(r, 10) == 0 {
                let (volume, pitch) = (0.5 + r.next_float(), r.next_float() * 0.7 + 0.6);
                sound(
                    engine,
                    "minecraft:block.campfire.crackle",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    volume,
                    pitch,
                );
            }
            let r = engine.random();
            if spawn_particles && next_int(r, 5) == 0 {
                let count = next_int(r, 1) + 1;
                for _ in 0..count {
                    let r = engine.random();
                    let v = [
                        f64::from(r.next_float() / 2.0),
                        5.0e-5,
                        f64::from(r.next_float() / 2.0),
                    ];
                    add(engine, T::Lava, [fx + 0.5, fy + 0.5, fz + 0.5], v);
                }
            }
        }
        Animate::Leaves(falling) => {
            let below = [pos[0], pos[1] - 1, pos[2]];
            // LeavesBlock.makeDrippingWaterParticles.
            if (ctx.raining_at)([pos[0], pos[1] + 1, pos[2]]) && next_int(engine.random(), 15) == 1
            {
                let below_facts = ctx.facts.get(ctx.state(below));
                if !below_facts.can_occlude || !ctx.sturdy(below) {
                    let r = engine.random();
                    let p = [fx + r.next_double(), fy - 0.05, fz + r.next_double()];
                    add(engine, T::DrippingWater, p, [0.0; 3]);
                }
            }
            // FallingParticlesLeavesBlock.makeFallingLeavesParticles.
            if let Some((ty, chance)) = falling
                && engine.random().next_float() < chance
                && !ctx.sturdy(below)
            {
                let r = engine.random();
                let p = [fx + r.next_double(), fy - 0.05, fz + r.next_double()];
                let options = if ty == T::TintedLeaves {
                    let c = ctx.foliage(pos);
                    let f = |s: u32| ((c >> s) & 255) as f32 / 255.0;
                    ParticleOptions::colored(ty, [f(16), f(8), f(0), 1.0])
                } else {
                    ParticleOptions::simple(ty)
                };
                engine.add(&options, p, [0.0; 3]);
            }
        }
        Animate::NetherPortal => {
            let r = engine.random();
            if next_int(r, 100) == 0 {
                let pitch = r.next_float() * 0.4 + 0.8;
                sound(
                    engine,
                    "minecraft:block.portal.ambient",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    0.5,
                    pitch,
                );
            }
            let along_z = ctx.state([pos[0] - 1, pos[1], pos[2]]) == state
                || ctx.state([pos[0] + 1, pos[1], pos[2]]) == state;
            for _ in 0..4 {
                let r = engine.random();
                let mut p = [
                    fx + r.next_double(),
                    fy + r.next_double(),
                    fz + r.next_double(),
                ];
                let mut v = [
                    f64::from(r.next_float() - 0.5) * 0.5,
                    f64::from(r.next_float() - 0.5) * 0.5,
                    f64::from(r.next_float() - 0.5) * 0.5,
                ];
                let flip = f64::from(next_int(r, 2) * 2 - 1);
                if along_z {
                    p[2] = fz + 0.5 + 0.25 * flip;
                    v[2] = f64::from(r.next_float() * 2.0) * flip;
                } else {
                    p[0] = fx + 0.5 + 0.25 * flip;
                    v[0] = f64::from(r.next_float() * 2.0) * flip;
                }
                add(engine, T::Portal, p, v);
            }
        }
        Animate::EnchantingTable => {
            for ox in -2i32..=2 {
                for oy in 0..=1 {
                    for oz in -2i32..=2 {
                        if ox.abs() != 2 && oz.abs() != 2 {
                            continue;
                        }
                        if next_int(engine.random(), 16) != 0 {
                            continue;
                        }
                        let provider = ctx.state([pos[0] + ox, pos[1] + oy, pos[2] + oz]);
                        let between = ctx.state([pos[0] + ox / 2, pos[1] + oy, pos[2] + oz / 2]);
                        if tables
                            .power_provider
                            .get(provider.0 as usize)
                            .copied()
                            .unwrap_or(false)
                            && tables
                                .power_transmitter
                                .get(between.0 as usize)
                                .copied()
                                .unwrap_or(false)
                        {
                            let r = engine.random();
                            let v = [
                                f64::from(ox as f32 + r.next_float() - 0.5),
                                f64::from(oy as f32 - r.next_float() - 1.0),
                                f64::from(oz as f32 + r.next_float() - 0.5),
                            ];
                            add(engine, T::Enchant, [fx + 0.5, fy + 2.0, fz + 0.5], v);
                        }
                    }
                }
            }
        }
        Animate::EndRod(facing) => {
            let r = engine.random();
            let x = fx + 0.55 - f64::from(r.next_float() * 0.1);
            let y = fy + 0.55 - f64::from(r.next_float() * 0.1);
            let z = fz + 0.55 - f64::from(r.next_float() * 0.1);
            let d = f64::from(0.4 - (r.next_float() + r.next_float()) * 0.4);
            if next_int(r, 5) == 0 {
                let v = [
                    r.next_gaussian() * 0.005,
                    r.next_gaussian() * 0.005,
                    r.next_gaussian() * 0.005,
                ];
                let p = [
                    x + f64::from(facing[0]) * d,
                    y + f64::from(facing[1]) * d,
                    z + f64::from(facing[2]) * d,
                ];
                add(engine, T::EndRod, p, v);
            }
        }
        Animate::SporeBlossom => {
            let r = engine.random();
            let p = [fx + r.next_double(), fy + 0.7, fz + r.next_double()];
            add(engine, T::FallingSporeBlossom, p, [0.0; 3]);
            for _ in 0..14 {
                let r = engine.random();
                let a = [
                    pos[0] + next_int(r, 21) - 10,
                    pos[1] - next_int(r, 10),
                    pos[2] + next_int(r, 21) - 10,
                ];
                if !ctx.facts.get(ctx.state(a)).full_collision {
                    let r = engine.random();
                    let p = [
                        f64::from(a[0]) + r.next_double(),
                        f64::from(a[1]) + r.next_double(),
                        f64::from(a[2]) + r.next_double(),
                    ];
                    add(engine, T::SporeBlossomAir, p, [0.0; 3]);
                }
            }
        }
        Animate::CryingObsidian => {
            let r = engine.random();
            if next_int(r, 5) == 0 {
                // Direction.getRandom: down, up, north, south, west, east.
                let d = [
                    [0, -1, 0],
                    [0, 1, 0],
                    [0, 0, -1],
                    [0, 0, 1],
                    [-1, 0, 0],
                    [1, 0, 0],
                ][next_int(r, 6) as usize];
                if d != [0, 1, 0] {
                    let relative = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                    if !ctx.sturdy(relative) {
                        let r = engine.random();
                        let off = |s: i32, r: &mut LegacyRandom| {
                            if s == 0 {
                                r.next_double()
                            } else {
                                0.5 + f64::from(s) * 0.6
                            }
                        };
                        let p = [fx + off(d[0], r), fy + off(d[1], r), fz + off(d[2], r)];
                        add(engine, T::DrippingObsidianTear, p, [0.0; 3]);
                    }
                }
            }
        }
        Animate::PotentSulfur => {
            // PotentSulfurBlock.animateTick: under a water source, two
            // bubbles and, one time in ten, the noxious gas.
            let above = ctx.facts.get(ctx.state([pos[0], pos[1] + 1, pos[2]]));
            if above.fluid == Fluid::Water && above.fluid_level == 8 && !above.fluid_falling {
                for _ in 0..2 {
                    let r = engine.random();
                    let p = [
                        fx + f64::from(r.next_float()),
                        fy + 1.0 + f64::from(r.next_float()),
                        fz + f64::from(r.next_float()),
                    ];
                    add(engine, T::SulfurBubbles, p, [0.0; 3]);
                }
                if next_int(engine.random(), 10) == 0 {
                    let name = "minecraft:block.potent_sulfur.noxious_gas";
                    sound(engine, name, [fx, fy, fz], 1.0, 1.0);
                }
            }
        }
        Animate::Mycelium => {
            let r = engine.random();
            if next_int(r, 10) == 0 {
                let p = [fx + r.next_double(), fy + 1.1, fz + r.next_double()];
                add(engine, T::Mycelium, p, [0.0; 3]);
            }
        }
        Animate::Candles(count) => {
            for o in candle_offsets(count) {
                candle(
                    engine,
                    [fx + o[0] / 16.0, fy + o[1] / 16.0, fz + o[2] / 16.0],
                );
            }
        }
        Animate::CandleCake => candle(engine, [fx + 0.5, fy + 1.0, fz + 0.5]),
        Animate::BrewingStand => {
            let r = engine.random();
            let p = [
                fx + 0.4 + f64::from(r.next_float() * 0.2),
                fy + 0.7 + f64::from(r.next_float() * 0.3),
                fz + 0.4 + f64::from(r.next_float() * 0.2),
            ];
            add(engine, T::Smoke, p, [0.0; 3]);
        }
        Animate::DriedGhast { waterlogged } => {
            // DriedGhastBlock.animateTick.
            let c = [fx + 0.5, fy + 0.5, fz + 0.5];
            if waterlogged {
                if next_int(engine.random(), 40) == 0 {
                    let name = "minecraft:block.dried_ghast.ambient_water";
                    sound(engine, name, c, 1.0, 1.0);
                }
                let r = engine.random();
                if next_int(r, 6) == 0 {
                    let x = c[0] + f64::from((r.next_float() * 2.0 - 1.0) / 3.0);
                    let y = c[1] + 0.4;
                    let z = c[2] + f64::from((r.next_float() * 2.0 - 1.0) / 3.0);
                    let v = [0.0, f64::from(r.next_float()), 0.0];
                    add(engine, T::HappyVillager, [x, y, z], v);
                }
            } else {
                let below = ctx.state([pos[0], pos[1] - 1, pos[2]]);
                let triggers = || {
                    tables
                        .dried_ghast_sounds
                        .get(below.0 as usize)
                        .copied()
                        .unwrap_or(false)
                };
                if next_int(engine.random(), 40) == 0 && triggers() {
                    sound(engine, "minecraft:block.dried_ghast.ambient", c, 1.0, 1.0);
                }
                if next_int(engine.random(), 6) == 0 {
                    add(engine, T::WhiteSmoke, c, [0.0, 0.02, 0.0]);
                }
            }
        }
        Animate::EndPortal => {
            // EndPortalBlock.animateTick: one smoke particle above the surface.
            let r = engine.random();
            let p = [fx + r.next_double(), fy + 0.8, fz + r.next_double()];
            add(engine, T::Smoke, p, [0.0; 3]);
        }
        Animate::EnderChest => {
            for _ in 0..3 {
                let r = engine.random();
                let flip_x = f64::from(next_int(r, 2) * 2 - 1);
                let flip_z = f64::from(next_int(r, 2) * 2 - 1);
                let p = [
                    fx + 0.5 + 0.25 * flip_x,
                    fy + f64::from(r.next_float()),
                    fz + 0.5 + 0.25 * flip_z,
                ];
                let v = [
                    f64::from(r.next_float()) * flip_x,
                    f64::from(r.next_float() - 0.5) * 0.125,
                    f64::from(r.next_float()) * flip_z,
                ];
                add(engine, T::Portal, p, v);
            }
        }
        Animate::Falling(color) => {
            if next_int(engine.random(), 16) == 0 {
                let below = ctx.facts.get(ctx.state([pos[0], pos[1] - 1, pos[2]]));
                // FallingBlock.isFree: air, fire, liquids or replaceable.
                if below.replaceable
                    || below.fluid != Fluid::None
                    || ctx.is_air([pos[0], pos[1] - 1, pos[2]])
                {
                    let r = engine.random();
                    let p = [fx + r.next_double(), fy - 0.05, fz + r.next_double()];
                    engine.add(
                        &ParticleOptions::block(T::FallingDust, String::new(), color),
                        p,
                        [0.0; 3],
                    );
                }
            }
        }
        Animate::RedstoneOre => {
            for d in [
                [0, -1, 0],
                [0, 1, 0],
                [0, 0, -1],
                [0, 0, 1],
                [-1, 0, 0],
                [1, 0, 0],
            ] {
                let relative = [pos[0] + d[0], pos[1] + d[1], pos[2] + d[2]];
                if ctx.facts.get(ctx.state(relative)).solid_render {
                    continue;
                }
                let r = engine.random();
                let mut c = |s: i32| {
                    if s != 0 {
                        0.5 + 0.5625 * f64::from(s)
                    } else {
                        f64::from(r.next_float())
                    }
                };
                let p = [fx + c(d[0]), fy + c(d[1]), fz + c(d[2])];
                redstone(engine, p);
            }
        }
        Animate::RedstoneWire { power, sides } => {
            let color = redstone_color(power);
            let horizontals = [[0, 0, -1], [1, 0, 0], [0, 0, 1], [-1, 0, 0]];
            let line = |engine: &mut ParticleEngine,
                        side: [i32; 3],
                        along: [i32; 3],
                        from: f32,
                        to: f32| {
                let span = to - from;
                let r = engine.random();
                if r.next_float() < 0.2 * span {
                    let on = from + span * r.next_float();
                    let c = |i: usize| {
                        0.5 + 0.4375 * f64::from(side[i]) + f64::from(on) * f64::from(along[i])
                    };
                    engine.add(
                        &ParticleOptions::dust(color, 1.0),
                        [fx + c(0), fy + c(1), fz + c(2)],
                        [0.0; 3],
                    );
                }
            };
            for (i, h) in horizontals.into_iter().enumerate() {
                match sides[i] {
                    2 => {
                        line(engine, h, [0, 1, 0], -0.5, 0.5);
                        line(engine, [0, -1, 0], h, 0.0, 0.5);
                    }
                    1 => line(engine, [0, -1, 0], h, 0.0, 0.5),
                    _ => line(engine, [0, -1, 0], h, 0.0, 0.3),
                }
            }
        }
        Animate::SculkSensor => {
            // SculkSensorBlock.animateTick.
            let r = engine.random();
            let direction = r.next_int_bound(6).unwrap_or(0);
            // Direction.values(): down, up, north, south, west, east.
            if direction >= 2 {
                let step = [[0, 0, -1], [0, 0, 1], [-1, 0, 0], [1, 0, 0]][direction as usize - 2];
                let x = fx
                    + 0.5
                    + if step[0] == 0 {
                        0.5 - r.next_double()
                    } else {
                        f64::from(step[0]) * 0.6
                    };
                let z = fz
                    + 0.5
                    + if step[2] == 0 {
                        0.5 - r.next_double()
                    } else {
                        f64::from(step[2]) * 0.6
                    };
                let ya = f64::from(r.next_float() * 0.04);
                engine.add(
                    &crate::particles::ParticleOptions::sculk_to_redstone(),
                    [x, fy + 0.25, z],
                    [0.0, ya, 0.0],
                );
            }
        }
        Animate::RespawnAnchor => {
            let r = engine.random();
            if next_int(r, 100) == 0 {
                sound(
                    engine,
                    "minecraft:block.respawn_anchor.ambient",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    1.0,
                    1.0,
                );
            }
            let r = engine.random();
            let p = [
                fx + 0.5 + (0.5 - r.next_double()),
                fy + 1.0,
                fz + 0.5 + (0.5 - r.next_double()),
            ];
            let ya = f64::from(r.next_float() * 0.04);
            add(engine, T::ReversePortal, p, [0.0, ya, 0.0]);
        }
        Animate::BubbleColumn { drag } => {
            if drag {
                add(engine, T::CurrentDown, [fx + 0.5, fy + 0.8, fz], [0.0; 3]);
                let r = engine.random();
                if next_int(r, 200) == 0 {
                    let (volume, pitch) = (0.2 + r.next_float() * 0.2, 0.9 + r.next_float() * 0.15);
                    sound(
                        engine,
                        "minecraft:block.bubble_column.whirlpool_ambient",
                        [fx, fy, fz],
                        volume,
                        pitch,
                    );
                }
            } else {
                add(
                    engine,
                    T::BubbleColumnUp,
                    [fx + 0.5, fy, fz + 0.5],
                    [0.0, 0.04, 0.0],
                );
                let r = engine.random();
                let p = [
                    fx + f64::from(r.next_float()),
                    fy + f64::from(r.next_float()),
                    fz + f64::from(r.next_float()),
                ];
                add(engine, T::BubbleColumnUp, p, [0.0, 0.04, 0.0]);
                let r = engine.random();
                if next_int(r, 200) == 0 {
                    let (volume, pitch) = (0.2 + r.next_float() * 0.2, 0.9 + r.next_float() * 0.15);
                    sound(
                        engine,
                        "minecraft:block.bubble_column.upwards_ambient",
                        [fx, fy, fz],
                        volume,
                        pitch,
                    );
                }
            }
        }
        Animate::Fire => {
            let r = engine.random();
            if next_int(r, 24) == 0 {
                let (volume, pitch) = (1.0 + r.next_float(), r.next_float() * 0.7 + 0.3);
                sound(
                    engine,
                    "minecraft:block.fire.ambient",
                    [fx + 0.5, fy + 0.5, fz + 0.5],
                    volume,
                    pitch,
                );
            }
        }
    }
}
/// `AbstractCandleBlock.addParticlesAndSound`.
fn candle(engine: &mut ParticleEngine, p: [f64; 3]) {
    let r = engine.random();
    let chance = r.next_float();
    if chance < 0.3 {
        add(engine, T::Smoke, p, [0.0; 3]);
        if chance < 0.17 {
            let r = engine.random();
            let (volume, pitch) = (1.0 + r.next_float(), r.next_float() * 0.7 + 0.3);
            sound(
                engine,
                "minecraft:block.candle.ambient",
                [p[0] + 0.5, p[1] + 0.5, p[2] + 0.5],
                volume,
                pitch,
            );
        }
    }
    add(engine, T::SmallFlame, p, [0.0; 3]);
}
/// `CampfireBlockEntity.particleTick` for every lit campfire (no cooking items).
pub fn campfire_particles<'a>(
    tables: &AnimateTables,
    engine: &mut ParticleEngine,
    ctx: &AnimateContext,
    blocks: impl Iterator<Item = &'a ([i32; 3], u32)>,
) {
    for &(pos, state) in blocks {
        if !tables.is_lit_campfire(state) {
            continue;
        }
        let signal = ctx
            .registry
            .state(BlockStateId(state))
            .is_some_and(|s| s.properties.get("signal_fire").map(String::as_str) == Some("true"));
        let r = engine.random();
        if r.next_float() < 0.11 {
            let count = next_int(r, 2) + 2;
            for _ in 0..count {
                make_campfire_smoke(engine, pos, signal);
            }
        }
    }
}
/// `CampfireBlock.makeParticles` (always visible, overriding the limiter).
pub fn make_campfire_smoke(engine: &mut ParticleEngine, pos: [i32; 3], signal: bool) {
    let r = engine.random();
    let sign = |r: &mut LegacyRandom| if r.next_boolean() { 1.0 } else { -1.0 };
    let dx = r.next_double() / 3.0;
    let x = f64::from(pos[0]) + 0.5 + dx * sign(r);
    let y = f64::from(pos[1]) + r.next_double() + r.next_double();
    let dz = r.next_double() / 3.0;
    let z = f64::from(pos[2]) + 0.5 + dz * sign(r);
    let ty = if signal {
        T::CampfireSignalSmoke
    } else {
        T::CampfireCosySmoke
    };
    engine.create(&ParticleOptions::simple(ty), [x, y, z], [0.0, 0.07, 0.0]);
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn classifies_animated_blocks() {
        let none = |_: &str| None;
        assert_eq!(classify("minecraft:torch", &none), Animate::Torch(T::Flame));
        let lit = |k: &str| (k == "lit").then(|| "true".to_owned());
        assert_eq!(
            classify("minecraft:furnace", &lit),
            Animate::Furnace([0, -1, 0])
        );
        assert_eq!(classify("minecraft:furnace", &none), Animate::None);
        let wall = |k: &str| (k == "facing").then(|| "north".to_owned());
        assert_eq!(
            classify("minecraft:soul_wall_torch", &wall),
            Animate::WallTorch(T::SoulFireFlame, [0, 0, -1])
        );
        let candles = |k: &str| match k {
            "lit" => Some("true".to_owned()),
            "candles" => Some("3".to_owned()),
            _ => None,
        };
        assert_eq!(
            classify("minecraft:red_candle", &candles),
            Animate::Candles(3)
        );
        assert_eq!(
            classify("minecraft:red_candle_cake", &lit),
            Animate::CandleCake
        );
    }
    #[test]
    fn redstone_colours_match_wire_table() {
        // RedstoneWireBlock.COLORS[15] = (1.0, 0.2, 0.1) before int packing.
        let c = redstone_color(15);
        assert!((c[0] - 1.0).abs() < 1e-6);
        assert!((c[1] - (51.0 / 255.0)).abs() < 0.01);
    }
}
