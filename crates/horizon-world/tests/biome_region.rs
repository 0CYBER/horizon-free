use horizon_core::position::ChunkPos;
use horizon_world::worldgen::biome_chunk::{BiomeChunk, BiomeRegion, BiomeRegionError};
#[test]
fn resident_biomes_track_replacement_fills_and_reject_ambiguous_chunks() {
    let p = ChunkPos::new(-1, 1);
    let mut chunk = BiomeChunk::new(p, -64, 32, 7).unwrap();
    chunk
        .fill(-64, -33, |[x, y, z]| {
            Ok::<_, ()>(x.wrapping_add(y).wrapping_add(z).rem_euclid(3))
        })
        .unwrap();
    assert!(matches!(
        BiomeRegion::new(vec![chunk.clone(), chunk.clone()], 0),
        Err(BiomeRegionError::InvalidChunks)
    ));
    assert!(matches!(
        BiomeRegion::new(
            vec![
                chunk.clone(),
                BiomeChunk::new(ChunkPos::new(0, 0), 0, 32, 0).unwrap()
            ],
            0
        ),
        Err(BiomeRegionError::InvalidChunks)
    ));
    let mut region = BiomeRegion::new(vec![chunk], 0).unwrap();
    assert_eq!(region.present_in_chunk(p).unwrap(), vec![0, 1, 2]);
    region
        .chunk_mut(p)
        .unwrap()
        .fill(-64, -33, |_| Ok::<_, ()>(9))
        .unwrap();
    assert_eq!(region.present_in_chunk(p).unwrap(), vec![9]);
    assert_eq!(region.noise_biome([-1, i32::MIN, 4]).unwrap(), &9);
    assert_eq!(region.noise_biome([-1, i32::MAX, 4]).unwrap(), &9);
    assert!(matches!(
        region.noise_biome([0, 0, 4]),
        Err(BiomeRegionError::MissingChunk(_))
    ));
}
