use crate::support::common::f64_bits;
use crate::support::shapes::bounds;
use horizon_core::{
    geometry::{Aabb, Vec3, VoxelShape},
    position::BlockPos,
};
use horizon_world::{
    block_entity_push::BlockPushWorld,
    block_landing::*,
    collision::CachedCollisionRegistry,
    entity_obstruction::EntityIdentity,
    movement_properties::MovementProperties,
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn fbits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc00000 } else { v.to_bits() }
}
fn pos(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
fn bbox(v: &Value) -> Aabb {
    Aabb::new(
        Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
        Vec3::new(number(&v[3]), number(&v[4]), number(&v[5])),
    )
}
struct World<'a, 'r> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    state: BlockStateId,
    air: BlockStateId,
    collision: &'a CachedCollisionRegistry<'r>,
    entities: Vec<Aabb>,
}
impl World<'_, '_> {
    fn flag(&self, k: &str) -> bool {
        self.f[k].as_bool().unwrap()
    }
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.f["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected landing failure"))
        } else {
            Ok(())
        }
    }
}
impl BlockPushWorld for World<'_, '_> {
    type Error = io::Error;
    fn collision_shape(&mut self, s: BlockStateId, _: BlockPos) -> io::Result<VoxelShape> {
        Ok(self
            .collision
            .cached_shape(s)?
            .expect("registered farmland/base shapes cached")
            .clone())
    }
    fn entities(&mut self, b: Aabb) -> io::Result<Vec<EntityIdentity>> {
        self.query(bounds("entities", b))?;
        Ok((0..self.entities.len())
            .map(|i| EntityIdentity(i as u64))
            .collect())
    }
    fn entity_bounds(&self, e: EntityIdentity) -> Aabb {
        self.entities[e.0 as usize]
    }
    fn teleport_relative(&mut self, e: EntityIdentity, v: Vec3) -> io::Result<()> {
        self.query(format!(
            "teleport {} {:016x} {:016x} {:016x}",
            e.0,
            f64_bits(v.x),
            f64_bits(v.y),
            f64_bits(v.z)
        ))?;
        let i = e.0 as usize;
        self.entities[i] = self.entities[i].move_by(v);
        if self.flag("mutate") && i + 1 < self.entities.len() {
            self.entities[i + 1] = self.entities[i + 1].move_by(Vec3::new(0.0, 0.125, 0.0));
        }
        Ok(())
    }
}
impl BlockLandingWorld for World<'_, '_> {
    type DamageSource = &'static str;
    type Sound = &'static str;
    fn is_server_level(&self) -> bool {
        self.flag("server")
    }
    fn is_client_side(&mut self) -> io::Result<bool> {
        self.query("client".into())?;
        Ok(self.flag("client"))
    }
    fn random_float(&mut self) -> io::Result<f32> {
        self.query("random".into())?;
        self.query("float".into())?;
        Ok(single(&self.f["random_float"]))
    }
    fn random_int(&mut self, b: i32) -> io::Result<i32> {
        self.query("random".into())?;
        self.query(format!("int {b}"))?;
        Ok(self.f["random_int"].as_i64().unwrap() as i32)
    }
    fn under_spawn_protection(&mut self, p: BlockPos) -> io::Result<bool> {
        self.query("server".into())?;
        self.query(format!("protection {}", pos(p)))?;
        Ok(self.flag("protected"))
    }
    fn mob_griefing(&mut self) -> io::Result<bool> {
        self.query("rules".into())?;
        self.query("mob-griefing".into())?;
        Ok(self.flag("griefing"))
    }
    fn entity_dimensions(&self) -> (f32, f32) {
        (single(&self.f["width"]), single(&self.f["height"]))
    }
    fn suppressing_bounce(&mut self) -> io::Result<bool> {
        self.query("suppress".into())?;
        Ok(self.flag("suppress"))
    }
    fn damage_source(
        &mut self,
        o: DamageSourceOwner,
        k: LandingDamage,
    ) -> io::Result<Self::DamageSource> {
        self.query(
            match o {
                DamageSourceOwner::Entity => "entity-sources",
                DamageSourceOwner::Level => "level-sources",
            }
            .into(),
        )?;
        let k = match k {
            LandingDamage::Fall => "fall",
            LandingDamage::Stalagmite => "stalagmite",
        };
        self.query(format!("{k}-source"))?;
        Ok(k)
    }
    fn cause_fall_damage(&mut self, d: f64, m: f32, s: Self::DamageSource) -> io::Result<bool> {
        self.query(format!("damage {:016x} {:08x} {s}", f64_bits(d), fbits(m)))?;
        Ok(self.flag("damage"))
    }
    fn known_sound(&self, s: LandingSound) -> Self::Sound {
        match s {
            LandingSound::HoneySlide => "honey-slide",
            LandingSound::HoneyFall => "honey-fall",
            LandingSound::TurtleEggBreak => "egg-break",
            LandingSound::ShelfMushroomBounce => "mushroom",
        }
    }
    fn living_fall_sound(&mut self, big: bool) -> io::Result<Self::Sound> {
        self.query("fall-sounds".into())?;
        Ok(if big { "big" } else { "small" })
    }
    fn entity_sound(&mut self, s: Self::Sound, v: f32, p: f32) -> io::Result<()> {
        self.query(format!(
            "entity-sound {s} {:08x} {:08x}",
            fbits(v),
            fbits(p)
        ))
    }
    fn block_sound(&mut self, p: BlockPos, s: Self::Sound, v: f32, pitch: f32) -> io::Result<()> {
        self.query(format!(
            "block-sound {} {s} {:08x} {:08x}",
            pos(p),
            fbits(v),
            fbits(pitch)
        ))
    }
    fn broadcast_entity_event(&mut self, e: u8) -> io::Result<()> {
        self.query(format!("broadcast {e}"))
    }
    fn destroy_block(&mut self, p: BlockPos, drops: bool) -> io::Result<bool> {
        self.query(format!("destroy {} {drops}", pos(p)))?;
        if self.flag("write") {
            self.state = self.air;
        }
        Ok(self.flag("write"))
    }
    fn set_block(&mut self, p: BlockPos, s: BlockStateId, flags: u32) -> io::Result<bool> {
        self.query(format!("set {} {} {flags}", pos(p), s.0))?;
        if self.flag("write") {
            self.state = s;
        }
        Ok(self.flag("write"))
    }
    fn block_event(
        &mut self,
        e: LandingEvent,
        p: BlockPos,
        s: BlockStateId,
        entity: bool,
    ) -> io::Result<()> {
        self.query(format!(
            "event {} {} {} {entity}",
            match e {
                LandingEvent::BlockChange => "change",
                LandingEvent::BlockDestroy => "destroy",
            },
            pos(p),
            s.0
        ))
    }
    fn level_event(&mut self, e: i32, p: BlockPos, d: i32) -> io::Result<()> {
        self.query(format!("level-event {e} {} {d}", pos(p)))
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let read = |name: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(
            nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?
                .0
                .tag,
        )
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let data = read("movement-properties.nbt")?;
    let properties = MovementProperties::from_nbt(&data, &registry)?;
    let collision = CachedCollisionRegistry::from_nbt(&read("cached-collision.nbt")?, &registry)?;
    let raw: BTreeMap<String, Value> =
        serde_json::from_str(&fs::read_to_string(dir.join("landing-definitions.json"))?)?;
    let mut definitions = BTreeMap::new();
    for (name, d) in raw {
        let fall = match d["fall"].as_str().unwrap() {
            "Block" => FallDefinition::Default,
            "HayBlock" => FallDefinition::Hay,
            "HoneyBlock" => FallDefinition::Honey {
                volume: single(&d["volume"]),
                pitch: single(&d["pitch"]),
            },
            "SlimeBlock" => FallDefinition::Slime,
            "PowderSnowBlock" => FallDefinition::PowderSnow,
            "PointedDripstoneBlock" => FallDefinition::PointedDripstone,
            "FarmlandBlock" => FallDefinition::Farmland {
                base_block: d["base"].as_str().unwrap().into(),
            },
            "TurtleEggBlock" => FallDefinition::TurtleEgg,
            _ => panic!("unknown landing family"),
        };
        definitions.insert(
            name,
            LandingDefinition {
                fall,
                shelf_mushroom_bounce: d["bounce"].as_bool().unwrap(),
            },
        );
    }
    let landing = BlockLandingRegistry::new(&properties, &definitions)?;
    let kinds = [
        LandingEntityKind::Other,
        LandingEntityKind::Living,
        LandingEntityKind::Player,
        LandingEntityKind::Zombie,
        LandingEntityKind::Turtle,
        LandingEntityKind::Bat,
        LandingEntityKind::Item,
        LandingEntityKind::PrimedTnt,
    ];
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let s = BlockStateId(f["state"].as_u64().unwrap() as u32);
        let p = &f["position"];
        let p = BlockPos::new(
            p[0].as_i64().unwrap() as i32,
            p[1].as_i64().unwrap() as i32,
            p[2].as_i64().unwrap() as i32,
        );
        let kind = kinds[f["kind"].as_u64().unwrap() as usize];
        let mut world = World {
            f: &f,
            out: &mut out,
            calls: 0,
            state: s,
            air: registry.default_state("minecraft:air").unwrap(),
            collision: &collision,
            entities: f["entities"].as_array().unwrap().iter().map(bbox).collect(),
        };
        let result = if world.flag("bounce") {
            landing.bounce_on(&mut world, s, p, kind)
        } else {
            landing.fall_on(&mut world, s, p, kind, number(&f["distance"]))
        };
        writeln!(world.out, "{}", if result.is_ok() { "ok" } else { "error" })?;
        writeln!(world.out, "state {}", world.state.0)?;
        for (i, b) in world.entities.iter().enumerate() {
            writeln!(world.out, "{}", bounds(&format!("final {i}"), *b))?;
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
