use super::*;
fn saved(palette: &PalettedContainer) -> (usize, Option<usize>) {
    let Tag::Compound(map) = palette.to_nbt() else {
        unreachable!()
    };
    let entries = match map.get(&key("palette")) {
        Some(Tag::List(entries)) => entries.len(),
        other => panic!("palette is {other:?}"),
    };
    let words = match map.get(&key("data")) {
        Some(Tag::LongArray(words)) => Some(words.len()),
        None => None,
        other => panic!("data is {other:?}"),
    };
    (entries, words)
}

/// `PalettedContainer.pack` re-encodes the cells into a fresh palette, so a
/// state no cell holds any more is not written and the bit count comes from
/// the states that remain: `getConfigurationForPaletteSize(17)` is
/// `SimpleBitStorage(5, 4096)` (342 longs), `(16)` is `(4, 4096)` (256).
#[test]
fn a_state_no_cell_holds_is_not_saved() {
    let mut palette = PalettedContainer::uniform(Tag::Int(0), 4096, 4).unwrap();
    for index in 0..16 {
        palette.set(index, Tag::Int(index as i32 + 1)).unwrap();
    }
    assert_eq!((palette.palette_len(), palette.bits()), (17, 5));
    palette.set(15, Tag::Int(0)).unwrap();

    let Tag::Compound(map) = palette.to_nbt() else {
        unreachable!()
    };
    let Some(Tag::LongArray(words)) = map.get(&key("data")) else {
        panic!("no data")
    };
    assert_eq!(words.len(), 256);
    let Some(Tag::List(entries)) = map.get(&key("palette")) else {
        panic!("no palette")
    };
    // First use over the cells: state 1 at index 0 … state 15 at index 14,
    // then air everywhere from index 15 on.
    let first_use = (1..=15)
        .map(Tag::Int)
        .chain([Tag::Int(0)])
        .collect::<Vec<_>>();
    assert_eq!(entries, &first_use);
    let loaded = PalettedContainer::from_nbt(&palette.to_nbt(), 4096, 4).unwrap();
    for index in 0..4096 {
        let want = if index <= 14 {
            Tag::Int(index as i32 + 1)
        } else {
            Tag::Int(0)
        };
        assert_eq!(loaded.get(index).unwrap(), &want);
    }
}

/// `pack` writes no `data` once one state is all that is left: a palette of
/// one is configuration bits 0, `ZeroBitStorage`, and empty `values`.
#[test]
fn a_section_that_became_uniform_saves_no_data() {
    let mut palette = PalettedContainer::uniform(Tag::Int(0), 4096, 4).unwrap();
    palette.set(4000, Tag::Int(9)).unwrap();
    assert_eq!(saved(&palette), (2, Some(256)));
    palette.set(4000, Tag::Int(0)).unwrap();
    assert_eq!(saved(&palette), (1, None));
}

#[test]
fn palette_grows_across_word_widths_and_compacts_to_uniform() {
    let mut palette = PalettedContainer::uniform(Tag::Int(0), 4096, 4).unwrap();
    for i in 0..4096 {
        palette.set(i, Tag::Int(i as i32)).unwrap();
    }
    assert_eq!(palette.bits(), 12);
    let mut loaded = PalettedContainer::from_nbt(&palette.to_nbt(), 4096, 4).unwrap();
    for i in 0..4096 {
        assert_eq!(loaded.get(i).unwrap(), &Tag::Int(i as i32));
        loaded.set(i, Tag::Int(7)).unwrap();
    }
    loaded.compact().unwrap();
    assert_eq!(loaded.bits(), 0);
    assert_eq!(loaded.palette_len(), 1);
    assert!(loaded.to_nbt().get("data").is_none());
    assert_eq!(loaded.get(4095).unwrap(), &Tag::Int(7));
}
#[test]
fn invalid_palette_data_fails_before_access() {
    let mut tag = Tag::compound([("palette", Tag::List(vec![Tag::Int(0), Tag::Int(1)]))]);
    assert!(PalettedContainer::from_nbt(&tag, 4096, 4).is_err());
    let Tag::Compound(map) = &mut tag else {
        unreachable!()
    };
    map.insert(key("data"), Tag::LongArray(vec![-1; 256]));
    assert!(PalettedContainer::from_nbt(&tag, 4096, 4).is_err());
}

#[test]
fn packing_matches_the_structural_tag_encoder_after_loading_duplicate_slots_and_editing() {
    fn old_pack(section: &PalettedContainer) -> Tag {
        let mut map = section.extra.clone();
        let mut seen = HashMap::new();
        let mut palette = Vec::new();
        let mut ids = Vec::with_capacity(section.entries);
        for cell in 0..section.entries {
            let tag = section.get(cell).unwrap();
            ids.push(*seen.entry(tag).or_insert_with(|| {
                palette.push(tag.clone());
                palette.len() as u32 - 1
            }));
        }
        let bits = bits_for_palette(palette.len(), section.min_bits);
        map.insert(key("palette"), Tag::List(palette));
        if bits > 0 {
            let mut storage = PackedStorage::new(bits, section.entries).unwrap();
            for (cell, id) in ids.into_iter().enumerate() {
                storage.set(cell, id).unwrap();
            }
            map.insert(
                key("data"),
                Tag::LongArray(storage.words().iter().map(|word| *word as i64).collect()),
            );
        }
        Tag::Compound(map)
    }
    fn state(id: usize) -> Tag {
        Tag::compound([
            ("Name", Tag::String(key(&format!("test:block_{}", id / 4)))),
            (
                "Properties",
                Tag::compound([("variation", Tag::Int((id % 4) as i32))]),
            ),
        ])
    }
    let encode = |tag| {
        horizon_core::nbt::write_named(
            &NamedTag { name: key(""), tag },
            horizon_core::nbt::Limits::saved(),
        )
        .unwrap()
    };
    let mut rng = 0x32d3_7495_b3b1_7805u64;
    let mut next = |limit: usize| {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        (rng % limit as u64) as usize
    };
    for distinct in [1, 4, 16, 17, 32, 64] {
        // An alias of the first entry appears in cell order before its original
        // slot. Saving must still emit one entry at the first *cell* using it.
        let mut tags: Vec<_> = (0..distinct).map(state).collect();
        tags.extend([state(0), state(distinct - 1), state(distinct + 1)]);
        let mut storage = PackedStorage::new(bits_for_palette(tags.len(), 4), 4096).unwrap();
        storage.set(0, distinct as u32).unwrap();
        for i in 1..4096 {
            storage.set(i, next(distinct + 2) as u32).unwrap();
        }
        let input = Tag::compound([
            ("palette", Tag::List(tags)),
            (
                "data",
                Tag::LongArray(storage.words().iter().map(|&w| w as i64).collect()),
            ),
            ("test:untouched", Tag::List(vec![Tag::Long(99)])),
        ]);
        let mut section = PalettedContainer::from_nbt(&input, 4096, 4).unwrap();
        assert_eq!(
            encode(section.to_nbt()),
            encode(old_pack(&section)),
            "loaded P={distinct}"
        );
        for round in 0..4 {
            for _ in 0..47 {
                section.set(next(4096), state(next(distinct + 5))).unwrap();
            }
            assert_eq!(
                encode(section.to_nbt()),
                encode(old_pack(&section)),
                "edited P={distinct} round {round}"
            );
        }
    }
}

/// The pre-optimization `get`, kept to compare against in the benchmark.
fn baseline_get(section: &BlockSection<'_>, index: usize) -> io::Result<BlockStateId> {
    if index >= 4096 {
        return Err(invalid("section index out of bounds"));
    }
    let id = section
        .storage
        .as_ref()
        .map_or(0, |s| s.get(index).unwrap());
    Ok(section.palette[id as usize])
}
/// The pre-optimization `set`, kept to compare against in the benchmark.
fn baseline_set(
    section: &mut BlockSection<'_>,
    index: usize,
    state: BlockStateId,
) -> io::Result<BlockStateId> {
    let old = baseline_get(section, index)?;
    if state.0 as usize >= section.registry.len() {
        return Err(invalid("state ID outside registry"));
    }
    if old == state {
        return Ok(old);
    }
    // Dead slots stay in `palette` but are on the free list: the old map
    // removed them, so only live entries count for either lookup.
    let old_id = section
        .palette
        .iter()
        .enumerate()
        .find(|&(id, &s)| s == old && section.counts[id] != 0)
        .map(|(id, _)| id as u32)
        .unwrap();
    let known = section
        .palette
        .iter()
        .enumerate()
        .find(|&(id, &s)| s == state && section.counts[id] != 0)
        .map(|(id, _)| id as u32);
    if section.counts[old_id as usize] == 1 && known.is_none() {
        section.palette[old_id as usize] = state;
        return Ok(old);
    }
    let id = if let Some(id) = known {
        id
    } else if let Some(id) = section.free.pop() {
        section.palette[id as usize] = state;
        id
    } else {
        let id = section.palette.len() as u32;
        let next_bits = bits_for_palette(section.palette.len() + 1, 4);
        if next_bits != section.bits() {
            let mut storage =
                PackedStorage::new(next_bits, 4096).map_err(|e| invalid(e.to_string()))?;
            if let Some(old) = &section.storage {
                for i in 0..4096 {
                    storage.set(i, old.get(i).unwrap()).unwrap();
                }
            }
            section.storage = Some(storage);
        }
        section.palette.push(state);
        section.counts.push(0);
        id
    };
    section
        .storage
        .as_mut()
        .unwrap()
        .set(index, id)
        .map_err(|e| invalid(e.to_string()))?;
    section.counts[old_id as usize] -= 1;
    section.counts[id as usize] += 1;
    if section.counts[old_id as usize] == 0 {
        section.free.push(old_id);
    }
    if section.counts[id as usize] == 4096 {
        section.palette = vec![state];
        section.counts = vec![4096];
        section.free.clear();
        section.storage = None;
    }
    Ok(old)
}

/// `main`'s `fill_run`, unchanged except that it writes through `set`
/// (checked against `baseline_set` above) instead of that file's own copy.
fn reference_fill_run(
    section: &mut BlockSection<'_>,
    x: i32,
    y_start: i32,
    z: i32,
    len: i32,
    state: BlockStateId,
    air: BlockStateId,
) -> io::Result<u16> {
    if !(0..16).contains(&x)
        || !(0..16).contains(&z)
        || len <= 0
        || y_start < 0
        || y_start + len > 16
    {
        return Err(invalid("section fill outside its bounds"));
    }
    let index = |y: i32| ((y as usize) << 8) | ((z as usize) << 4) | x as usize;
    let top = y_start + len - 1;
    let mut added = 0u16;
    let old = section.set(index(top), state)?;
    if old == air {
        added += 1;
    }
    let Some(id) = id_in(&section.palette, &section.counts, &mut section.last, state) else {
        return Ok(added);
    };
    let Some(storage) = section.storage.as_mut() else {
        return Ok(added);
    };
    for y in (y_start..top).rev() {
        let i = index(y);
        let old_id = storage.read(i);
        let old_state = section.palette[old_id as usize];
        if old_state == state {
            continue;
        }
        storage.write(i, id);
        section.counts[old_id as usize] -= 1;
        section.counts[id as usize] += 1;
        if old_state == air {
            added += 1;
        }
        if section.counts[old_id as usize] == 0 {
            section.free.push(old_id);
            if section.last == Some((old_state, old_id)) {
                section.last = None;
            }
        }
    }
    if section.counts[id as usize] == 4096 {
        section.palette = vec![state];
        section.last = Some((state, 0));
        section.counts = vec![4096];
        section.free.clear();
        section.storage = None;
    }
    Ok(added)
}

/// Palette order, counts, free slots, width and every packed word.
fn assert_fields(section: &BlockSection<'_>, reference: &BlockSection<'_>, what: &str) {
    assert_eq!(section.palette, reference.palette, "{what}: palette");
    assert_eq!(section.counts, reference.counts, "{what}: counts");
    assert_eq!(section.free, reference.free, "{what}: free slots");
    assert_eq!(section.bits(), reference.bits(), "{what}: bits");
    assert_eq!(
        section.storage.as_ref().map(|s| s.words()),
        reference.storage.as_ref().map(|s| s.words()),
        "{what}: packed words"
    );
}

/// Every block, as `get` reads it.
fn assert_blocks(section: &BlockSection<'_>, reference: &BlockSection<'_>, what: &str) {
    for index in 0..4096 {
        assert_eq!(
            section.state(index),
            reference.state(index),
            "{what}: block {index}"
        );
    }
}

/// `set` and `fill_run` against `main`'s implementations, over every width
/// the palette can pick: 1 state, 2..4, 5..16, 17..32, 33..64, 65..128,
/// 129..256 and the global ids past that (4 bits to 12). Runs cover whole
/// sections, single blocks and every length in between.
#[test]
fn fill_run_and_set_match_reference_across_palette_widths() {
    let registry = benchmark_registry(4100);
    let air = BlockStateId(0);
    for &(distinct, seed) in &[
        (1u64, 11u64),
        (2, 12),
        (3, 13),
        (4, 14),
        (5, 15),
        (16, 16),
        (17, 17),
        (31, 18),
        (32, 19),
        (33, 20),
        (64, 21),
        (65, 22),
        (128, 23),
        (129, 24),
        (200, 25),
        (256, 26),
        (257, 27),
        (400, 28),
        (4096, 29),
    ] {
        let mut section = BlockSection::uniform(air, &registry).unwrap();
        let mut reference = BlockSection::uniform(air, &registry).unwrap();
        let mut rng = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for step in 0..600u32 {
            let state = BlockStateId((next() % distinct) as u32);
            if step % 3 == 0 {
                let index = (next() % 4096) as usize;
                let what = format!("distinct {distinct} step {step} set {index}");
                assert_eq!(
                    section.set(index, state).unwrap(),
                    reference.set(index, state).unwrap(),
                    "{what}"
                );
                assert_fields(&section, &reference, &what);
                continue;
            }
            let x = (next() % 16) as i32;
            let z = (next() % 16) as i32;
            let y_start = (next() % 16) as i32;
            let len = 1 + (next() % (16 - y_start as u64)) as i32;
            // Every section width, and fills that land on the section top
            // and bottom.
            let len = if step % 7 == 0 { 16 - y_start } else { len };
            let what = format!("distinct {distinct} step {step} run {x},{y_start},{z}+{len}");
            assert_eq!(
                section.fill_run(x, y_start, z, len, state, air).unwrap(),
                reference_fill_run(&mut reference, x, y_start, z, len, state, air).unwrap(),
                "{what}: air blocks added"
            );
            assert_fields(&section, &reference, &what);
            if step % 16 == 0 {
                assert_blocks(&section, &reference, &what);
            }
        }
        // One state over the whole section collapses it to a uniform one.
        let state = BlockStateId((next() % distinct) as u32);
        for x in 0..16 {
            for z in 0..16 {
                assert_eq!(
                    section.fill_run(x, 0, z, 16, state, air).unwrap(),
                    reference_fill_run(&mut reference, x, 0, z, 16, state, air).unwrap()
                );
            }
        }
        assert_fields(&section, &reference, "collapsed");
        assert_blocks(&section, &reference, "collapsed");
        assert_eq!(section.bits(), 0, "a uniform section has no words");
    }
}

fn benchmark_registry(states: usize) -> BlockRegistry {
    let list: Vec<Tag> = (0..states)
        .map(|i| Tag::compound([("id", Tag::String(format!("minecraft:b{i}").as_str().into()))]))
        .collect();
    let defaults: std::collections::BTreeMap<_, _> = (0..states)
        .map(|i| {
            (
                JavaString::from(format!("minecraft:b{i}").as_str()),
                Tag::Int(i as i32),
            )
        })
        .collect();
    let tag = Tag::compound([
        ("DataVersion", Tag::Int(crate::DATA_VERSION)),
        ("states", Tag::List(list)),
        ("defaults", Tag::Compound(defaults)),
    ]);
    BlockRegistry::from_nbt(&tag).unwrap()
}

/// `fill_states` reads what `state` reads at every index, from a uniform
/// section (no storage) through every palette width.
#[test]
fn fill_states_matches_state() {
    let registry = benchmark_registry(64);
    for &(distinct, seed) in &[(1u64, 1u64), (2, 2), (16, 3), (17, 4), (64, 6)] {
        let mut section = BlockSection::uniform(BlockStateId(0), &registry).unwrap();
        let mut rng = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut out = [0u32; 4096];
        for round in 0..4 {
            section.fill_states(&mut out);
            for (index, &value) in out.iter().enumerate() {
                assert_eq!(value, section.state(index).0, "round {round} index {index}");
            }
            for _ in 0..2_000 {
                rng ^= rng << 13;
                rng ^= rng >> 7;
                rng ^= rng << 17;
                let index = (rng % 4096) as usize;
                let value = ((rng >> 20) % distinct) as u32;
                section.set(index, BlockStateId(value)).unwrap();
            }
        }
    }
}

/// The optimized `set`/`get` must agree with the pre-optimization ones on
/// every field, across growth, collapse and reclaimed palette slots.
#[test]
fn set_matches_baseline_under_random_edits() {
    let registry = benchmark_registry(64);
    for &(distinct, seed) in &[(1u64, 1u64), (2, 2), (16, 3), (17, 4), (24, 5), (64, 6)] {
        let mut base = BlockSection::uniform(BlockStateId(0), &registry).unwrap();
        let mut fast = BlockSection::uniform(BlockStateId(0), &registry).unwrap();
        let mut rng = seed.wrapping_mul(0x9e37_79b9_7f4a_7c15) | 1;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for step in 0..50_000u32 {
            if step % 5_000 == 4_999 {
                let value = (next() % distinct) as u32;
                for index in 0..4096 {
                    assert_eq!(
                        baseline_set(&mut base, index, BlockStateId(value)).unwrap(),
                        fast.set(index, BlockStateId(value)).unwrap()
                    );
                }
            } else {
                let index = (next() % 4096) as usize;
                let value = (next() % distinct) as u32;
                assert_eq!(
                    baseline_set(&mut base, index, BlockStateId(value)).unwrap(),
                    fast.set(index, BlockStateId(value)).unwrap()
                );
            }
            assert_eq!(base.bits(), fast.bits());
        }
        assert_eq!(base.palette, fast.palette);
        assert_eq!(base.counts, fast.counts);
        assert_eq!(base.free, fast.free);
        for index in 0..4096 {
            assert_eq!(
                baseline_get(&base, index).unwrap(),
                fast.get(index).unwrap(),
                "distinct {distinct} index {index}"
            );
        }
        assert_eq!(base.to_nbt().unwrap(), fast.to_nbt().unwrap());
    }
}

/// A lookup must not revive a freed palette slot: the next new state pops
/// that slot off the free list and would silently overwrite it.
#[test]
fn lookups_do_not_revive_freed_palette_slots() {
    let registry = benchmark_registry(8);
    let mut section = BlockSection::uniform(BlockStateId(0), &registry).unwrap();
    for (index, state) in [(3, 3), (0, 1), (0, 0), (1, 1), (2, 2)] {
        section.set(index, BlockStateId(state)).unwrap();
    }
    assert_eq!(section.get(1).unwrap(), BlockStateId(1));
    assert_eq!(section.get(2).unwrap(), BlockStateId(2));
    assert_eq!(section.get(3).unwrap(), BlockStateId(3));
}

/// `set` and `get` against the pre-optimization implementations, on the same
/// edit stream. The result is checked before timing; 51 measured rounds
/// alternate order and report the median and best nanosecond-per-operation.
#[test]
#[ignore = "microbenchmark; run with --ignored --nocapture"]
fn benchmark_section_get_set() {
    use std::{hint::black_box, time::Instant};
    const OPS: usize = 50_000;
    const ROUNDS: u32 = 51;
    let registry = benchmark_registry(32);
    let mut rng = 0x1234_5678_9abc_def1u64;
    let mut next = || {
        rng ^= rng << 13;
        rng ^= rng >> 7;
        rng ^= rng << 17;
        rng
    };
    let ops: Vec<(usize, u32)> = (0..OPS)
        .map(|_| ((next() % 4096) as usize, (next() % 24) as u32))
        .collect();
    let make = || BlockSection::uniform(BlockStateId(0), &registry).unwrap();

    let mut base = make();
    let mut fast = make();
    for &(index, value) in &ops {
        assert_eq!(
            baseline_set(&mut base, index, BlockStateId(value)).unwrap(),
            fast.set(index, BlockStateId(value)).unwrap()
        );
    }
    assert_eq!(base.bits(), fast.bits());
    assert_eq!(base.palette, fast.palette);
    assert_eq!(base.counts, fast.counts);
    assert_eq!(base.free, fast.free);
    assert_eq!(base.to_nbt().unwrap(), fast.to_nbt().unwrap());
    for i in 0..4096 {
        assert_eq!(baseline_get(&base, i).unwrap(), fast.get(i).unwrap());
    }

    let mut times = [Vec::new(), Vec::new()];
    for round in 0..ROUNDS {
        for optimized in [round % 2 == 0, round % 2 != 0] {
            let mut section = make();
            let started = Instant::now();
            for &(index, value) in &ops {
                if optimized {
                    section
                        .set(black_box(index), black_box(BlockStateId(value)))
                        .unwrap();
                } else {
                    baseline_set(
                        &mut section,
                        black_box(index),
                        black_box(BlockStateId(value)),
                    )
                    .unwrap();
                }
            }
            black_box(&section);
            if round > 0 {
                times[usize::from(optimized)]
                    .push(started.elapsed().as_secs_f64() * 1e9 / OPS as f64);
            }
        }
    }
    for values in &mut times {
        values.sort_by(f64::total_cmp);
    }
    let median = |v: &[f64]| v[v.len() / 2];
    println!(
        "section set median: baseline {:.3} ns/op optimized {:.3} ns/op speedup {:.3}x",
        median(&times[0]),
        median(&times[1]),
        median(&times[0]) / median(&times[1])
    );
    println!(
        "section set best:   baseline {:.3} ns/op optimized {:.3} ns/op speedup {:.3}x",
        times[0][0],
        times[1][0],
        times[0][0] / times[1][0]
    );

    let mut get_times = [Vec::new(), Vec::new()];
    for round in 0..ROUNDS {
        for optimized in [round % 2 == 0, round % 2 != 0] {
            let started = Instant::now();
            let mut acc = 0u32;
            for &(index, _) in &ops {
                let state = if optimized {
                    fast.get(black_box(index)).unwrap()
                } else {
                    baseline_get(&base, black_box(index)).unwrap()
                };
                acc ^= state.0;
            }
            black_box(acc);
            if round > 0 {
                get_times[usize::from(optimized)]
                    .push(started.elapsed().as_secs_f64() * 1e9 / OPS as f64);
            }
        }
    }
    for values in &mut get_times {
        values.sort_by(f64::total_cmp);
    }
    println!(
        "section get median: baseline {:.3} ns/op optimized {:.3} ns/op speedup {:.3}x",
        median(&get_times[0]),
        median(&get_times[1]),
        median(&get_times[0]) / median(&get_times[1])
    );
    println!(
        "section get best:   baseline {:.3} ns/op optimized {:.3} ns/op speedup {:.3}x",
        get_times[0][0],
        get_times[1][0],
        get_times[0][0] / get_times[1][0]
    );
}

/// `BlockEntity.getPosFromTag`: a saved position in another chunk keeps its
/// offset inside the chunk but moves into the chunk that saved it; a
/// position already there is kept as it is.
#[test]
fn a_block_entity_saved_in_another_chunk_moves_into_this_one() {
    assert_eq!(super::block_entity_pos((37, 70, -3), [0, 0]), (5, 70, 13));
    assert_eq!(super::block_entity_pos((-1, -60, 19), [0, 0]), (15, -60, 3));
    assert_eq!(
        super::block_entity_pos((5, 70, 13), [0, 0]),
        (5, 70, 13),
        "already in the chunk"
    );
    // Another chunk: its own corner is chunk * 16.
    assert_eq!(
        super::block_entity_pos((37, 70, -3), [-2, 5]),
        (-27, 70, 93)
    );
}
