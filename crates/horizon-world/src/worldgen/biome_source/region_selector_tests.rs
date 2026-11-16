use super::{
    BiomeSourceError, ResolvedBiomeSource,
    region_selector::{RegionError, RegionSelector},
    regions::{LayoutRegion, RegionLayout, RegionPoint, SelectorParams},
};
use crate::worldgen::{biome::MultiNoiseBiomeSource, climate::ClimateSampler};
use horizon_core::{
    density::{ConstantSampler, DensitySampler, GradientSampler, SamplerContext, TilingMode},
    geometry::Axis,
    position::BlockPos,
    random::{PositionalRandom, RandomSource, XoroshiroPositionalRandom, XoroshiroRandom},
};
use std::sync::Arc;

fn point(biome: &str, temperature: f32, humidity: f32) -> RegionPoint {
    let mut intervals = [(0., 0.); 6];
    intervals[0] = (temperature, temperature);
    intervals[1] = (humidity, humidity);
    RegionPoint {
        biome: biome.to_owned(),
        intervals,
        offset: 0.,
    }
}
fn vanilla() -> MultiNoiseBiomeSource {
    let points = [
        point("test:cold_dry", -0.8, -0.8),
        point("test:cold_wet", -0.8, 0.8),
        point("test:hot_dry", 0.8, -0.8),
        point("test:hot_wet", 0.8, 0.8),
    ];
    MultiNoiseBiomeSource::new(
        points
            .iter()
            .map(|p| (p.parameter_point().unwrap(), p.biome.clone()))
            .collect(),
    )
    .unwrap()
}
/// Temperature runs along X and humidity along Z, so every vanilla biome shows up nearby.
fn climate() -> ClimateSampler {
    ClimateSampler::new(std::array::from_fn(|i| -> Arc<dyn DensitySampler> {
        match i {
            0 => Arc::new(GradientSampler::new(
                Axis::X,
                TilingMode::Repeat,
                0,
                400,
                -1.,
                1.,
            )),
            1 => Arc::new(GradientSampler::new(
                Axis::Z,
                TilingMode::MirroredRepeat,
                0,
                600,
                -1.,
                1.,
            )),
            _ => Arc::new(ConstantSampler(0.)),
        }
    }))
}
fn region(id: &str, weight: u32) -> LayoutRegion {
    LayoutRegion {
        owner: "test".to_owned(),
        id: format!("test:{id}"),
        weight,
        points: vec![
            point(&format!("test:{id}_cold"), -0.8, 0.),
            point(&format!("test:{id}_hot"), 0.8, 0.),
        ],
        substituted: false,
    }
}
fn layout(vanilla_weight: u32, regions: Vec<LayoutRegion>) -> RegionLayout {
    RegionLayout {
        selector: SelectorParams {
            cell: 8,
            vanilla_weight,
            ..SelectorParams::current()
        },
        regions,
    }
}
fn root(seed: i64) -> XoroshiroPositionalRandom {
    XoroshiroRandom::new(seed).fork_positional()
}
/// Quart columns across cell (8) and chunk (4) edges on both sides of 0, and far out.
fn columns() -> Vec<[i32; 2]> {
    let axis: Vec<i32> = (-37..=37).chain([-100_001, -100_000, 99_999]).collect();
    axis.iter()
        .flat_map(|&x| axis.iter().map(move |&z| [x, z]))
        .collect()
}
fn owners(selector: &RegionSelector) -> Vec<usize> {
    columns()
        .into_iter()
        .map(|[x, z]| selector.owner(x, z))
        .collect()
}
fn biomes(source: &ResolvedBiomeSource, climate: &ClimateSampler) -> Vec<String> {
    let erosion = ConstantSampler(0.);
    let mut resolver = source.resolver(climate, &erosion);
    let mut context = SamplerContext::default();
    columns()
        .into_iter()
        .flat_map(|[x, z]| [[x, -3, z], [x, 5, z]])
        .map(|quart| resolver.sample(&mut context, quart).unwrap().to_owned())
        .collect()
}

#[test]
fn without_regions_the_source_is_vanilla_unchanged() {
    let climate = climate();
    let plain = biomes(&ResolvedBiomeSource::MultiNoise(vanilla()), &climate);
    let empty = layout(10, Vec::new());
    for saved in [None, Some(&empty)] {
        let source = ResolvedBiomeSource::MultiNoise(vanilla())
            .with_regions(saved, &root(1))
            .unwrap();
        assert!(matches!(source, ResolvedBiomeSource::MultiNoise(_)));
        assert_eq!(biomes(&source, &climate), plain);
    }
    // Regions that never win a cell leave every quart vanilla's, scalar and prepared.
    let unpicked = ResolvedBiomeSource::MultiNoise(vanilla())
        .with_regions(
            Some(&layout(i32::MAX as u32 - 1, vec![region("never", 1)])),
            &root(1),
        )
        .unwrap();
    assert!(matches!(unpicked, ResolvedBiomeSource::Regions(_)));
    assert_eq!(biomes(&unpicked, &climate), plain);
    let erosion = ConstantSampler(0.);
    let plain_source = ResolvedBiomeSource::MultiNoise(vanilla());
    let mut expected = plain_source.resolver(&climate, &erosion);
    let mut resolver = unpicked.resolver(&climate, &erosion);
    let mut context = SamplerContext::default();
    let volume = resolver
        .prepare_volume(&mut context, [-20, -2, -20], [40, 4, 40])
        .unwrap();
    for x in -20..20 {
        for z in -20..20 {
            let quart = [x, 1, z];
            assert_eq!(
                resolver
                    .sample_prepared(&mut context, volume.as_ref(), quart)
                    .unwrap(),
                expected
                    .sample_prepared(&mut context, volume.as_ref(), quart)
                    .unwrap(),
                "{quart:?}"
            );
        }
    }
}

#[test]
fn owner_is_the_nearest_jitter_point_of_the_spec_stream() {
    let layout = layout(10, vec![region("a", 1), region("b", 3)]);
    let root = root(42);
    let selector = RegionSelector::new(&layout, &root).unwrap();
    // The ruling's stream, drawn here independently of the selector.
    let stream = root.from_hash_of("horizon:biome_region").fork_positional();
    let ends = [10i64, 11, 14];
    let seed = |cx: i64, cz: i64| {
        let mut random = stream.at(BlockPos::new(cx as i32, 0, cz as i32));
        let jx = random.next_int_bound(8).unwrap() as i64;
        let jz = random.next_int_bound(8).unwrap() as i64;
        let pick = random.next_int_bound(14).unwrap() as i64;
        (
            cx * 8 + jx,
            cz * 8 + jz,
            ends.iter().position(|&end| pick < end).unwrap(),
        )
    };
    let mut seen = [false; 3];
    for [x, z] in columns() {
        let (qx, qz) = (i64::from(x), i64::from(z));
        let (cx, cz) = (qx.div_euclid(8), qz.div_euclid(8));
        let mut best = (i64::MAX, 0);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (px, pz, slot) = seed(cx + dx, cz + dz);
                let distance = (px - qx).pow(2) + (pz - qz).pow(2);
                if distance < best.0 {
                    best = (distance, slot);
                }
            }
        }
        assert_eq!(selector.owner(x, z), best.1, "{x} {z}");
        seen[best.1] = true;
        assert_eq!(selector.cell_slot(cx as i32, cz as i32), seed(cx, cz).2);
    }
    assert_eq!(seen, [true; 3], "the grid must reach every slot");
}

#[test]
fn same_seed_and_layout_give_the_same_owners() {
    let layout = layout(10, vec![region("a", 1), region("b", 3)]);
    let first = owners(&RegionSelector::new(&layout, &root(7)).unwrap());
    assert_eq!(
        owners(&RegionSelector::new(&layout, &root(7)).unwrap()),
        first
    );
    assert_ne!(
        owners(&RegionSelector::new(&layout, &root(8)).unwrap()),
        first
    );
}

#[test]
fn selection_reads_only_the_saved_weights_in_order() {
    let saved = layout(10, vec![region("a", 1), region("b", 3)]);
    let first = owners(&RegionSelector::new(&saved, &root(3)).unwrap());
    let rebuilt = saved.clone();
    assert_eq!(
        owners(&RegionSelector::new(&rebuilt, &root(3)).unwrap()),
        first
    );
    // Other ids, owners, points and substitution: same slots, same weights, same map.
    let mut renamed = layout(10, vec![region("z", 1), region("y", 3)]);
    renamed.regions[0].owner = "other".to_owned();
    renamed.regions[0].points.pop();
    renamed.regions[1].substituted = true;
    assert_eq!(
        owners(&RegionSelector::new(&renamed, &root(3)).unwrap()),
        first
    );
    // Swapping the saved order swaps the weights between slots: the map moves.
    let swapped = layout(10, vec![region("b", 3), region("a", 1)]);
    assert_ne!(
        owners(&RegionSelector::new(&swapped, &root(3)).unwrap()),
        first
    );
}

#[test]
fn cells_split_by_weight() {
    let layout = layout(10, vec![region("a", 1), region("b", 3)]);
    let selector = RegionSelector::new(&layout, &root(99)).unwrap();
    let mut counts = [0u32; 3];
    for cx in -100..100 {
        for cz in -50..50 {
            counts[selector.cell_slot(cx, cz)] += 1;
        }
    }
    for (count, weight) in counts.into_iter().zip([10., 1., 3.]) {
        let share = f64::from(count) / 20_000.;
        assert!(
            (share - weight / 14.).abs() <= 0.02,
            "{counts:?}: share {share} for weight {weight}"
        );
    }
}

#[test]
fn a_substituted_region_resolves_through_vanilla_and_moves_no_border() {
    let climate = climate();
    let erosion = ConstantSampler(0.);
    let original = layout(10, vec![region("a", 4), region("b", 4)]);
    let mut substituted = original.clone();
    substituted.regions[0].substituted = true;
    let selector = RegionSelector::new(&original, &root(11)).unwrap();
    assert_eq!(
        owners(&RegionSelector::new(&substituted, &root(11)).unwrap()),
        owners(&selector)
    );
    let source = |layout: &RegionLayout| {
        ResolvedBiomeSource::MultiNoise(vanilla())
            .with_regions(Some(layout), &root(11))
            .unwrap()
    };
    let (before, after) = (source(&original), source(&substituted));
    let plain = ResolvedBiomeSource::MultiNoise(vanilla());
    let mut before = before.resolver(&climate, &erosion);
    let mut after = after.resolver(&climate, &erosion);
    let mut plain = plain.resolver(&climate, &erosion);
    let mut context = SamplerContext::default();
    let mut slots = [0; 3];
    for [x, z] in columns() {
        let quart = [x, 0, z];
        let slot = selector.owner(x, z);
        slots[slot] += 1;
        let biome = after.sample(&mut context, quart).unwrap();
        match slot {
            1 => {
                assert_eq!(biome, plain.sample(&mut context, quart).unwrap());
                assert!(biome.starts_with("test:") && !biome.starts_with("test:a_"));
            }
            _ => assert_eq!(biome, before.sample(&mut context, quart).unwrap()),
        }
        if slot == 2 {
            assert!(biome.starts_with("test:b_"), "{biome}");
        }
    }
    assert!(slots.iter().all(|&n| n > 0), "{slots:?}");
}

#[test]
fn layouts_this_build_cannot_run_are_refused() {
    let refused = |layout: RegionLayout| RegionSelector::new(&layout, &root(0)).err();
    let mut version = layout(10, vec![region("a", 1)]);
    version.selector.version = 2;
    assert_eq!(refused(version.clone()), Some(RegionError::Version(2)));
    for weight in [0, 1001, u32::MAX] {
        assert_eq!(
            refused(layout(10, vec![region("a", weight)])),
            Some(RegionError::Weight)
        );
    }
    let mut widest = layout(10, vec![region("a", 1)]);
    widest.selector.cell = 1 << 30;
    assert_eq!(refused(widest.clone()), None);
    widest.selector.cell += 1;
    assert_eq!(refused(widest), Some(RegionError::Cell((1 << 30) + 1)));
    let mut cell = layout(10, vec![region("a", 1)]);
    cell.selector.cell = 0;
    assert_eq!(refused(cell), Some(RegionError::Cell(0)));
    assert_eq!(
        refused(layout(0, vec![region("a", 0)])),
        Some(RegionError::Weight)
    );
    assert_eq!(
        refused(layout(u32::MAX, vec![region("a", 1)])),
        Some(RegionError::Weight)
    );
    assert_eq!(
        refused(layout(i32::MAX as u32, vec![region("a", 1)])),
        Some(RegionError::Weight)
    );
    assert_eq!(
        refused(layout(i32::MAX as u32 - 1, vec![region("a", 1)])),
        None
    );
    assert!(matches!(
        ResolvedBiomeSource::MultiNoise(vanilla()).with_regions(Some(&version), &root(0)),
        Err(BiomeSourceError::Regions(RegionError::Version(2)))
    ));
}

#[test]
fn building_the_selector_leaves_the_generator_random_where_it_was() {
    let mut random = XoroshiroRandom::new(5);
    let positional = random.fork_positional();
    let mut untouched = XoroshiroRandom::new(5);
    let untouched_positional = untouched.fork_positional();
    RegionSelector::new(&layout(10, vec![region("a", 1)]), &positional).unwrap();
    assert_eq!(positional, untouched_positional);
    assert_eq!(random.next_long(), untouched.next_long());
}

#[test]
fn selector_version_one_has_literal_seeded_draws_and_owners() {
    // Independently calculated with Python hashlib MD5 and unsigned xoroshiro arithmetic.
    let root = root(42);
    let stream = root.from_hash_of("horizon:biome_region").fork_positional();
    for ((x, z), expected) in [
        ((0, 0), [4, 5, 7]),
        ((-1, 0), [2, 2, 13]),
        ((2, -3), [6, 1, 12]),
    ] {
        let mut random = stream.at(BlockPos::new(x, 0, z));
        assert_eq!(
            [
                random.next_int_bound(8).unwrap(),
                random.next_int_bound(8).unwrap(),
                random.next_int_bound(14).unwrap()
            ],
            expected
        );
    }
    let selector =
        RegionSelector::new(&layout(10, vec![region("a", 1), region("b", 3)]), &root).unwrap();
    for ((x, z), expected) in [
        ((-9, -1), 2),
        ((-4, 0), 0),
        ((0, 0), 0),
        ((3, 17), 2),
        ((31, -12), 0),
    ] {
        assert_eq!(selector.owner(x, z), expected, "quart {x}, {z}");
    }
}
