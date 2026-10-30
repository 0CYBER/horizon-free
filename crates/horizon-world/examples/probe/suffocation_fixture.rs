use horizon_core::geometry::VoxelShape;
use horizon_world::{
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
    collision::CachedCollisionRegistry,
    dynamic_support::DynamicSupportWorld,
    nbt::{self, Limits},
    registry::{BlockRegistry, BlockStateId},
    suffocation::{SuffocationDefinition, SuffocationRegistry},
    worldgen::tags::TagDefinitions,
};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
struct World<'a, W> {
    out: &'a mut W,
    calls: i32,
    fail: i32,
    entity: &'a str,
}
impl<W: Write> World<'_, W> {
    fn query(&mut self, p: [i32; 3]) -> io::Result<()> {
        writeln!(self.out, "entity {} {} {}", p[0], p[1], p[2])?;
        let count = self.calls;
        self.calls += 1;
        if count == self.fail {
            Err(io::Error::other("injected suffocation query failure"))
        } else {
            Ok(())
        }
    }
}
impl<W: Write> DynamicSupportWorld for World<'_, W> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> io::Result<Option<bool>> {
        self.query(p)?;
        Ok(match self.entity {
            "none" | "other" => None,
            "closed" => Some(true),
            _ => Some(false),
        })
    }
    fn moving_piston_shape(&mut self, p: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        self.query(p)?;
        Ok(None)
    }
}
impl<W: Write> DynamicCollisionWorld for World<'_, W> {
    fn shulker_collision_state(
        &mut self,
        p: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        self.query(p)?;
        Ok(None)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let read = |name| -> Result<_, Box<dyn std::error::Error>> {
        Ok(
            nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?
                .0
                .tag,
        )
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let cached = Arc::new(CachedCollisionRegistry::from_nbt(
        &read("cached-collision.nbt")?,
        &registry,
    )?);
    let collisions = Arc::new(BlockCollisionRegistry::new(cached)?);
    let definitions: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("definitions.json"))?)?;
    let definitions = definitions
        .into_iter()
        .map(|(name, kind)| {
            Ok((
                name,
                match kind.as_str() {
                    "tagged_full" => SuffocationDefinition::TaggedFullCollision,
                    "always" => SuffocationDefinition::Always,
                    "never" => SuffocationDefinition::Never,
                    "not_extended" => SuffocationDefinition::NotExtendedPiston,
                    "closed_shulker" => SuffocationDefinition::ClosedShulker,
                    _ => return Err(io::Error::other("unknown suffocation rule")),
                },
            ))
        })
        .collect::<io::Result<BTreeMap<_, _>>>()?;
    let resources: BTreeMap<String, String> =
        serde_json::from_str(&fs::read_to_string(dir.join("tags.json"))?)?;
    let mut tags = TagDefinitions::default();
    for (name, json) in resources {
        tags.apply_json(&name, "vanilla", &json)?;
    }
    let built = tags.build(|name, _| registry.default_state(name).map(|_| name.to_owned()))?;
    if !built.missing.is_empty() {
        return Err("unresolved native block tags".into());
    }
    let natural = built
        .tags
        .get("minecraft:causes_suffocation")
        .ok_or("missing suffocation tag")?
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let all = (0..registry.len())
        .map(|i| registry.state(BlockStateId(i as u32)).unwrap().name.clone())
        .collect::<BTreeSet<_>>();
    let bindings = [
        SuffocationRegistry::new(collisions.clone(), &definitions, &natural)?,
        SuffocationRegistry::new(collisions.clone(), &definitions, &BTreeSet::new())?,
        SuffocationRegistry::new(collisions.clone(), &definitions, &all)?,
    ];
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let state = BlockStateId(f["state"].as_u64().unwrap() as u32);
        let p = std::array::from_fn(|i| f["position"][i].as_i64().unwrap() as i32);
        let block = collisions.collision_block(state)?;
        writeln!(
            out,
            "metadata {} {} {}",
            state.0, block.large_collision_shape, block.moving_piston
        )?;
        let mut world = World {
            out: &mut out,
            calls: 0,
            fail: f["fail"].as_i64().unwrap() as i32,
            entity: f["entity"].as_str().unwrap(),
        };
        match bindings[f["tags"].as_u64().unwrap() as usize].is_suffocating(&mut world, state, p) {
            Ok(v) => writeln!(world.out, "suffocating {v}")?,
            Err(_) => writeln!(world.out, "suffocating error")?,
        }
        writeln!(world.out, "queries {}", world.calls)?;
    }
    Ok(())
}
