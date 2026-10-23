use crate::support::common::f64_bits;
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::{
    entity_obstruction::EntityIdentity,
    fluid::{FluidKind, FluidState},
    fluid_shape::FluidShapeCache,
    interaction_shapes::InteractionShapes,
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
    registry_fall_ray::{FallRayRegistry, FallRayWorld, fall_reset_ray},
};
use serde_json::Value;
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
    path::Path,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn vector(v: &Value) -> Vec3 {
    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2]))
}
fn pos(p: BlockPos) -> String {
    format!("{} {} {}", p.x, p.y, p.z)
}
struct World<'a, 's, 'r> {
    f: &'a Value,
    op: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    p: BlockPos,
    registry: &'a FallRayRegistry<'s, 'r>,
}
impl World<'_, '_, '_> {
    fn flag(&self, n: &str) -> bool {
        self.f[n].as_bool().unwrap()
    }
    fn query(&mut self, s: String) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.op["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected registered ray failure"))
        } else {
            Ok(())
        }
    }
    fn at(&self, p: BlockPos) -> BlockStateId {
        BlockStateId(if p == self.p {
            self.f["state"].as_u64().unwrap() as u32
        } else if p == self.p.offset(0, 1, 0) {
            self.op["above"].as_u64().unwrap() as u32
        } else {
            0
        })
    }
}
impl FallRayWorld for World<'_, '_, '_> {
    type Error = io::Error;
    fn block_state(&mut self, p: BlockPos) -> io::Result<BlockStateId> {
        self.query(format!("block {}", pos(p)))?;
        Ok(self.at(p))
    }
    fn fluid_state(&mut self, p: BlockPos) -> io::Result<FluidState> {
        self.query(format!("fluid {}", pos(p)))?;
        self.registry.fluid_for_block(self.at(p))
    }
    fn is_server_level(&self) -> bool {
        self.flag("server")
    }
    fn entity_is_player(&mut self, _: EntityIdentity) -> io::Result<bool> {
        self.query("player".into())?;
        Ok(self.flag("player"))
    }
    fn players_nether_portal_delay(&mut self) -> io::Result<i32> {
        self.query("rules".into())?;
        self.query("portal-delay".into())?;
        Ok(self.f["delay"].as_i64().unwrap() as i32)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    let read = |n: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(nbt::read_named(&fs::read(dir.join(n))?, Limits::default())?
            .0
            .tag)
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let interactions = InteractionShapes::from_nbt(&read("interaction-shapes.nbt")?, &registry)?;
    let tags: Value = serde_json::from_str(&fs::read_to_string(dir.join("bound-tags.json"))?)?;
    let names = |n: &str| -> BTreeSet<String> {
        tags[n]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };
    let all = (0..registry.len())
        .map(|i| registry.state(BlockStateId(i as u32)).unwrap().name.clone())
        .collect();
    let blocks = [names("reset"), BTreeSet::new(), all];
    let fluids = [
        names("water"),
        BTreeSet::new(),
        FluidKind::ALL
            .into_iter()
            .map(|f| f.name().to_owned())
            .collect(),
    ];
    let mut bindings = Vec::new();
    for b in &blocks {
        for f in &fluids {
            bindings.push(FallRayRegistry::new(&interactions, b, f)?);
        }
    }
    assert!(
        FallRayRegistry::new(
            &interactions,
            &BTreeSet::from(["minecraft:missing".into()]),
            &fluids[0]
        )
        .is_err()
    );
    assert!(
        FallRayRegistry::new(
            &interactions,
            &blocks[0],
            &BTreeSet::from(["minecraft:missing".into()])
        )
        .is_err()
    );
    assert!(
        bindings[0]
            .fluid_for_block(BlockStateId(registry.len() as u32))
            .is_err()
    );
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let p = &f["position"];
        let p = BlockPos::new(
            p[0].as_i64().unwrap() as i32,
            p[1].as_i64().unwrap() as i32,
            p[2].as_i64().unwrap() as i32,
        );
        let binding = &bindings
            [(f["block_tags"].as_u64().unwrap() * 3 + f["fluid_tags"].as_u64().unwrap()) as usize];
        let entity = f["entity"].as_bool().unwrap().then_some(EntityIdentity(0));
        let mut cache = FluidShapeCache::default();
        for op in f["ops"].as_array().unwrap() {
            let mut w = World {
                f: &f,
                op,
                out: &mut out,
                calls: 0,
                p,
                registry: binding,
            };
            match fall_reset_ray(
                &mut w,
                binding,
                &mut cache,
                entity,
                vector(&op["from"]),
                vector(&op["to"]),
                10000,
            ) {
                Ok(r) => {
                    let h = r.hit;
                    writeln!(
                        w.out,
                        "hit {} {:016x} {:016x} {:016x} {} {} {}",
                        r.miss,
                        f64_bits(h.location.x),
                        f64_bits(h.location.y),
                        f64_bits(h.location.z),
                        format!("{:?}", h.direction).to_lowercase(),
                        pos(h.block),
                        h.inside
                    )?;
                }
                Err(_) => writeln!(w.out, "error")?,
            }
            writeln!(w.out, "cached {}", cache.cached_states())?;
            writeln!(w.out, "queries {}", w.calls)?;
        }
    }
    Ok(())
}
