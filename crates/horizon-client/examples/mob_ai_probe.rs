//! Replays `tools/test_mob_ai_parity.py` scenarios on Horizon's mob AI and
//! prints each mob's AI-visible state after every tick in the format of
//! `VanillaMobAiOracle`.
//!
//! usage: mob_ai_probe RESOURCES SCENARIOS OUTPUT
use horizon_client::{
    assets::Assets,
    biome::Biomes,
    blocks::BlockFacts,
    chunk::{Chunk, HEIGHT, MIN_Y},
    collisions,
    items::Items,
    light::{LightTables, light_chunk},
    mobs::{
        Mob, MobData, MobEvent, MobTypes, MobWorld, PlayerView, Seen, Species, Who,
        control::Operation, spawning::LocalDifficulty,
    },
    world::World,
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::LegacyRandom,
};
use horizon_world::{chunk::BlockState, registry::BlockRegistry, registry::BlockStateId};
use std::{collections::BTreeMap, fmt::Write as _, fs, path::Path, sync::Arc};

struct MobSpec {
    kind: String,
    pos: [f64; 3],
    yaw: f32,
    seed: i64,
    baby: bool,
    flags: Vec<String>,
}
#[derive(Default)]
struct Scenario {
    name: String,
    ticks: i32,
    time: i64,
    rain: bool,
    thunder: bool,
    origin: [i32; 2],
    blocks: Vec<Vec<String>>,
    player: Option<[f64; 3]>,
    creative: bool,
    mobs: Vec<MobSpec>,
}

fn parse(text: &str) -> Vec<Scenario> {
    let mut out = Vec::new();
    let mut cur: Option<Scenario> = None;
    for line in text.lines() {
        let f: Vec<&str> = line.split_whitespace().collect();
        if f.is_empty() || f[0].starts_with('#') {
            continue;
        }
        let s = cur.get_or_insert_with(Scenario::default);
        match f[0] {
            "scenario" => {
                s.name = f[1].to_owned();
                s.time = 6000;
                for kv in &f[2..] {
                    let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
                    match k {
                        "ticks" => s.ticks = v.parse().unwrap(),
                        "time" => s.time = v.parse().unwrap(),
                        "rain" => s.rain = true,
                        "thunder" => s.thunder = true,
                        "levelseed" => {}
                        _ => panic!("unknown scenario option {kv}"),
                    }
                }
            }
            "origin" => s.origin = [f[1].parse().unwrap(), f[2].parse().unwrap()],
            "fill" | "block" => s.blocks.push(f.iter().map(|x| (*x).to_owned()).collect()),
            "player" => {
                s.player = Some([
                    f[1].parse().unwrap(),
                    f[2].parse().unwrap(),
                    f[3].parse().unwrap(),
                ]);
                s.creative = f.get(4) == Some(&"creative");
            }
            "mob" => s.mobs.push(MobSpec {
                kind: f[1].to_owned(),
                pos: [
                    f[2].parse().unwrap(),
                    f[3].parse().unwrap(),
                    f[4].parse().unwrap(),
                ],
                yaw: f[5].parse().unwrap(),
                seed: f[6].parse().unwrap(),
                baby: f[7..].contains(&"baby"),
                flags: f[7..]
                    .iter()
                    .filter(|x| **x != "baby")
                    .map(|x| (*x).to_owned())
                    .collect(),
            }),
            "end" => out.push(cur.take().unwrap()),
            other => panic!("unknown scenario line {other}"),
        }
    }
    out
}

/// `minecraft:name[key=value,...]`
fn state(registry: &BlockRegistry, text: &str) -> BlockStateId {
    let (name, props) = match text.split_once('[') {
        Some((n, rest)) => (n, rest.trim_end_matches(']')),
        None => (text, ""),
    };
    let mut properties = BTreeMap::new();
    for kv in props.split(',').filter(|s| !s.is_empty()) {
        let (k, v) = kv.split_once('=').expect("property");
        properties.insert(k.to_owned(), v.to_owned());
    }
    registry
        .resolve(&BlockState {
            name: name.to_owned(),
            properties,
        })
        .unwrap_or_else(|e| panic!("{text}: {e}"))
}

fn d(v: f64) -> String {
    format!("{:x}", v.to_bits())
}
fn f(v: f32) -> String {
    format!("{:x}", v.to_bits())
}
fn who(w: Option<Who>) -> String {
    match w {
        None => "none".into(),
        Some(Who::Player) => "player".into(),
        Some(Who::Mob(id)) => format!("e{id}"),
    }
}

/// Vanilla's goal class for a Horizon goal (most share the name).
fn goal_name(m: &Mob, name: &str) -> String {
    let vanilla = match name {
        "Float" => "FloatGoal",
        "Stroll" => "WaterAvoidingRandomStrollGoal",
        "LookAtPlayer" => "LookAtPlayerGoal",
        "RandomLookAround" => "RandomLookAroundGoal",
        "HurtByTarget" => "HurtByTargetGoal",
        "NearestTarget" => "NearestAttackableTargetGoal",
        "Panic" => "PanicGoal",
        "Breed" => "BreedGoal",
        "Tempt" => "TemptGoal",
        "FollowParent" => "FollowParentGoal",
        "MeleeAttack" => match m.species() {
            Species::Zombie | Species::Husk | Species::ZombieVillager => "ZombieAttackGoal",
            _ => "MeleeAttackGoal",
        },
        "MoveThroughVillage" => "MoveThroughVillageGoal",
        "RestrictSun" => "RestrictSunGoal",
        "FleeSun" => "FleeSunGoal",
        "AvoidEntity" => "AvoidEntityGoal",
        "BowAttack" => "RangedBowAttackGoal",
        "Swell" => "SwellGoal",
        "LeapAtTarget" => "LeapAtTargetGoal",
        "EatBlock" => "EatBlockGoal",
        "MoveTowardsRestriction" => "MoveTowardsRestrictionGoal",
        "TradeWithPlayer" => "TradeWithPlayerGoal",
        "LookAtTradingPlayer" => "LookAtTradingPlayerGoal",
        "WanderToPosition" => "WanderToPositionGoal",
        "DrinkItem" => "UseItemGoal",
        "BreathAir" => "BreathAirGoal",
        "TryFindWater" => "TryFindWaterGoal",
        "RangedAttack" => "RangedAttackGoal",
        "MoveTowardsTarget" => "MoveTowardsTargetGoal",
        other => return other.to_owned(),
    };
    vanilla.to_owned()
}

fn goals(m: &Mob, names: Vec<&'static str>) -> String {
    if names.is_empty() {
        return "-".into();
    }
    names
        .iter()
        .map(|n| goal_name(m, n))
        .collect::<Vec<_>>()
        .join("|")
}

fn seen(m: &Mob) -> Seen {
    Seen {
        id: m.id,
        species: m.species(),
        position: m.position,
        bounds: m.bounds(),
        eye_y: m.eye_y(),
        alive: m.alive(),
        baby: m.is_baby(),
        in_love: if m.species() == Species::Villager {
            horizon_client::mobs::villager::can_breed(m)
        } else {
            m.extra.in_love > 0
        },
        tame: m.extra.tame,
        hostile: m.hostile(),
        target: m.ai.target,
        health: m.health,
        variant: m.extra.variant,
        color: m.extra.color,
        job: (m.species() == Species::Villager)
            .then_some(m.extra.home)
            .flatten(),
        bed: (m.species() == Species::Villager)
            .then_some(m.extra.bed)
            .flatten(),
        yaw: m.yaw,
        velocity: m.velocity,
        leash: m.extra.leash,
        caravan: horizon_client::mobs::caravan::head_of(m),
        delay: m.extra.ints[5],
        hurt_by: m.ai.last_hurt_by.map(|w| (w, m.ai.last_hurt_by_time)),
        active_raid: horizon_client::mobs::raider::has_active_raid(m),
        player_rep: 0,
    }
}

fn print(out: &mut String, t: i32, i: usize, m: &Mob) {
    let _ = write!(out, "{t} {i}");
    if m.removed {
        let _ = writeln!(out, " removed DISCARDED");
        return;
    }
    let p = m.position;
    let v = m.velocity;
    let _ = write!(
        out,
        "\tp {},{},{}\tv {},{},{}",
        d(p.x),
        d(p.y),
        d(p.z),
        d(v.x),
        d(v.y),
        d(v.z)
    );
    let _ = write!(
        out,
        "\tr {},{},{},{}",
        f(m.yaw),
        f(m.head_pitch),
        f(m.head_yaw),
        f(m.body_yaw)
    );
    let _ = write!(
        out,
        "\tg {}{}\thp {}",
        u8::from(m.on_ground),
        u8::from(m.horizontal_collision),
        f(m.health)
    );
    let _ = write!(
        out,
        "\tgoals {}\ttargets {}\ttarget {}",
        goals(m, m.ai.goals.running_names()),
        goals(m, m.ai.targets.running_names()),
        who(m.ai.target)
    );
    let _ = write!(out, "\tnav ");
    match &m.ai.nav.path {
        None => out.push_str("none"),
        Some(path) => {
            let _ = write!(
                out,
                "{},{},{} {}/{}{}",
                path.target[0],
                path.target[1],
                path.target[2],
                path.next,
                path.nodes.len(),
                if path.next >= path.nodes.len() {
                    " done"
                } else {
                    ""
                }
            );
        }
    }
    let mv = &m.ai.mov;
    let op = match mv.operation {
        Operation::Wait => "WAIT",
        Operation::MoveTo => "MOVE_TO",
        Operation::Strafe => "STRAFE",
        Operation::Jumping => "JUMPING",
    };
    let _ = write!(
        out,
        "\tmove {op} {},{},{} {}",
        d(mv.wanted[0]),
        d(mv.wanted[1]),
        d(mv.wanted[2]),
        d(mv.speed_modifier)
    );
    let lk = &m.ai.look;
    let _ = write!(
        out,
        "\tlook {} {},{},{}",
        lk.cooldown,
        d(lk.wanted[0]),
        d(lk.wanted[1]),
        d(lk.wanted[2])
    );
    let _ = write!(
        out,
        "\tin {},{},{},{}",
        f(m.ai.xxa),
        f(m.ai.zza),
        f(m.ai.speed),
        u8::from(m.ai.jumping)
    );
    let _ = writeln!(out, "\trng {:x}", m.random.state());
}

fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.len() != 3 {
        return Err("usage: mob_ai_probe RESOURCES SCENARIOS OUTPUT".into());
    }
    let root = Path::new(&args[0]);
    let assets = Assets::load(root)?;
    let registry = assets.registry;
    let facts = BlockFacts::load(root, registry)?;
    let data = MobData::load(root, &facts);
    let types = MobTypes::load(root)?;
    let biomes = Biomes::load(root)?;
    let items = Items::load(root, registry, &facts)?;
    let tables = LightTables::load(root, registry, &facts)?;
    let collision = collisions::load(root, registry)?;
    let air = registry.default_state("minecraft:air").ok_or("no air")?;
    let layers = [
        state(registry, "minecraft:bedrock"),
        state(registry, "minecraft:dirt"),
        state(registry, "minecraft:dirt"),
        state(registry, "minecraft:grass_block"),
    ];
    let plains = biomes.id("minecraft:plains");
    let mut out = String::new();
    for s in parse(&fs::read_to_string(&args[1])?) {
        let _ = writeln!(out, "== {}", s.name);
        let mut world = World::new(air, 0);
        let (ocx, ocz) = (s.origin[0] >> 4, s.origin[1] >> 4);
        const R: i32 = 4;
        for cz in ocz - R..=ocz + R {
            for cx in ocx - R..=ocx + R {
                let mut chunk = Chunk::empty(cx, cz);
                for qy in 0..HEIGHT / 4 {
                    for qz in 0..4 {
                        for qx in 0..4 {
                            chunk.set_biome_cell(qx, qy, qz, plains);
                        }
                    }
                }
                world.chunks.insert((cx, cz), Arc::new(chunk));
                for z in 0..16 {
                    for x in 0..16 {
                        for (y, layer) in layers.iter().enumerate() {
                            world.set(cx * 16 + x, MIN_Y + y as i32, cz * 16 + z, *layer)?;
                        }
                    }
                }
            }
        }
        for b in &s.blocks {
            let n = |i: usize| b[i].parse::<i32>().unwrap();
            if b[0] == "block" {
                world.set(n(1), n(2), n(3), state(registry, &b[4]))?;
            } else {
                let st = state(registry, &b[7]);
                for x in n(1).min(n(4))..=n(1).max(n(4)) {
                    for y in n(2).min(n(5))..=n(2).max(n(5)) {
                        for z in n(3).min(n(6))..=n(3).max(n(6)) {
                            world.set(x, y, z, st)?;
                        }
                    }
                }
            }
        }
        let keys: Vec<(i32, i32)> = world.chunks.keys().copied().collect();
        let lit: Vec<((i32, i32), Chunk)> = keys
            .iter()
            .map(|&(x, z)| {
                let region: [[Option<&Chunk>; 3]; 3] = std::array::from_fn(|dz| {
                    std::array::from_fn(|dx| {
                        world
                            .chunks
                            .get(&(x + dx as i32 - 1, z + dz as i32 - 1))
                            .map(|c| &**c)
                    })
                });
                let mut c = (*world.chunks[&(x, z)]).clone();
                c.sections = light_chunk(&region, &tables, true);
                c.lit = true;
                ((x, z), c)
            })
            .collect();
        for (k, c) in lit {
            world.chunks.insert(k, Arc::new(c));
        }
        world.day_time = s.time;
        world.game_time = 1000;
        let mut mobs: Vec<Mob> = Vec::new();
        for (i, spec) in s.mobs.iter().enumerate() {
            let kind = types
                .types
                .get(&spec.kind)
                .ok_or_else(|| format!("unknown mob {}", spec.kind))?;
            let mut m = Mob::new(
                100_000 + i as u32,
                Arc::new(kind.clone()),
                Vec3::new(spec.pos[0], spec.pos[1], spec.pos[2]),
                spec.yaw,
            );
            if spec.baby {
                m.extra.age = -24000;
            }
            for flag in &spec.flags {
                match flag.as_str() {
                    "persistent" => m.persistent = true,
                    "noai" => m.no_ai = true,
                    other => panic!("unknown mob flag {other}"),
                }
            }
            m.random = LegacyRandom::new(spec.seed);
            mobs.push(m);
        }
        let player = s.player.map(|p| {
            let position = Vec3::new(p[0], p[1], p[2]);
            PlayerView {
                position,
                bounds: Aabb::new(
                    Vec3::new(p[0] - 0.3, p[1], p[2] - 0.3),
                    Vec3::new(p[0] + 0.3, p[1] + 1.8, p[2] + 0.3),
                ),
                eye: [p[0], p[1] + f64::from(1.62_f32), p[2]],
                alive: true,
                creative: s.creative,
                sneaking: false,
                look: [0.0, 0.0, 1.0],
                main_hand: None,
                off_hand: None,
                head: None,
                armor_cover: 0.0,
                spectator: false,
                gold_armor: false,
                hurt_by: None,
                last_hurt: None,
                velocity: Vec3::ZERO,
                yaw: 0.0,
                hero: false,
                uuid: [0, 0x1234, 0, 0x5678],
                allies: Default::default(),
            }
        });
        let mut scratch = LegacyRandom::new(0);
        for (i, m) in mobs.iter().enumerate() {
            print(&mut out, 0, i, m);
        }
        for t in 1..=s.ticks {
            world.game_time += 1;
            let snapshot: Vec<Seen> = mobs.iter().filter(|m| !m.removed).map(seen).collect();
            let w = MobWorld {
                world: &world,
                registry,
                facts: &facts,
                collision: &collision,
                data: &data,
                player: player.clone(),
                others: &snapshot,
                daylight: (s.time.rem_euclid(24000)) < 12000,
                game_time: world.game_time,
                day_time: world.day_time,
                raining: s.rain,
                thundering: s.thunder,
                nether: false,
                block_tags: &items.block_tags,
                biomes: &biomes,
                mob_griefing: true,
                difficulty: LocalDifficulty::new(2, world.day_time, 0, 0.0),
                end: false,
                world_seed: 0,
            };
            let mut events = Vec::new();
            for m in &mut mobs {
                if m.removed {
                    continue;
                }
                if !m.tick(&w, &mut scratch, &mut events) {
                    m.removed = true;
                }
            }
            for (i, m) in mobs.iter().enumerate() {
                print(&mut out, t, i, m);
            }
            for e in &events {
                if let MobEvent::Sound(name, _, volume, pitch) = e {
                    let _ = writeln!(
                        out,
                        "{t} sound minecraft:{name} {} {}",
                        f(*volume),
                        f(*pitch)
                    );
                } else if let MobEvent::SoundId(name, _, volume, pitch) = e {
                    let _ = writeln!(out, "{t} sound {name} {} {}", f(*volume), f(*pitch));
                }
            }
        }
    }
    fs::write(&args[2], out)?;
    Ok(())
}
