//! Exercises world primitives using local vanilla-generated fixtures.
use horizon_world::{
    chunk::{BlockSection, BlockState, PalettedContainer},
    nbt::{self, JavaString, Limits, NamedTag, Tag},
    registry::{BlockRegistry, BlockStateId},
};
use std::{error::Error, fs, path::PathBuf};

pub fn main() -> Result<(), Box<dyn Error>> {
    let base = PathBuf::from(crate::args().nth(1).ok_or("expected fixture directory")?);
    let output = base.join("rust/palettes");
    fs::create_dir_all(&output)?;
    for entry in fs::read_dir(base.join("palettes"))? {
        let entry = entry?;
        let filename = entry
            .file_name()
            .into_string()
            .map_err(|_| "invalid filename")?;
        let biome = filename.starts_with("biome-");
        let count: usize = filename
            .split_once('-')
            .unwrap()
            .1
            .trim_end_matches(".nbt")
            .parse()?;
        let len = if biome { 64 } else { 4096 };
        let (root, _) = nbt::read_named(&fs::read(entry.path())?, Limits::default())?;
        let mut palette = PalettedContainer::from_nbt(&root.tag, len, if biome { 1 } else { 4 })?;
        for i in 0..len {
            assert_eq!(
                palette.get(i)?,
                &Tag::String(JavaString::from(format!("state{}", i % count).as_str()))
            );
        }
        palette.set(len - 1, Tag::String(JavaString::from("state4096")))?;
        for kind in ["edited", "compact"] {
            if kind == "compact" {
                palette.compact()?;
            }
            let root = NamedTag {
                name: root.name.clone(),
                tag: palette.to_nbt(),
            };
            fs::write(
                output.join(format!("{kind}-{filename}")),
                nbt::write_named(&root, Limits::default())?,
            )?;
        }
    }
    let (registry_root, _) = nbt::read_named(
        &fs::read(base.join("block-registry.nbt"))?,
        Limits::default(),
    )?;
    let registry = BlockRegistry::from_nbt(&registry_root.tag)?;
    let Some(Tag::List(states)) = registry_root.tag.get("states") else {
        return Err("missing states".into());
    };
    for (i, tag) in states.iter().enumerate() {
        let state = BlockState::from_nbt(tag)?;
        assert_eq!(registry.id(&state), Some(BlockStateId(i as u32)));
        assert_eq!(registry.state(BlockStateId(i as u32)), Some(&state));
        assert_eq!(registry.resolve(&state)?, BlockStateId(i as u32));
        let descriptor = BlockState {
            name: state.name.clone(),
            properties: Default::default(),
        };
        assert_eq!(
            registry.resolve(&descriptor)?,
            registry.default_state(&state.name).unwrap()
        );
    }
    fs::write(
        base.join("rust/block-registry.nbt"),
        nbt::write_named(&registry_root, Limits::default())?,
    )?;
    let mut section = BlockSection::uniform(BlockStateId(0), &registry)?;
    for i in 0..4096 {
        section.set(i, BlockStateId(((i * 7) % registry.len()) as u32))?;
    }
    assert_eq!(section.bits(), 12);
    let root = NamedTag {
        name: "".into(),
        tag: section.to_nbt()?,
    };
    fs::write(
        base.join("rust/runtime-section.nbt"),
        nbt::write_named(&root, Limits::default())?,
    )?;
    let loaded = BlockSection::from_nbt(&root.tag, &registry)?;
    for i in 0..4096 {
        assert_eq!(loaded.get(i)?, section.get(i)?);
    }
    assert!(section.set(4096, BlockStateId(0)).is_err());
    assert!(section.set(0, BlockStateId(registry.len() as u32)).is_err());
    // Rotate every registered state through one slot to exercise slot reuse.
    for i in 0..registry.len() {
        section.set(0, BlockStateId(i as u32))?;
    }
    for i in 1..4096 {
        assert_eq!(
            section.get(i)?,
            BlockStateId(((i * 7) % registry.len()) as u32)
        );
    }
    for i in 0..4096 {
        section.set(i, BlockStateId(0))?;
    }
    assert_eq!(section.bits(), 0);
    // Mixed replacement, sharing, and compaction must preserve every block.
    let mut expected = vec![BlockStateId(0); 4096];
    let mut random = 0x31415926u64;
    for step in 0..100_000 {
        random = random.wrapping_mul(6364136223846793005).wrapping_add(1);
        let index = (random >> 32) as usize % 4096;
        let state = BlockStateId(if step % 3 == 0 {
            0
        } else {
            (random >> 16) as u32 % registry.len() as u32
        });
        assert_eq!(section.set(index, state)?, expected[index]);
        expected[index] = state;
        if step % 8192 == 0 {
            section.compact()?;
            for (index, state) in expected.iter().enumerate() {
                assert_eq!(section.get(index)?, *state);
            }
        }
    }
    let mutated = NamedTag {
        name: "".into(),
        tag: section.to_nbt()?,
    };
    fs::write(
        base.join("rust/runtime-mutated.nbt"),
        nbt::write_named(&mutated, Limits::default())?,
    )?;
    let loaded = BlockSection::from_nbt(&mutated.tag, &registry)?;
    for (index, state) in expected.iter().enumerate() {
        assert_eq!(loaded.get(index)?, *state);
        section.set(index, BlockStateId(0))?;
    }
    section.compact()?;
    assert_eq!(section.bits(), 0);
    assert!(section.to_nbt()?.get("data").is_none());
    println!(
        "Rust registry: {} states and default-state resolutions checked",
        registry.len()
    );
    Ok(())
}
