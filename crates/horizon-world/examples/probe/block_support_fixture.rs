use horizon_core::geometry::Direction;
use horizon_world::{
    block_support::{CachedBlockSupportRegistry, SupportType},
    nbt::{self, Limits, Tag},
    registry::{BlockRegistry, BlockStateId},
};
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).expect("private fixture directory");
    let dir = Path::new(&arg);
    let read = |name: &str| -> Result<Tag, Box<dyn std::error::Error>> {
        Ok(
            nbt::read_named(&fs::read(dir.join(name))?, Limits::default())?
                .0
                .tag,
        )
    };
    let registry = BlockRegistry::from_nbt(&read("block-registry.nbt")?)?;
    let tag = read("cached-support.nbt")?;
    let support = CachedBlockSupportRegistry::from_nbt(&tag, &registry)?;
    assert_eq!(registry.len(), 35723);
    assert_eq!(support.cached_state_count(), 35524);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for id in 0..registry.len() {
        let state = BlockStateId(id as u32);
        if support.cached_shape(state)?.is_none() {
            writeln!(out, "state {id} uncached")?;
            for d in Direction::ALL {
                assert!(
                    support
                        .is_face_sturdy(state, d, SupportType::Full)?
                        .is_none()
                );
            }
            continue;
        }
        writeln!(out, "state {id} cached")?;
        for d in Direction::ALL {
            write!(out, "support {}", d as u8)?;
            for t in [SupportType::Full, SupportType::Center, SupportType::Rigid] {
                write!(out, " {}", support.is_face_sturdy(state, d, t)?.unwrap())?;
            }
            writeln!(
                out,
                " {}",
                support
                    .is_face_sturdy(state, d, SupportType::Full)?
                    .unwrap()
            )?;
        }
    }
    let mut bad = tag.clone();
    *bad.get_mut("Kind").unwrap() = Tag::String("collision".into());
    assert!(CachedBlockSupportRegistry::from_nbt(&bad, &registry).is_err());
    assert!(
        support
            .is_face_sturdy(
                BlockStateId(registry.len() as u32),
                Direction::Down,
                SupportType::Full
            )
            .is_err()
    );
    Ok(())
}
