//! Integrated registry/cursor differential transport using resident TerrainChunk storage.
use crate::support::shapes::{emit, values};
use horizon_core::{
    geometry::{Aabb, Axis, Vec3, VoxelShape},
    position::{BlockPos, ChunkPos},
};
use horizon_world::{
    block_collision::{BlockCollisionRegistry, DynamicCollisionWorld, ShulkerCollisionState},
    block_collision_scan::{BlockCollisionScan, BlockCollisionWorld, CollisionScanError},
    collision::CachedCollisionRegistry,
    collision_context::CollisionContext,
    dynamic_support::DynamicSupportWorld,
    edge_backoff::{EdgeBackoffInput, EdgeBackoffWorld, back_off_from_edge},
    entity_obstruction::EntityIdentity,
    fluid::FluidState,
    fluid_occupancy::{FluidOccupancyWorld, FluidPresence},
    free_space::offset_is_free,
    heightmap::HeightmapState,
    liquid_collision::LiquidCollisionWorld,
    nbt::{self, Limits},
    powder_snow_collision::{PowderSnowEntity, PowderSnowWorld},
    registry::{BlockRegistry, BlockStateId},
    registry_collision_scan::{CollisionChunks, RegistryCollisionWorld},
    suffocation::{SuffocationDefinition, SuffocationRegistry},
    world_movement::{
        CollisionCheckRequest, EntityMovementWorld, MovementRequest, no_world_collision,
        resolve_world_movement,
    },
    worldgen::{
        tags::TagDefinitions,
        terrain::{TerrainBlockStates, TerrainChunk},
    },
};
use serde_json::Value;
use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet},
    fs,
    io::{self, Write},
    path::Path,
    rc::Rc,
    sync::Arc,
};
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
struct Trace {
    out: io::BufWriter<fs::File>,
    calls: i32,
    fail: i32,
}
impl Trace {
    fn query(&mut self, text: String) -> io::Result<()> {
        writeln!(self.out, "{text}")?;
        let call = self.calls;
        self.calls += 1;
        if call == self.fail {
            Err(io::Error::other("injected resident world failure"))
        } else {
            Ok(())
        }
    }
}
type SharedTrace = Rc<RefCell<Trace>>;
struct Entities {
    mode: u64,
    scope: &'static str,
    trace: SharedTrace,
}
impl Entities {
    fn query(&mut self, p: [i32; 3]) -> io::Result<()> {
        self.trace
            .borrow_mut()
            .query(format!("entity {} {} {} {}", self.scope, p[0], p[1], p[2]))
    }
}
impl DynamicSupportWorld for Entities {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> io::Result<Option<bool>> {
        self.query(p)?;
        Ok(if self.mode == 0 {
            None
        } else {
            Some(self.mode == 1)
        })
    }
    fn moving_piston_shape(&mut self, p: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        self.query(p)?;
        Ok(None)
    }
}
impl DynamicCollisionWorld for Entities {
    fn shulker_collision_state(
        &mut self,
        p: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        self.query(p)?;
        let progress = if self.mode == 1 { 0.0 } else { 0.5 };
        Ok((self.mode != 0).then_some(ShulkerCollisionState {
            previous_progress: progress,
            progress,
        }))
    }
}
#[derive(Clone, Copy)]
struct Air(bool);
impl HeightmapState for Air {
    fn is_air(self) -> bool {
        self.0
    }
    fn is_air_block(self) -> bool {
        self.0
    }
    fn has_fluid(self) -> bool {
        false
    }
    fn blocks_motion_in_heightmap(self) -> bool {
        false
    }
    fn blocks_motion_in_heightmap_no_leaves(self) -> bool {
        false
    }
}
#[derive(Clone)]
struct Properties(Arc<Vec<bool>>);
impl TerrainBlockStates for Properties {
    type State = Air;
    fn heightmap_state(&self, id: BlockStateId) -> Air {
        Air(self.0[id.0 as usize])
    }
}
struct MovementShapes {
    entities: Vec<VoxelShape>,
    border: Option<VoxelShape>,
}
struct World<'r> {
    movement: Option<MovementShapes>,
    chunks: BTreeMap<(i32, i32), (TerrainChunk<'r, Properties>, Entities)>,
    global: Entities,
    trace: SharedTrace,
}
fn query_bounds(trace: &SharedTrace, label: &str, b: Aabb) -> io::Result<()> {
    let text = format!(
        "{label} {:016x} {:016x} {:016x} {:016x} {:016x} {:016x}",
        b.min(Axis::X).to_bits(),
        b.min(Axis::Y).to_bits(),
        b.min(Axis::Z).to_bits(),
        b.max(Axis::X).to_bits(),
        b.max(Axis::Y).to_bits(),
        b.max(Axis::Z).to_bits()
    );
    trace.borrow_mut().query(text)
}
fn box_shape(v: &Value) -> io::Result<VoxelShape> {
    VoxelShape::from_box(Aabb::new(
        Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
        Vec3::new(number(&v[3]), number(&v[4]), number(&v[5])),
    ))
    .map_err(io::Error::other)
}
impl FluidOccupancyWorld for World<'_> {
    type Error = io::Error;
    type Chunk = (i32, i32);
    fn minimum_section_y(&mut self) -> io::Result<i32> {
        self.trace.borrow_mut().query("liquid-min-section".into())?;
        Ok(-1)
    }
    fn maximum_section_y(&mut self) -> io::Result<i32> {
        self.trace.borrow_mut().query("liquid-max-section".into())?;
        Ok(1)
    }
    fn chunk(&mut self, x: i32, z: i32) -> io::Result<Self::Chunk> {
        self.trace
            .borrow_mut()
            .query(format!("liquid-chunk {x} {z}"))?;
        if self.chunks.contains_key(&(x, z)) {
            Ok((x, z))
        } else {
            Err(io::Error::other("missing required liquid chunk"))
        }
    }
    fn section(
        &mut self,
        chunk: Self::Chunk,
        y: i32,
    ) -> io::Result<&horizon_world::chunk::BlockSection<'_>> {
        self.trace
            .borrow_mut()
            .query(format!("liquid-section {y}"))?;
        self.chunks[&chunk]
            .0
            .section((y + 1) as usize)
            .ok_or_else(|| io::Error::other("missing section"))
    }
    fn minimum_y(&mut self) -> io::Result<i32> {
        self.trace.borrow_mut().query("liquid-min-y".into())?;
        Ok(-16)
    }
    fn maximum_y(&mut self) -> io::Result<i32> {
        self.trace.borrow_mut().query("liquid-max-y".into())?;
        Ok(31)
    }
}
impl EntityMovementWorld for World<'_> {
    type Error = io::Error;
    fn entity_collisions(
        &mut self,
        _: EntityIdentity,
        bounds: Aabb,
    ) -> io::Result<Vec<VoxelShape>> {
        query_bounds(&self.trace, "entities", bounds)?;
        Ok(self.movement.as_ref().unwrap().entities.clone())
    }
    fn movement_border(
        &mut self,
        _: EntityIdentity,
        bounds: Aabb,
    ) -> io::Result<Option<VoxelShape>> {
        self.trace.borrow_mut().query("border".to_owned())?;
        query_bounds(&self.trace, "border-test", bounds)?;
        let border = &self.movement.as_ref().unwrap().border;
        if border.is_some() {
            self.trace.borrow_mut().query("border-shape".to_owned())?;
        }
        Ok(border.clone())
    }
}
struct EdgeWorld<'w, 'c, W> {
    world: &'w mut W,
    context: CollisionContext<'c>,
    step: f32,
    grounded: bool,
    staying: bool,
}
impl<W, E> EdgeBackoffWorld for EdgeWorld<'_, '_, W>
where
    W: BlockCollisionWorld<Error = E> + EntityMovementWorld<Error = E>,
{
    type Error = CollisionScanError<E>;
    fn max_up_step(&mut self) -> Result<f32, Self::Error> {
        Ok(self.step)
    }
    fn staying_on_ground_surface(&mut self) -> Result<bool, Self::Error> {
        Ok(self.staying)
    }
    fn on_ground(&mut self) -> Result<bool, Self::Error> {
        Ok(self.grounded)
    }
    fn no_collision(&mut self, bounds: Aabb) -> Result<bool, Self::Error> {
        no_world_collision(
            self.world,
            CollisionCheckRequest {
                entity: EntityIdentity(1),
                context: self.context,
                bounds,
                candidate_budget: 1_000_000,
            },
        )
    }
}
impl DynamicSupportWorld for World<'_> {
    type Error = io::Error;
    fn shulker_is_closed(&mut self, p: [i32; 3]) -> io::Result<Option<bool>> {
        self.global.shulker_is_closed(p)
    }
    fn moving_piston_shape(&mut self, p: [i32; 3]) -> io::Result<Option<VoxelShape>> {
        self.global.moving_piston_shape(p)
    }
}
impl DynamicCollisionWorld for World<'_> {
    fn shulker_collision_state(
        &mut self,
        p: [i32; 3],
    ) -> io::Result<Option<ShulkerCollisionState>> {
        self.global.shulker_collision_state(p)
    }
}
impl LiquidCollisionWorld for World<'_> {
    type Error = io::Error;
    fn entity_liquid_shape(&mut self, _: EntityIdentity) -> io::Result<VoxelShape> {
        panic!("unexpected entity query")
    }
    fn fluid(&mut self, _: BlockPos) -> io::Result<FluidState> {
        panic!("unexpected fluid query")
    }
    fn entity_can_stand(&mut self, _: EntityIdentity, _: FluidState) -> io::Result<bool> {
        panic!("unexpected entity query")
    }
}
impl PowderSnowWorld for World<'_> {
    type Error = io::Error;
    fn powder_snow_entity(&mut self, _: EntityIdentity) -> io::Result<PowderSnowEntity> {
        panic!("unexpected entity query")
    }
    fn powder_snow_walkable_tag(&mut self, _: EntityIdentity) -> io::Result<bool> {
        panic!("unexpected entity query")
    }
    fn feet_are_leather_boots(&mut self, _: EntityIdentity) -> io::Result<bool> {
        panic!("unexpected entity query")
    }
}
impl CollisionChunks for World<'_> {
    type Chunk = ChunkPos;
    type ChunkWorld = Entities;
    fn collision_chunk(&mut self, p: ChunkPos) -> io::Result<Option<ChunkPos>> {
        self.trace
            .borrow_mut()
            .query(format!("chunk {} {}", p.x, p.z))?;
        Ok(self.chunks.contains_key(&(p.x, p.z)).then_some(p))
    }
    fn block_state(&mut self, chunk: ChunkPos, p: BlockPos) -> io::Result<BlockStateId> {
        assert_eq!(chunk, p.chunk());
        self.trace
            .borrow_mut()
            .query(format!("block {} {} {}", p.x, p.y, p.z))?;
        Ok(self.chunks[&(chunk.x, chunk.z)].0.block(p.x, p.y, p.z))
    }
    fn chunk_world(&mut self, p: ChunkPos) -> &mut Entities {
        &mut self.chunks.get_mut(&(p.x, p.z)).unwrap().1
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
    let binding = SuffocationRegistry::new(collisions, &definitions, &natural)?;
    let fluids = FluidPresence::new(&registry)?;
    let properties = Properties(Arc::new(
        (0..registry.len())
            .map(|id| {
                matches!(
                    registry
                        .state(BlockStateId(id as u32))
                        .unwrap()
                        .name
                        .as_str(),
                    "minecraft:air" | "minecraft:cave_air" | "minecraft:void_air"
                )
            })
            .collect(),
    ));
    let trace = Rc::new(RefCell::new(Trace {
        out: io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?),
        calls: 0,
        fail: -1,
    }));
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        {
            let mut t = trace.borrow_mut();
            t.calls = 0;
            t.fail = f["fail"].as_i64().unwrap() as i32;
        }
        let mut chunks = BTreeMap::new();
        for c in f["chunks"].as_array().unwrap() {
            let x = c[0].as_i64().unwrap() as i32;
            let z = c[1].as_i64().unwrap() as i32;
            chunks.insert(
                (x, z),
                (
                    TerrainChunk::new(ChunkPos::new(x, z), -16, 48, &registry, properties.clone())?,
                    Entities {
                        mode: f["entities"][1].as_u64().unwrap(),
                        scope: "chunk",
                        trace: trace.clone(),
                    },
                ),
            );
        }
        for b in f["blocks"].as_array().unwrap() {
            let x = b[0].as_i64().unwrap() as i32;
            let y = b[1].as_i64().unwrap() as i32;
            let z = b[2].as_i64().unwrap() as i32;
            chunks
                .get_mut(&(x >> 4, z >> 4))
                .unwrap()
                .0
                .set_block_worldgen(x, y, z, BlockStateId(b[3].as_u64().unwrap() as u32))?;
        }
        let movement = f
            .get("movement")
            .map(|m| -> io::Result<_> {
                Ok(MovementShapes {
                    entities: m["entities"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .map(box_shape)
                        .collect::<io::Result<_>>()?,
                    border: if m["border"].is_null() {
                        None
                    } else {
                        Some(box_shape(&m["border"])?)
                    },
                })
            })
            .transpose()?;
        let mut world = World {
            movement,
            chunks,
            global: Entities {
                mode: f["entities"][0].as_u64().unwrap(),
                scope: "global",
                trace: trace.clone(),
            },
            trace: trace.clone(),
        };
        let mut bound = RegistryCollisionWorld::new(&binding, &mut world)?;
        let context = match f["context"].as_u64().unwrap() {
            0 => CollisionContext::empty(false),
            1 => CollisionContext::empty(true),
            2 => CollisionContext::position(number(&f["y"])),
            _ => CollisionContext::placement(None),
        };
        let v = &f["box"];
        let bounds = Aabb::new(
            Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
            Vec3::new(number(&v[3]), number(&v[4]), number(&v[5])),
        );
        if let Some(m) = f.get("movement") {
            let v = &m["delta"];
            if let Some(edge) = m.get("edge") {
                let mut world = EdgeWorld {
                    world: &mut bound,
                    context,
                    step: number(&m["step"]) as f32,
                    grounded: m["grounded"].as_bool().unwrap(),
                    staying: edge["staying"].as_bool().unwrap(),
                };
                let result = back_off_from_edge(
                    &mut world,
                    EdgeBackoffInput {
                        movement: Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
                        bounds,
                        flying: edge["flying"].as_bool().unwrap(),
                        self_or_player: matches!(
                            edge["mover"].as_str().unwrap(),
                            "SELF" | "PLAYER"
                        ),
                        fall_distance: number(&edge["fall"]),
                        query_budget: 10000,
                    },
                );
                let mut t = trace.borrow_mut();
                match result {
                    Ok(v) => values(&mut t.out, "edge", [v.x, v.y, v.z])?,
                    Err(_) => writeln!(t.out, "error")?,
                };
                let calls = t.calls;
                writeln!(t.out, "done edge {calls}")?;
                continue;
            }
            if m.get("free").and_then(Value::as_bool) == Some(true) {
                let result = offset_is_free(
                    &mut bound,
                    &fluids,
                    CollisionCheckRequest {
                        entity: EntityIdentity(1),
                        context,
                        bounds,
                        candidate_budget: 1_000_000,
                    },
                    Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
                    1_000_000,
                );
                let mut t = trace.borrow_mut();
                match result {
                    Ok(b) => writeln!(t.out, "free {b}")?,
                    Err(_) => writeln!(t.out, "error")?,
                };
                let calls = t.calls;
                writeln!(t.out, "done free {calls}")?;
                continue;
            }
            if m.get("check").and_then(Value::as_bool) == Some(true) {
                let result = no_world_collision(
                    &mut bound,
                    CollisionCheckRequest {
                        entity: EntityIdentity(1),
                        context,
                        bounds,
                        candidate_budget: 1_000_000,
                    },
                );
                let mut t = trace.borrow_mut();
                match result {
                    Ok(value) => writeln!(t.out, "clear {value}")?,
                    Err(_) => writeln!(t.out, "error")?,
                };
                let calls = t.calls;
                writeln!(t.out, "done check {calls}")?;
                continue;
            }
            let result = resolve_world_movement(
                &mut bound,
                MovementRequest {
                    entity: EntityIdentity(1),
                    context,
                    bounds,
                    movement: Vec3::new(number(&v[0]), number(&v[1]), number(&v[2])),
                    max_step: number(&m["step"]) as f32,
                    on_ground: m["grounded"].as_bool().unwrap(),
                    candidate_budget: 1_000_000,
                },
            );
            let mut t = trace.borrow_mut();
            match result {
                Ok(v) => values(&mut t.out, "movement", [v.x, v.y, v.z])?,
                Err(_) => writeln!(t.out, "error")?,
            }
            let calls = t.calls;
            writeln!(t.out, "done movement {calls}")?;
            continue;
        }
        let mut scan = BlockCollisionScan::new(
            &mut bound,
            context,
            bounds,
            f["suffocating"].as_bool().unwrap(),
            1_000_000,
        )?;
        let mut hits = 0;
        for _ in 0..f["limit"].as_u64().unwrap() {
            match scan.next() {
                None => break,
                Some(Ok(hit)) => {
                    let mut t = trace.borrow_mut();
                    let p = hit.position;
                    writeln!(t.out, "hit {} {} {}", p.x, p.y, p.z)?;
                    emit(&mut t.out, &hit.shape)?;
                    hits += 1;
                }
                Some(Err(_)) => {
                    writeln!(trace.borrow_mut().out, "error")?;
                    break;
                }
            }
        }
        let mut t = trace.borrow_mut();
        let calls = t.calls;
        writeln!(t.out, "done {hits} {calls}")?;
    }
    Ok(())
}
