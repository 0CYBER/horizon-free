//! Queries imported cached shapes through resident section state IDs.
use horizon_core::{
    geometry::{Aabb, Axis, Vec3},
    position::BlockPos,
};
use horizon_world::{
    chunk::BlockSection,
    collision::CachedCollisionRegistry,
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
};
use std::{
    error::Error,
    fs,
    io::{BufRead, Write},
    path::PathBuf,
};

fn bits(value: f64) -> u64 {
    if value.is_nan() {
        0x7ff8_0000_0000_0000
    } else {
        value.to_bits()
    }
}
fn values(
    out: &mut impl Write,
    label: &str,
    numbers: impl IntoIterator<Item = f64>,
) -> std::io::Result<()> {
    write!(out, "{label}")?;
    for value in numbers {
        write!(out, " {:016x}", bits(value))?;
    }
    writeln!(out)
}
fn box_values(out: &mut impl Write, label: &str, b: Aabb) -> std::io::Result<()> {
    values(
        out,
        label,
        b.min_position()
            .components()
            .into_iter()
            .chain(b.max_position().components()),
    )
}
struct Input<'a>(std::str::SplitWhitespace<'a>);
impl Input<'_> {
    fn integer(&mut self) -> i32 {
        self.0.next().unwrap().parse().unwrap()
    }
    fn number(&mut self) -> f64 {
        f64::from_bits(u64::from_str_radix(self.0.next().unwrap(), 16).unwrap())
    }
    fn vector(&mut self) -> Vec3 {
        Vec3::new(self.number(), self.number(), self.number())
    }
}
pub fn main() -> Result<(), Box<dyn Error>> {
    let base = PathBuf::from(crate::args().nth(1).ok_or("fixture directory required")?);
    let (registry_root, _) = nbt::read_named(
        &fs::read(base.join("block-registry.nbt"))?,
        Limits::default(),
    )?;
    let registry = BlockRegistry::from_nbt(&registry_root.tag)?;
    let (root, _) = nbt::read_named(
        &fs::read(base.join("cached-collision.nbt"))?,
        Limits::default(),
    )?;
    let collisions = CachedCollisionRegistry::from_nbt(&root.tag, &registry)?;
    assert!(collisions.has_canonical_identities());
    let mut section = BlockSection::uniform(BlockStateId(0), &registry)?;
    let mut out = std::io::BufWriter::new(fs::File::create(base.join("rust.txt"))?);
    let mut identities = std::collections::HashMap::new();
    for id in 0..registry.len() {
        let index = id % 4096;
        section.set(index, BlockStateId(id as u32))?;
        match collisions.section_shape(&section, index)? {
            None => writeln!(out, "state {id} uncached")?,
            Some(shape) => {
                writeln!(out, "state {id} cached {}", shape.boxes().len())?;
                let count = identities.len();
                let identity = *identities.entry(std::ptr::from_ref(shape)).or_insert(count);
                writeln!(out, "identity {identity}")?;
                writeln!(
                    out,
                    "canonical {} {}",
                    shape.same_identity(&horizon_core::geometry::VoxelShape::empty()),
                    shape.same_identity(&horizon_core::geometry::VoxelShape::block())
                )?;
                let flags = shape.cube_coordinate_axes();
                writeln!(
                    out,
                    "cube {} {}",
                    u8::from(shape.is_cube()),
                    u8::from(flags[0]) + 2 * u8::from(flags[1]) + 4 * u8::from(flags[2])
                )?;
                values(
                    &mut out,
                    "axis-extrema",
                    Axis::ALL
                        .map(|axis| shape.min(axis))
                        .into_iter()
                        .chain(Axis::ALL.map(|axis| shape.max(axis))),
                )?;
                if let Some(b) = shape.bounds() {
                    box_values(&mut out, "bounds", b)?;
                } else {
                    writeln!(out, "bounds none")?;
                }
                for b in shape.boxes() {
                    box_values(&mut out, "part", *b)?;
                }
            }
        }
    }
    let other_registry = BlockRegistry::from_nbt(&registry_root.tag)?;
    let other_section = BlockSection::uniform(BlockStateId(0), &other_registry)?;
    assert!(collisions.section_shape(&other_section, 0).is_err());
    assert!(
        collisions
            .cached_shape(BlockStateId(registry.len() as u32))
            .is_err()
    );
    assert!(collisions.section_shape(&section, 4096).is_err());
    // Import failures must not create a usable partial collision registry.
    let mut corrupt = root.tag.clone();
    *corrupt.get_mut("DataVersion").unwrap() = Tag::Int(0);
    assert!(CachedCollisionRegistry::from_nbt(&corrupt, &registry).is_err());
    let mut corrupt = root.tag.clone();
    let Some(Tag::IntArray(mapping)) = corrupt.get_mut("cached_shapes") else {
        unreachable!()
    };
    mapping[0] = -2;
    assert!(CachedCollisionRegistry::from_nbt(&corrupt, &registry).is_err());
    let mut corrupt = root.tag.clone();
    let Some(Tag::List(states)) = corrupt.get_mut("states") else {
        unreachable!()
    };
    states.swap(0, 1);
    assert!(CachedCollisionRegistry::from_nbt(&corrupt, &registry).is_err());
    let mut count = 0;
    for line in std::io::BufReader::new(fs::File::open(base.join("inputs.txt"))?).lines() {
        let line = line?;
        let mut input = Input(line.split_whitespace());
        let id = input.integer();
        let block = BlockPos::new(input.integer(), input.integer(), input.integer());
        let b = Aabb::new(input.vector(), input.vector());
        let start = input.vector();
        let end = input.vector();
        let point = input.vector();
        let amount = input.number();
        section.set(count % 4096, BlockStateId(id as u32))?;
        let shape = collisions
            .section_shape(&section, count % 4096)?
            .ok_or("query used uncached state")?;
        let moved = shape.moved(Vec3::from_block(block));
        values(
            &mut out,
            "collide",
            Axis::ALL.map(|axis| moved.collide(axis, b, amount)),
        )?;
        if let Some(h) = shape.clip(start, end, block) {
            writeln!(
                out,
                "hit {} {} {:016x} {:016x} {:016x}",
                h.direction as u8,
                u8::from(h.inside),
                bits(h.location.x),
                bits(h.location.y),
                bits(h.location.z)
            )?;
        } else {
            writeln!(out, "hit none")?;
        }
        if let Some(p) = moved.closest_point(point) {
            values(&mut out, "closest", p.components())?;
        } else {
            writeln!(out, "closest none")?;
        }
        writeln!(
            out,
            "index {} {} {}",
            moved.find_index(Axis::X, point.x),
            moved.find_index(Axis::Y, point.y),
            moved.find_index(Axis::Z, point.z)
        )?;
        assert!(input.0.next().is_none());
        count += 1;
    }
    out.flush()?;
    println!(
        "HORIZON_COLLISION_RESULT {{\"registry_states\":{},\"cached_states\":{},\"uncached_states\":{},\"unique_shapes\":{},\"unique_grids\":{},\"queries\":{}}}",
        registry.len(),
        collisions.cached_state_count(),
        registry.len() - collisions.cached_state_count(),
        collisions.shape_count(),
        collisions.unique_grid_count(),
        count
    );
    Ok(())
}
