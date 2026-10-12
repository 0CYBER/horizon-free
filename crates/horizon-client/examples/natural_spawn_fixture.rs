//! Natural spawning decisions for differential tests against vanilla
//! (tools/test_natural_spawn_parity.py): reads the box-world cases of
//! `VanillaNaturalSpawnOracle` and prints what `mobs::spawn_selection` does in
//! the same order and format.
//!
//! The oracle runs `finalizeSpawn` with a fixed random (nextInt 0, nextFloat
//! 0.75); `neutral_box` is the box that leaves a mob with: polar bears,
//! ocelots and rabbits after the first of their group are babies (chance
//! 1.0), foxes and axolotls from the third, sulfur cubes have size 2, salmon
//! are small, slimes and magma cubes keep size 1, tropical fish school.
//!
//! usage: natural_spawn_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    assets::Assets,
    biome::{Biomes, SpawnEntry},
    blocks::BlockFacts,
    collisions::{self, LookupCollisions, no_collision_empty},
    items::Items,
    mobs::{
        MobTypes, Species,
        data::MobData,
        spawn_rules::{Dimension, SpawnLevel},
        spawn_selection::{
            self, Category, Counted, Event, GenEvent, Heightmap, NaturalLevel, SpawnCost,
            SpawnPlayer, SpawnRequest, SpawnState, Spawned, StructureAt, StructureSpawns,
            spawn_box,
        },
        species::{self, Baby},
    },
};
use horizon_core::{
    geometry::Aabb,
    random::{LegacyRandom, RandomSource},
};
use horizon_world::{
    block_collision::BlockCollisionRegistry, chunk::BlockState, registry::BlockStateId,
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap, HashSet},
    fmt::Write as _,
    fs,
    path::Path,
};

#[derive(Clone, Copy)]
struct Bx([i32; 6]);
impl Bx {
    fn parse(s: &str) -> Self {
        let v: Vec<i32> = s.split(',').map(|n| n.parse().unwrap_or(0)).collect();
        Self([v[0], v[1], v[2], v[3], v[4], v[5]])
    }
    fn has(&self, x: i32, y: i32, z: i32) -> bool {
        let b = self.0;
        x >= b[0] && x <= b[3] && y >= b[1] && y <= b[4] && z >= b[2] && z <= b[5]
    }
}
struct Start {
    id: String,
    pieces: Vec<Bx>,
    /// The start's bounding box (pieces encapsulated, inflated by 12 for
    /// terrain adaptation).
    bounds: Bx,
}

struct Shared<'a> {
    facts: &'a BlockFacts,
    data: &'a MobData,
    block_tags: &'a HashMap<String, HashSet<String>>,
    types: &'a MobTypes,
    biomes: &'a Biomes,
    structures: &'a StructureSpawns,
    collision: &'a BlockCollisionRegistry<'a>,
    air: BlockStateId,
}
struct World<'a> {
    s: &'a Shared<'a>,
    blocks: Vec<(Bx, BlockStateId)>,
    lights: Vec<(Bx, i32, i32)>,
    biomes: Vec<(Bx, String)>,
    biome: String,
    sky: i32,
    light: i32,
    darken: i32,
    thunder: bool,
    sea: i32,
    seed: i64,
    peaceful: bool,
    dimension: Dimension,
    dimension_id: String,
    slime: f32,
    starts: Vec<Start>,
    players: Vec<([f64; 3], String)>,
    respawn: Option<[i32; 3]>,
    unloaded: HashSet<(i32, i32)>,
    spawn_mobs: bool,
    /// Boxes of living entities (case entities, then spawned mobs).
    entities: RefCell<Vec<Aabb>>,
}
impl World<'_> {
    fn biome_at(&self, x: i32, y: i32, z: i32) -> &str {
        self.biomes
            .iter()
            .rev()
            .find(|(b, _)| b.has(x, y, z))
            .map_or(&self.biome, |(_, n)| n)
    }
    fn biome_info(&self, x: i32, y: i32, z: i32) -> Option<&horizon_client::biome::BiomeInfo> {
        let i = *self.s.biomes.index.get(self.biome_at(x, y, z))?;
        Some(self.s.biomes.get(i))
    }
    fn lights_at(&self, x: i32, y: i32, z: i32) -> (i32, i32) {
        self.lights
            .iter()
            .rev()
            .find(|(b, ..)| b.has(x, y, z))
            .map_or((self.sky, self.light), |(_, s, b)| (*s, *b))
    }
}
impl SpawnLevel for World<'_> {
    fn state(&self, x: i32, y: i32, z: i32) -> BlockStateId {
        self.blocks
            .iter()
            .rev()
            .find(|(b, _)| b.has(x, y, z))
            .map_or(self.s.air, |(_, s)| *s)
    }
    fn facts(&self) -> &BlockFacts {
        self.s.facts
    }
    fn mob_data(&self) -> &MobData {
        self.s.data
    }
    fn block_tag(&self, tag: &str, block: &str) -> bool {
        self.s
            .block_tags
            .get(tag)
            .is_some_and(|s| s.contains(block))
    }
    fn biome_tag(&self, tag: &str, x: i32, y: i32, z: i32) -> bool {
        self.s
            .data
            .biome_tags
            .get(tag)
            .is_some_and(|s| s.contains(self.biome_at(x, y, z)))
    }
    fn sky_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.lights_at(x, y, z).0
    }
    fn block_light(&self, x: i32, y: i32, z: i32) -> i32 {
        self.lights_at(x, y, z).1
    }
    fn sky_darken(&self) -> i32 {
        self.darken
    }
    fn thundering(&self) -> bool {
        self.thunder
    }
    fn sea_level(&self) -> i32 {
        self.sea
    }
    fn world_surface(&self, x: i32, z: i32) -> i32 {
        self.heightmap(Heightmap::WorldSurface, x, z)
    }
    fn peaceful(&self) -> bool {
        self.peaceful
    }
    fn dimension(&self) -> Dimension {
        self.dimension
    }
    fn surface_slime_chance(&self, _: i32, _: i32, _: i32) -> f32 {
        self.slime
    }
    fn world_seed(&self) -> i64 {
        self.seed
    }
    fn player_near(&self, x: f64, y: f64, z: f64, distance: f64) -> bool {
        self.players.iter().any(|(p, mode)| {
            let d = (p[0] - x).powi(2) + (p[1] - y).powi(2) + (p[2] - z).powi(2);
            (mode == "survival" || mode == "adventure") && d < distance * distance
        })
    }
}
impl NaturalLevel for World<'_> {
    fn min_y(&self) -> i32 {
        if self.dimension == Dimension::Overworld {
            -64
        } else {
            0
        }
    }
    fn max_y(&self) -> i32 {
        if self.dimension == Dimension::Overworld {
            319
        } else {
            255
        }
    }
    fn ambient_light(&self) -> f32 {
        match self.dimension {
            Dimension::Overworld => 0.0,
            Dimension::Nether => 0.1,
            Dimension::End => 0.25,
        }
    }
    fn has_ceiling(&self) -> bool {
        self.dimension == Dimension::Nether
    }
    fn mob_type(&self, entity: &str) -> Option<(Species, f32, f32, Category)> {
        let t = self.s.types.types.get(entity)?;
        Some((
            t.species,
            t.width as f32,
            t.height as f32,
            Category::from_name(&t.category),
        ))
    }
    fn biome_spawns(&self, x: i32, y: i32, z: i32) -> Option<&HashMap<String, Vec<SpawnEntry>>> {
        self.biome_info(x, y, z).map(|b| &b.spawns)
    }
    fn spawn_cost(&self, entity: &str, x: i32, y: i32, z: i32) -> Option<SpawnCost> {
        self.biome_info(x, y, z)?.spawn_costs.get(entity).copied()
    }
    fn creature_probability(&self, x: i32, y: i32, z: i32) -> f32 {
        self.biome_info(x, y, z)
            .map_or(0.1, |b| b.creature_probability)
    }
    fn structures_at(&self, x: i32, y: i32, z: i32) -> Vec<StructureAt> {
        let (cx, cz) = (x >> 4, z >> 4);
        let mut out: Vec<StructureAt> = Vec::new();
        for start in &self.starts {
            let b = start.bounds.0;
            // Referenced: the start's box reaches the chunk's columns.
            if b[3] < cx * 16 || b[0] > cx * 16 + 15 || b[5] < cz * 16 || b[2] > cz * 16 + 15 {
                continue;
            }
            let in_piece = start.pieces.iter().any(|p| p.has(x, y, z));
            let in_bounds = start.bounds.has(x, y, z);
            match out.iter_mut().find(|s| s.id == start.id) {
                Some(s) => {
                    s.in_piece |= in_piece;
                    s.in_bounds |= in_bounds;
                }
                None => out.push(StructureAt {
                    id: start.id.clone(),
                    in_piece,
                    in_bounds,
                }),
            }
        }
        out
    }
    fn structure_spawns(&self) -> &StructureSpawns {
        self.s.structures
    }
    fn nearest_player_distance_sqr(&self, x: f64, y: f64, z: f64) -> Option<f64> {
        let mut best: Option<f64> = None;
        for (p, mode) in &self.players {
            if mode == "spectator" {
                continue;
            }
            let d = [p[0] - x, p[1] - y, p[2] - z];
            let d = d[0] * d[0] + d[1] * d[1] + d[2] * d[2];
            if best.is_none_or(|b| d < b) {
                best = Some(d);
            }
        }
        best
    }
    fn respawn(&self) -> Option<[i32; 3]> {
        self.respawn
    }
    fn can_spawn_in_chunk(&self, cx: i32, cz: i32) -> bool {
        !self.unloaded.contains(&(cx, cz))
    }
    fn no_collision(&self, bounds: Aabb) -> bool {
        let lookup = |x: i32, y: i32, z: i32| self.state(x, y, z);
        let mut world = LookupCollisions {
            block: &lookup,
            binding: self.s.collision,
        };
        no_collision_empty(&mut world, bounds)
    }
    fn unobstructed(&self, bounds: Aabb) -> bool {
        !self.entities.borrow().iter().any(|b| b.intersects(bounds))
    }
    fn spawn_mobs(&self) -> bool {
        self.spawn_mobs
    }
    fn heightmap(&self, kind: Heightmap, x: i32, z: i32) -> i32 {
        (self.min_y()..=self.max_y())
            .rev()
            .find(|&y| spawn_selection::heightmap_blocks(self, kind, x, y, z))
            .map_or(self.min_y(), |y| y + 1)
    }
    fn pathfindable_land(&self, x: i32, y: i32, z: i32) -> bool {
        self.s.data.state(self.state(x, y, z)).flags & horizon_client::mobs::data::LAND != 0
    }
}

/// The box a mob is left with by `finalizeSpawn` under the oracle's fixed random.
fn neutral_box(l: &World, r: &SpawnRequest) -> Aabb {
    let t = &l.s.types.types[r.entity];
    let (mut w, mut h) = (t.width as f32, t.height as f32);
    let baby = match r.species {
        Species::PolarBear | Species::Ocelot | Species::Rabbit => r.group_index > 0,
        Species::Fox | Species::Axolotl => r.group_index >= 2,
        _ => false,
    };
    if baby {
        match species::def(r.species).baby {
            Baby::Abs(bw, bh, _) => (w, h) = (bw as f32, bh as f32),
            Baby::Scale(s, _) => (w, h) = (w * s as f32, h * s as f32),
            Baby::None => {}
        }
    }
    match r.species {
        Species::SulfurCube => (w, h) = (w * 2.0, h * 2.0),
        Species::Salmon => (w, h) = (w * 0.5, h * 0.5),
        _ => {}
    }
    let [x, y, z] = r.position;
    spawn_box(x, y, z, w, h)
}

fn parse_state(spec: &str) -> BlockState {
    let (name, props) = match spec.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (spec, ""),
    };
    BlockState {
        name: name.to_owned(),
        properties: props
            .split(',')
            .filter(|p| !p.is_empty())
            .filter_map(|p| p.split_once('='))
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
    }
}
fn bits(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
/// The yaw as the oracle prints it: a cube mob grown by `finalizeSpawn`
/// (sulfur cubes, size 2 under the fixed random) takes its constructor's
/// head rotation from the entity's unseeded random.
fn yaw_text(entity: &str, v: f32) -> String {
    if entity == "minecraft:sulfur_cube" {
        "head".to_owned()
    } else {
        format!("{:x}", v.to_bits())
    }
}

fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    let root = Path::new(
        args.get(1)
            .ok_or("usage: natural_spawn_fixture RESOURCES INPUT OUTPUT")?,
    );
    let input = fs::read_to_string(args.get(2).ok_or("missing INPUT")?)?;
    let assets = Assets::load(root)?;
    let registry = assets.registry;
    let facts = BlockFacts::load(root, registry)?;
    let data = MobData::load(root, &facts);
    let items = Items::tags_only(root)?;
    let types = MobTypes::load(root)?;
    let biomes = Biomes::load(root)?;
    let structures = StructureSpawns::load(root);
    let collision = collisions::load(root, registry)?;
    let shared = Shared {
        facts: &facts,
        data: &data,
        block_tags: &items.block_tags,
        types: &types,
        biomes: &biomes,
        structures: &structures,
        collision: &collision,
        air: facts.air,
    };
    let mut adaptation: HashMap<String, bool> = HashMap::new();
    let mut out = String::new();
    for raw in input.lines() {
        if raw.trim().is_empty() {
            continue;
        }
        let mut c = BTreeMap::new();
        let mut name = "";
        for token in raw.split_whitespace() {
            match token.split_once('=') {
                Some((k, v)) => {
                    c.insert(k, v);
                }
                None => name = token,
            }
        }
        let get =
            |k: &str, d: &'static str| -> String { c.get(k).copied().unwrap_or(d).to_owned() };
        let indexed = |prefix: &str| -> Vec<&str> {
            (0..)
                .map_while(|i| c.get(format!("{prefix}{i}").as_str()).copied())
                .collect()
        };
        let mut blocks = Vec::new();
        for k in indexed("k") {
            let (b, s) = k.split_once('/').ok_or("bad block box")?;
            blocks.push((Bx::parse(b), registry.resolve(&parse_state(s))?));
        }
        let lights = indexed("l")
            .into_iter()
            .filter_map(|l| {
                let (b, v) = l.split_once('/')?;
                let (s, k) = v.split_once(',')?;
                Some((Bx::parse(b), s.parse().ok()?, k.parse().ok()?))
            })
            .collect();
        let biome_boxes = indexed("b")
            .into_iter()
            .filter_map(|l| {
                let (b, n) = l.split_once('/')?;
                Some((Bx::parse(b), n.to_owned()))
            })
            .collect();
        let mut starts = Vec::new();
        for s in indexed("s") {
            let (id, pieces) = s.split_once('/').ok_or("bad structure")?;
            let pieces: Vec<Bx> = pieces.split(';').map(Bx::parse).collect();
            let mut b = pieces[0].0;
            for p in &pieces[1..] {
                for i in 0..3 {
                    b[i] = b[i].min(p.0[i]);
                    b[i + 3] = b[i + 3].max(p.0[i + 3]);
                }
            }
            let adapts = *adaptation.entry(id.to_owned()).or_insert_with(|| {
                let path = root.join(format!(
                    "data/minecraft/worldgen/structure/{}.json",
                    id.trim_start_matches("minecraft:")
                ));
                fs::read(path)
                    .ok()
                    .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                    .and_then(|v| v["terrain_adaptation"].as_str().map(|a| a != "none"))
                    .unwrap_or(false)
            });
            if adapts {
                for i in 0..3 {
                    b[i] -= 12;
                    b[i + 3] += 12;
                }
            }
            starts.push(Start {
                id: id.to_owned(),
                pieces,
                bounds: Bx(b),
            });
        }
        let players = indexed("p")
            .into_iter()
            .map(|p| {
                let mut parts = p.split('/');
                let v: Vec<f64> = parts
                    .next()
                    .unwrap_or("")
                    .split(',')
                    .map(|n| n.parse().unwrap_or(0.0))
                    .collect();
                (
                    [v[0], v[1], v[2]],
                    parts.next().unwrap_or("survival").to_owned(),
                )
            })
            .collect::<Vec<_>>();
        let dimension_id = get("dim", "minecraft:overworld");
        let dimension = match dimension_id.as_str() {
            "minecraft:the_nether" => Dimension::Nether,
            "minecraft:the_end" => Dimension::End,
            _ => Dimension::Overworld,
        };
        let respawn = {
            let r = get("respawn", "0,-1000,0/minecraft:overworld");
            let (p, d) = r.split_once('/').ok_or("bad respawn")?;
            let v: Vec<i32> = p.split(',').map(|n| n.parse().unwrap_or(0)).collect();
            (d == dimension_id).then_some([v[0], v[1], v[2]])
        };
        let unloaded = c
            .get("unloaded")
            .map(|u| {
                u.split(';')
                    .filter_map(|p| {
                        let (x, z) = p.split_once(',')?;
                        Some((x.parse().ok()?, z.parse().ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default();
        // Case entities: (type, position, persistent).
        let mut case_entities = Vec::new();
        for e in indexed("e") {
            let parts: Vec<&str> = e.split('/').collect();
            let v: Vec<f64> = parts[1]
                .split(',')
                .map(|n| n.parse().unwrap_or(0.0))
                .collect();
            // EntityType.create makes no monsters in peaceful.
            if get("difficulty", "normal") == "peaceful"
                && types.types.get(parts[0]).is_some_and(|t| {
                    !horizon_client::mobs::spawn_rules::allowed_in_peaceful(t.species)
                })
            {
                continue;
            }
            case_entities.push((
                parts[0].to_owned(),
                [v[0], v[1], v[2]],
                parts.get(2) == Some(&"p"),
            ));
        }
        let world = World {
            s: &shared,
            blocks,
            lights,
            biomes: biome_boxes,
            biome: get("biome", "minecraft:plains"),
            sky: get("sky", "15").parse()?,
            light: get("light", "0").parse()?,
            darken: get("darken", "0").parse()?,
            thunder: get("thunder", "0") == "1",
            sea: get("sea", "63").parse()?,
            seed: get("wseed", "0").parse()?,
            peaceful: get("difficulty", "normal") == "peaceful",
            dimension,
            dimension_id: dimension_id.clone(),
            slime: get("slime", "0.0").parse()?,
            starts,
            players,
            respawn,
            unloaded,
            spawn_mobs: get("rules", "1") != "0",
            entities: RefCell::new(
                case_entities
                    .iter()
                    .filter_map(|(t, p, _)| {
                        let t = types.types.get(t)?;
                        Some(spawn_box(p[0], p[1], p[2], t.width as f32, t.height as f32))
                    })
                    .collect(),
            ),
        };
        let _ = &world.dimension_id;
        let chunk = {
            let v: Vec<i32> = get("chunk", "0,0")
                .split(',')
                .map(|n| n.parse().unwrap_or(0))
                .collect();
            (v[0], v[1])
        };
        let mut random = LegacyRandom::new(get("seed", "0").parse()?);
        let mut log: Vec<String> = Vec::new();
        let mut spawn = |l: &World, r: &SpawnRequest| -> Option<Spawned> {
            let bounds = neutral_box(l, r);
            l.entities.borrow_mut().push(bounds);
            Some(Spawned {
                bounds,
                school: true,
            })
        };
        let mode = get("mode", "chunk");
        if mode == "gen" {
            let mut gen_random = LegacyRandom::new(get("gseed", "0").parse()?);
            // The level random of the spawn predicates: no creature's predicate
            // draws from it (the oracle's answers fixed values).
            let mut rules = LegacyRandom::new(0);
            let mut spawns = Vec::new();
            let source = [chunk.0 * 16, world.max_y(), chunk.1 * 16];
            spawn_selection::spawn_mobs_for_chunk_generation(
                &world,
                source,
                chunk,
                &mut gen_random,
                &mut rules,
                &mut spawn,
                &mut |e| match e {
                    GenEvent::Column(x, z) => log.push(format!("col {x} {z}")),
                    GenEvent::Spawn(t, p, yaw) => spawns.push(format!(
                        "spawn {t} {} {} {} {}",
                        bits(p[0]),
                        bits(p[1]),
                        bits(p[2]),
                        yaw_text(&t, yaw)
                    )),
                },
            );
            log.extend(spawns);
            log.push(format!(
                "next {}",
                gen_random.next_int_bound(1000).unwrap_or(0)
            ));
        } else {
            let counted: Vec<Counted> = case_entities
                .iter()
                .filter_map(|(t, p, persistent)| {
                    let kind = types.types.get(t)?;
                    Some(Counted {
                        entity: t,
                        category: Category::from_name(&kind.category),
                        block: [
                            p[0].floor() as i32,
                            p[1].floor() as i32,
                            p[2].floor() as i32,
                        ],
                        is_mob: true,
                        persistent: *persistent,
                    })
                })
                .collect();
            let players = world
                .players
                .iter()
                .map(|(p, mode)| SpawnPlayer {
                    position: *p,
                    spectator: mode == "spectator",
                })
                .collect();
            let mut state = SpawnState::new(
                &world,
                get("chunkcount", "289").parse()?,
                players,
                &counted,
                |cx, cz| !world.unloaded.contains(&(cx, cz)),
            );
            let mut trace = |e: Event| {
                log.push(match e {
                    Event::Categories(list) => format!(
                        "cats {}",
                        list.iter().map(|c| c.name()).collect::<Vec<_>>().join(",")
                    ),
                    Event::Local(c, ok) => format!("local {} {ok}", c.name()),
                    Event::Start(c, p) => format!("start {} {} {} {}", c.name(), p[0], p[1], p[2]),
                    Event::Try(x, z) => format!("try {x} {z}"),
                    Event::Test(t, p, ok) => format!("test {t} {} {} {} {ok}", p[0], p[1], p[2]),
                    Event::Spawn(t, p, yaw) => format!(
                        "spawn {t} {} {} {} {}",
                        bits(p[0]),
                        bits(p[1]),
                        bits(p[2]),
                        yaw_text(&t, yaw)
                    ),
                })
            };
            if mode == "pos" {
                let category = Category::from_name(&get("cat", "monster"));
                let v: Vec<i32> = get("start", "0,0,0")
                    .split(',')
                    .map(|n| n.parse().unwrap_or(0))
                    .collect();
                spawn_selection::spawn_category_for_position(
                    &world,
                    &mut random,
                    &mut state,
                    category,
                    chunk,
                    [v[0], v[1], v[2]],
                    &mut spawn,
                    &mut trace,
                );
            } else {
                let categories = state
                    .filtered_categories(get("enemies", "1") == "1", get("persistent", "1") == "1");
                trace(Event::Categories(categories.clone()));
                spawn_selection::spawn_for_chunk(
                    &world,
                    &mut random,
                    &mut state,
                    &categories,
                    chunk,
                    &mut spawn,
                    &mut trace,
                );
            }
            log.push(format!("next {}", random.next_int_bound(1000).unwrap_or(0)));
        }
        let _ = writeln!(out, "{name} {}", log.join(" ; "));
    }
    fs::write(args.get(3).ok_or("missing OUTPUT")?, out)?;
    Ok(())
}
