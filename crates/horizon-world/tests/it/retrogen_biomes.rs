use horizon_core::position::ChunkPos;
use horizon_world::{
    retrogen::RetrogenBiomeResolver,
    worldgen::biome_chunk::{BiomeChunk, BiomeFillError},
};
use std::cell::RefCell;

#[test]
fn candidates_precede_old_columns_and_errors_short_circuit() {
    let events = RefCell::new(Vec::new());
    let mut resolver = RetrogenBiomeResolver::new(true, |p| {
        events.borrow_mut().push(p);
        if p[1] == 99 {
            Err("candidate")
        } else {
            Ok("minecraft:plains")
        }
    });
    let old = |p| {
        events.borrow_mut().push(p);
        Ok("horizon:old")
    };
    assert_eq!(
        resolver.sample([-3, -15, 8], |v| Some(*v), old),
        Ok("horizon:old")
    );
    assert_eq!(*events.borrow(), [[-3, -15, 8], [-3, 0, 8]]);
    events.borrow_mut().clear();
    assert_eq!(
        resolver.sample([1, 99, 2], |v| Some(*v), old),
        Err("candidate")
    );
    assert_eq!(*events.borrow(), [[1, 99, 2]]);
    assert_eq!(
        resolver.sample([1, -1, 2], |v| Some(*v), |_| Err("old")),
        Err("old")
    );
    for name in [
        "minecraft:lush_caves",
        "minecraft:dripstone_caves",
        "minecraft:deep_dark",
    ] {
        let mut resolver = RetrogenBiomeResolver::new(true, |_| Ok::<_, ()>(name));
        assert_eq!(
            resolver.sample(
                [0, -1, 0],
                |v| Some(*v),
                |_| panic!("retained candidate must not read old column")
            ),
            Ok(name)
        );
        assert_eq!(
            resolver.sample([0, -1, 0], |_| None, |_| Ok("old")),
            Ok("old")
        );
    }
    let mut normal = RetrogenBiomeResolver::new(false, |_| Ok::<_, ()>("candidate"));
    assert_eq!(
        normal.sample(
            [0, 0, 0],
            |_| panic!("normal resolver must not inspect keys"),
            |_| panic!("normal resolver must not read old columns")
        ),
        Ok("candidate")
    );
}

#[test]
fn upgrade_fill_preserves_columns_and_section_atomicity() {
    let mut chunk = BiomeChunk::new(ChunkPos::new(-1, 2), -64, 128, String::new()).unwrap();
    chunk
        .fill(-64, 63, |[x, y, z]| Ok::<_, ()>(format!("old:{x}:{y}:{z}")))
        .unwrap();
    let original = chunk.clone();
    let mut calls = 0;
    let result = chunk.fill_with_retrogen(
        -64,
        -1,
        true,
        |_| {
            calls += 1;
            if calls == 66 {
                Err("failure")
            } else {
                Ok("minecraft:plains".to_owned())
            }
        },
        |name| Some(name),
    );
    assert_eq!(result, Err(BiomeFillError::Resolve("failure")));
    for y in -16..-12 {
        for x in -4..0 {
            for z in 8..12 {
                assert_eq!(
                    chunk.noise_biome([x, y, z]),
                    original.noise_biome([x, 0, z])
                );
            }
        }
    }
    for section in 1..8 {
        assert_eq!(chunk.section(section), original.section(section));
    }
    chunk
        .fill_with_retrogen(
            -64,
            -1,
            true,
            |_| Ok::<_, ()>("minecraft:lush_caves".to_owned()),
            |name| Some(name),
        )
        .unwrap();
    assert_eq!(chunk.noise_biome([-4, -16, 8]), "minecraft:lush_caves");
    for section in 4..8 {
        assert_eq!(chunk.section(section), original.section(section));
    }
}
