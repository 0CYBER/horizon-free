use super::*;
const BLOCKS: Strategy = Strategy::blocks(16);

#[test]
fn pack_nbt_caches_lazily_and_follows_unchanged_clones() {
    let mut palette = PalettedContainer::new(BLOCKS, 0);
    palette.set(1, 1);
    let budget = Arc::new(SavedPaletteBudget::new(1 << 20));
    let name = |id| Tag::Int(id as i32);

    // Lazy: nothing cached until the first save.
    assert_eq!(budget.used(), 0);
    assert!(palette.saved_nbt.get().is_none());
    let base_heap = palette.heap_bytes();

    let first = palette.pack_nbt(Some(&budget), name);
    let cached_charge = budget.used();
    assert!(cached_charge > 0);
    assert_eq!(palette.heap_bytes(), base_heap + cached_charge);
    let old = Arc::downgrade(palette.saved_nbt.get().unwrap());

    // An unchanged clone shares the entry without charging the budget again.
    let unchanged = palette.clone();
    assert_eq!(unchanged.pack_nbt(Some(&budget), name), first);
    assert!(old.ptr_eq(&Arc::downgrade(unchanged.saved_nbt.get().unwrap())));
    assert_eq!(budget.used(), cached_charge);

    // A no-op write keeps the cache; a real mutation invalidates it.
    palette.set(1, 1);
    assert!(palette.saved_nbt.get().is_some());
    palette.set(1, 2);
    assert!(palette.saved_nbt.get().is_none());
    assert_eq!(palette.heap_bytes(), base_heap);

    let changed = palette.pack_nbt(Some(&budget), name);
    assert_ne!(first, changed);
    assert_eq!(changed, palette.pack_nbt(None, name));
    // Both old (held by `unchanged`) and new entries are charged until `unchanged` drops.
    assert!(budget.used() > cached_charge);
    drop(unchanged);
    assert!(old.upgrade().is_none());
    let after_drop_unchanged = budget.used();
    assert!(after_drop_unchanged > 0);

    // Cache hit re-resolves registry names at save time.
    assert_eq!(
        palette.pack_nbt(Some(&budget), |id| Tag::Long(i64::from(id) + 100)),
        palette.pack_nbt(None, |id| Tag::Long(i64::from(id) + 100))
    );

    drop(palette);
    assert_eq!(budget.used(), 0);
}

#[test]
fn heap_bytes_counts_a_shared_cached_palette_once_across_clones() {
    let mut palette = PalettedContainer::new(BLOCKS, 0);
    palette.set(1, 1);
    let base_heap = palette.heap_bytes();
    let budget = Arc::new(SavedPaletteBudget::new(1 << 20));
    palette.pack_nbt(Some(&budget), |id| Tag::Int(id as i32));
    let charge = budget.used();
    let clones = [palette.clone(), palette.clone(), palette.clone()];
    let cached: usize = std::iter::once(&palette)
        .chain(&clones)
        .map(|holder| holder.heap_bytes() - holder.storage_heap_bytes())
        .sum();
    // Four holders: the entry counts once, not four times (less the rounding).
    assert!(
        cached <= charge && cached + 4 > charge,
        "{cached} of {charge}"
    );
    drop(clones);
    assert_eq!(palette.heap_bytes(), base_heap + charge);
}

#[test]
fn pack_nbt_uniform_and_collapsed_sections_bypass_cache() {
    let budget = Arc::new(SavedPaletteBudget::new(1 << 20));
    let name = |id| Tag::Int(id as i32);

    let single = PalettedContainer::new(BLOCKS, 5);
    let mut collapsed = PalettedContainer::new(BLOCKS, 5);
    collapsed.set(100, 9);
    collapsed.set(100, 5);
    let global = global_uniform(3000);

    for container in [&single, &collapsed, &global] {
        assert_eq!(
            container.pack_nbt(Some(&budget), name),
            container.pack_nbt(None, name)
        );
        assert!(container.saved_nbt.get().is_none());
        assert_eq!(budget.used(), 0);
    }
}

#[test]
fn pack_nbt_budget_cap_and_retry_after_drop() {
    let name = |id| Tag::Int(id as i32);
    let mut words = vec![0; storage_len(4, BLOCKS.entries())];
    words[0] = 0x10;
    let load = || PalettedContainer::from_packed(BLOCKS, vec![3, 4], Some(&words)).unwrap();
    let first = load();
    let second = load();
    let budget = Arc::new(SavedPaletteBudget::new(3000));
    assert_eq!(budget.used(), 0);
    assert!(first.saved_nbt.get().is_none());

    let expected = first.pack_nbt(Some(&budget), name);
    assert!(first.saved_nbt.get().is_some());
    assert!(budget.used() <= budget.limit());

    // Over cap: packs identically without admitting into the cache.
    assert_eq!(second.pack_nbt(Some(&budget), name), expected);
    assert!(second.saved_nbt.get().is_none());

    // Dropping the first owner frees its budget so the second can be admitted.
    drop(first);
    assert_eq!(budget.used(), 0);
    assert_eq!(second.pack_nbt(Some(&budget), name), expected);
    assert!(second.saved_nbt.get().is_some());
    drop(second);
    assert_eq!(budget.used(), 0);
}

#[test]
fn pack_nbt_matches_pack_across_cases_and_isolates_budgets() {
    let budget_a = Arc::new(SavedPaletteBudget::new(1 << 20));
    let budget_b = Arc::new(SavedPaletteBudget::new(1 << 20));
    let name = |id| Tag::Int(id as i32);
    for (label, container) in pack_cases() {
        let uncached = container.pack_nbt(None, name);
        let first_a = container.pack_nbt(Some(&budget_a), name);
        let hit_a = container.pack_nbt(Some(&budget_a), name);
        let other_b = container.pack_nbt(Some(&budget_b), name);
        assert_eq!(first_a, uncached, "{label}: first cached save");
        assert_eq!(hit_a, uncached, "{label}: repeated cached save");
        assert_eq!(other_b, uncached, "{label}: isolated budget save");
        assert_eq!(budget_b.used(), 0, "{label}: second budget unaffected");
    }
    assert_eq!(budget_a.used(), 0);
}
const BIOMES: Strategy = Strategy::biomes(7);

#[test]
fn locate_matches_division_for_every_width() {
    for bits in 1..=32u8 {
        let per = 64 / usize::from(bits);
        for i in 0..4096 {
            let expected = (i / per, ((i % per) * usize::from(bits)) as u32);
            assert_eq!(locate(bits, i), expected, "{bits} bits, entry {i}");
        }
    }
}
fn rng(state: &mut u64) -> u64 {
    *state ^= *state << 13;
    *state ^= *state >> 7;
    *state ^= *state << 17;
    *state
}
#[test]
fn runs_read_the_stored_values() {
    let mut state = 0x9E37_79B9_7F4A_7C15;
    // One value, a list palette, and the global palette.
    for distinct in [1u64, 5, 3000] {
        let values: Vec<u32> = (0..4096)
            .map(|_| (rng(&mut state) % distinct) as u32)
            .collect();
        let container = PalettedContainer::from_values(BLOCKS, &values);
        for start in 0..4095 {
            let mut pair = [u32::MAX; 2];
            container.get_run(start, &mut pair);
            assert_eq!(pair, values[start..start + 2]);
        }
        for _ in 0..200 {
            let start = (rng(&mut state) % 4096) as usize;
            let len = (rng(&mut state) % 20) as usize;
            let len = len.min(4096 - start);
            let mut run = vec![u32::MAX; len];
            container.get_run(start, &mut run);
            assert_eq!(
                run,
                values[start..start + len],
                "{distinct} values, {start}+{len}"
            );
        }
    }
}
#[test]
fn four_bit_fast_paths_match_get() {
    let mut state = 0x517C_C1B7_2722_0A95;
    // Both minimal (2 values) and full (16 values) 4-bit palettes, plus 5-bit and single.
    for (distinct, expected_bits) in [(1usize, 0u8), (2, 4), (16, 4), (20, 5)] {
        let values: Vec<u32> = (0..4096)
            .map(|i| {
                let slot = if i < distinct {
                    i
                } else {
                    (rng(&mut state) as usize) % distinct
                };
                100 + slot as u32 * 7
            })
            .collect();
        let container = PalettedContainer::from_values(BLOCKS, &values);
        assert_eq!(
            container.bits(),
            expected_bits,
            "{distinct} distinct values"
        );

        let mut unpacked = vec![u32::MAX; 4096];
        container.unpack(&mut unpacked);
        assert_eq!(unpacked, values, "unpack ({distinct} values)");

        // Every 16-aligned 16-entry row hits the 4-bit `get_run` fast path.
        for row in 0..256 {
            let start = row * 16;
            let mut buf = [u32::MAX; 16];
            container.get_run(start, &mut buf);
            assert_eq!(
                buf,
                values[start..start + 16],
                "aligned 16-run at {start} ({distinct} values)"
            );
        }

        // Unaligned 16-entry runs and non-16 lengths at aligned starts stay exact.
        for (start, len) in [
            (1, 16),
            (15, 16),
            (17, 16),
            (4079, 16),
            (0, 0),
            (0, 1),
            (0, 15),
            (16, 17),
            (32, 32),
        ] {
            let mut buf = vec![u32::MAX; len];
            container.get_run(start, &mut buf);
            assert_eq!(
                buf,
                values[start..start + len],
                "run {start}+{len} ({distinct} values)"
            );
        }
    }
}
#[test]
fn containers_stored_alike_are_the_same() {
    let values: Vec<u32> = (0..4096).map(|i| (i % 7) as u32).collect();
    let a = PalettedContainer::from_values(BLOCKS, &values);
    assert!(a.same_storage(&a.clone()));
    let mut b = a.clone();
    b.set(100, 3);
    assert!(!a.same_storage(&b));
    // the same values stored another way are not found alike (the caller unpacks those)
    let mut c = PalettedContainer::new(BLOCKS, 0);
    for (i, &v) in values.iter().enumerate() {
        c.set(i, v);
    }
    assert!(PalettedContainer::new(BLOCKS, 5).same_storage(&PalettedContainer::new(BLOCKS, 5)));
    assert!(!PalettedContainer::new(BLOCKS, 5).same_storage(&PalettedContainer::new(BLOCKS, 6)));
    let mut out = vec![0; 4096];
    c.unpack(&mut out);
    assert_eq!(out, values);
}
#[test]
fn grows_through_vanilla_widths() {
    let mut c = PalettedContainer::new(BLOCKS, 0);
    assert_eq!((c.bits(), c.palette_len()), (0, 1));
    let mut expected = Vec::new();
    for n in 1..=300u32 {
        c.set(n as usize, n);
        expected.push(match n + 1 {
            2..=16 => 4,
            17..=32 => 5,
            33..=64 => 6,
            65..=128 => 7,
            129..=256 => 8,
            _ => 16,
        });
        assert_eq!(c.bits(), *expected.last().unwrap(), "after {n} values");
    }
    assert!(c.is_global());
    for n in 1..=300u32 {
        assert_eq!(c.get(n as usize), n);
    }
    let mut b = PalettedContainer::new(BIOMES, 3);
    for n in 1..=9u32 {
        b.set(n as usize, n + 3);
        let want = match n + 1 {
            2 => 1,
            3..=4 => 2,
            5..=8 => 3,
            _ => 7,
        };
        assert_eq!(b.bits(), want, "after {n} biomes");
    }
}
#[test]
fn matches_a_plain_array_under_random_edits() {
    for (strategy, distinct) in [(BLOCKS, 700u64), (BLOCKS, 12), (BIOMES, 9), (BIOMES, 3)] {
        let entries = strategy.entries();
        let mut state = 0x1234_5678_9ABC_DEF1u64;
        let mut c = PalettedContainer::new(strategy, 5);
        let mut plain = vec![5u32; entries];
        for _ in 0..20_000 {
            let r = rng(&mut state);
            let i = (r % entries as u64) as usize;
            let v = ((r >> 20) % distinct) as u32 * 7;
            assert_eq!(c.set(i, v), plain[i]);
            plain[i] = v;
        }
        let mut out = vec![0; entries];
        c.unpack(&mut out);
        assert_eq!(out, plain);
        for (i, &v) in plain.iter().enumerate() {
            assert_eq!(c.get(i), v);
        }
        let packed = c.pack();
        let loaded = PalettedContainer::from_packed(
            strategy,
            packed.palette.clone(),
            packed.data.as_deref(),
        )
        .unwrap();
        assert_eq!(loaded, c);
        assert_eq!(loaded.pack(), packed);
        let again = PalettedContainer::from_values(strategy, &plain);
        assert_eq!(again, c);
        assert_eq!(again.pack(), packed);
        let (bits, palette, words) = c.to_wire();
        let wire = PalettedContainer::from_wire(strategy, bits, palette, words.to_vec()).unwrap();
        assert_eq!(wire, c);
        for wanted in [0, 7, 14, 35, 7 * 699, 5] {
            assert_eq!(
                c.has(|v| v == wanted || v == wanted + 7),
                plain.iter().any(|&v| v == wanted || v == wanted + 7),
                "has {wanted} in {distinct} values"
            );
        }
    }
}
#[test]
fn has_matches_a_scan_at_every_width() {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    for (strategy, distinct) in [
        (BLOCKS, 2u32),
        (BLOCKS, 3),
        (BLOCKS, 17),
        (BLOCKS, 33),
        (BLOCKS, 65),
        (BLOCKS, 129),
        (BIOMES, 2),
        (BIOMES, 3),
        (BIOMES, 5),
    ] {
        let entries = strategy.entries();
        let mut c = PalettedContainer::new(strategy, 0);
        let mut plain = vec![0u32; entries];
        for i in 0..entries {
            let v = (rng(&mut state) % u64::from(distinct)) as u32;
            c.set(i, v);
            plain[i] = v;
        }
        // Take some values away again (stale entries), the last index too.
        for gone in [0, 1, distinct - 1] {
            for i in 0..entries {
                if plain[i] == gone {
                    c.set(i, distinct);
                    plain[i] = distinct;
                }
            }
            for m in [3, 5, 7] {
                let hit = |v: u32| v % m == gone % m;
                assert_eq!(c.has(hit), plain.iter().any(|&v| hit(v)));
            }
            for wanted in 0..=distinct {
                for other in [wanted, wanted + 1, wanted + 2, wanted + 3, wanted + 4] {
                    let hit = |v: u32| v == wanted || v == other;
                    assert_eq!(
                        c.has(hit),
                        plain.iter().any(|&v| hit(v)),
                        "{distinct} values, {wanted}/{other}, without {gone}"
                    );
                }
            }
        }
    }
}
#[test]
fn has_ignores_stale_palette_entries() {
    let mut c = PalettedContainer::new(BLOCKS, 0);
    assert!(c.has(|v| v == 0) && !c.has(|v| v == 3));
    c.set(4095, 3);
    assert!(c.has(|v| v == 3));
    c.set(4095, 0);
    assert!(c.maybe_has(|v| v == 3) && !c.has(|v| v == 3));
    for v in 1..=300 {
        c.set(v as usize, v);
    }
    assert!(c.is_global() && c.has(|v| v == 300));
    c.set(300, 0);
    assert!(c.maybe_has(|v| v == 300) && !c.has(|v| v == 300));
}
#[test]
fn uniform_detection_ignores_stale_palette_entries() {
    let mut c = PalettedContainer::new(BLOCKS, 0);
    c.set(10, 3);
    assert!(!c.is_uniform(0));
    c.set(10, 0);
    assert!(c.palette_len() == 2 && c.is_uniform(0) && c.uniform() == Some(0));
    assert!(!c.is_uniform(3));
    for i in 0..4096 {
        c.set(i, 3);
    }
    assert_eq!(c.uniform(), Some(3));
}
#[test]
fn rejects_malformed_wire_data() {
    assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![1, 2], vec![0; 255]).is_err());
    assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![1, 2], vec![0x3333; 256]).is_err());
    assert!(PalettedContainer::from_wire(BLOCKS, 4, vec![], vec![0; 256]).is_err());
    assert!(PalettedContainer::from_wire(BLOCKS, 0, vec![1, 2], vec![]).is_err());
    assert!(PalettedContainer::from_wire(BIOMES, 2, vec![1, 2, 3, 4, 5], vec![0; 4]).is_err());
    assert!(PalettedContainer::from_wire(BLOCKS, 0, vec![9], vec![]).is_ok());
}
/// Every word of every width against the entry-at-a-time scan, over
/// valid data, a bad entry anywhere in a word, garbage only in a word's
/// padding fields, and the last word's padding set to all ones: the same
/// verdict, and the first offending entry named as before.
#[test]
#[ignore = "daily"]
fn index_check_matches_an_entry_by_entry_scan() {
    fn scan(strategy: Strategy, bits: u8, len: usize, words: &[u64]) -> Result<(), u32> {
        let per = 64 / usize::from(bits);
        for i in 0..strategy.entries() {
            if i / per >= words.len() {
                break;
            }
            let index = read(words, bits, i);
            if index as usize >= len {
                return Err(index);
            }
        }
        Ok(())
    }
    fn compare(strategy: Strategy, bits: u8, len: usize, words: &[u64], what: &str) {
        let container = PalettedContainer {
            strategy,
            bits,
            palette: Palette::List((0..len as u32).collect()),
            words: words.to_vec(),
            identity: Identity::default(),
            saved_nbt: OnceLock::new(),
        };
        match (container.check_indices(), scan(strategy, bits, len, words)) {
            (Ok(()), Ok(())) => {}
            (Err(PaletteError::BadIndex { index, palette }), Err(first)) => {
                assert_eq!((index, palette), (first, len), "{what}");
            }
            (a, b) => panic!("{what}: {a:?} vs {b:?}"),
        }
    }
    let mut state = 0x2545_F491_4F6C_DD1Du64;
    for strategy in [BLOCKS, BIOMES] {
        let entries = strategy.entries();
        let max_len = match strategy.kind {
            Kind::Blocks => 256,
            Kind::Biomes => 8,
        };
        for len in 2..=max_len {
            let bits = match strategy.config(ceil_log2(len)) {
                Config::List(bits) => bits,
                _ => unreachable!("{len} entries in a single-value palette"),
            };
            let per = 64 / usize::from(bits);
            let words_len = storage_len(bits, entries);
            let valid = pack_words(bits, entries, (0..entries).map(|i| (i % len) as u32));
            let name = format!("{len} entries, {bits} bits");
            compare(strategy, bits, len, &valid, &name);
            if len == 1 << bits {
                // No field can hold an index past the palette.
                continue;
            }
            let top = mask(bits) as usize;
            // A bad entry in the first, a middle and the last field of a
            // word, and in the container's last entry.
            for at in [0, per / 2, per - 1, entries / 2, entries - 1] {
                let mut bad = valid.clone();
                let index = (len + (rng(&mut state) % 7) as usize).min(top);
                write(&mut bad, bits, at, index as u32);
                compare(strategy, bits, len, &bad, &format!("{name}, bad at {at}"));
            }
            // Garbage in the last word's padding fields only.
            let rest = entries - (words_len - 1) * per;
            if rest < per {
                let mut padded = valid.clone();
                padded[words_len - 1] |= u64::MAX << (rest * usize::from(bits));
                compare(strategy, bits, len, &padded, &format!("{name}, padding"));
            }
            // A whole random word, every field drawn from `0..=len`.
            for _ in 0..64 {
                let mut words = valid.clone();
                let word = (rng(&mut state) % words_len as u64) as usize;
                words[word] = 0;
                for field in 0..per {
                    let index = (rng(&mut state) % (len as u64 + 1)).min(top as u64);
                    words[word] |= index << (field * usize::from(bits));
                }
                compare(
                    strategy,
                    bits,
                    len,
                    &words,
                    &format!("{name}, random word {word}"),
                );
            }
        }
    }
}
#[test]
fn saved_validation_matches_entrywise_reference() {
    // Keep the previous loader as an independent entry-at-a-time reference:
    // errors, resident palette order and unused storage bits must all agree.
    fn reference(
        strategy: Strategy,
        palette: Vec<u32>,
        data: Option<&[u64]>,
    ) -> Result<PalettedContainer, PaletteError> {
        if palette.is_empty() {
            return Err(PaletteError::BadPalette { len: 0 });
        }
        let config = strategy.config(ceil_log2(palette.len()));
        let entries = strategy.entries();
        let bits = match config {
            Config::Single => return Ok(PalettedContainer::new(strategy, palette[0])),
            Config::List(bits) => bits,
            Config::Global => ceil_log2(palette.len()),
        };
        let expected = storage_len(bits, entries);
        let data = data.ok_or(PaletteError::BadStorage {
            expected,
            actual: 0,
        })?;
        if data.len() != expected {
            return Err(PaletteError::BadStorage {
                expected,
                actual: data.len(),
            });
        }
        let mut ids = Vec::with_capacity(entries);
        for i in 0..entries {
            let index = read(data, bits, i);
            if index as usize >= palette.len() {
                return Err(PaletteError::BadIndex {
                    index,
                    palette: palette.len(),
                });
            }
            ids.push(index);
        }
        Ok(match config {
            Config::List(bits) => PalettedContainer {
                strategy,
                bits,
                palette: Palette::List(palette),
                words: data.to_vec(),
                identity: Identity::default(),
                saved_nbt: OnceLock::new(),
            },
            _ => PalettedContainer::build(strategy, palette, &ids),
        })
    }
    fn compare(strategy: Strategy, palette: &[u32], data: Option<&[u64]>) {
        let expected = reference(strategy, palette.to_vec(), data);
        let actual = PalettedContainer::from_packed(strategy, palette.to_vec(), data);
        match (actual, expected) {
            (Ok(a), Ok(b)) => {
                assert_eq!(a.strategy, b.strategy);
                assert_eq!(a.bits, b.bits);
                assert_eq!(a.palette, b.palette);
                assert_eq!(a.words, b.words);
            }
            (Err(a), Err(b)) => assert_eq!(a, b),
            (a, b) => panic!("loader disagreement: {a:?} vs {b:?}"),
        }
    }
    for strategy in [BLOCKS, BIOMES] {
        for len in [
            0, 1, 2, 3, 4, 5, 8, 9, 15, 16, 17, 31, 32, 33, 63, 64, 65, 127, 128, 129, 255, 256,
            257, 511, 512, 513, 1023, 1024, 1025, 2047, 2048, 2049, 4096, 4097,
        ] {
            let palette: Vec<u32> = (0..len).map(|i| (i as u32 * 7) % 127).collect();
            compare(strategy, &palette, None);
            compare(strategy, &palette, Some(&[]));
            if len <= 1 {
                compare(strategy, &palette, Some(&[u64::MAX]));
                continue;
            }
            let bits = match strategy.config(ceil_log2(len)) {
                Config::List(bits) => bits,
                _ => ceil_log2(len),
            };
            let entries = strategy.entries();
            let mut words = pack_words(bits, entries, (0..entries).map(|i| (i % len) as u32));
            compare(strategy, &palette, Some(&words));
            compare(strategy, &palette, Some(&words[..words.len() - 1]));
            // Nonzero padding in every word and in the final partial word
            // is legal and must be preserved by resident list palettes.
            let per = 64 / usize::from(bits);
            for (word, positions) in words.iter_mut().zip((0..entries).step_by(per)) {
                let used = (entries - positions).min(per) * usize::from(bits);
                if used < 64 {
                    *word |= u64::MAX << used;
                }
            }
            compare(strategy, &palette, Some(&words));
            if len < 1usize << bits {
                for i in [0, per - 1, per, entries / 2, entries - 1] {
                    let mut invalid = words.clone();
                    write(&mut invalid, bits, i, len as u32);
                    // A second invalid entry verifies first-error ordering.
                    write(&mut invalid, bits, entries - 1, mask(bits) as u32);
                    compare(strategy, &palette, Some(&invalid));
                }
            }
        }
    }
}
#[test]
fn division_magic_is_exact() {
    for bits in 1..=32u8 {
        let per = 64 / usize::from(bits);
        for i in 0..4096usize {
            assert_eq!(locate(bits, i).0, i / per, "bits {bits} index {i}");
        }
    }
}

/// The pre-optimization `unpack`, kept to compare against in the benchmark.
fn baseline_unpack(c: &PalettedContainer, out: &mut [u32]) {
    match &c.palette {
        Palette::Single(value) => out.fill(*value),
        palette => {
            let bits = usize::from(c.bits);
            let per = 64 / bits;
            let m = mask(c.bits);
            for (chunk, &word) in out.chunks_mut(per).zip(&c.words) {
                for (slot, cell) in chunk.iter_mut().enumerate() {
                    let id = ((word >> (slot * bits)) & m) as u32;
                    *cell = match palette {
                        Palette::List(list) => list[id as usize],
                        _ => id,
                    };
                }
            }
        }
    }
}

/// `unpack` against the pre-optimization implementation, for a list palette
/// and the global palette. The result is checked before timing; 25 measured
/// rounds alternate order and report the median and best microsecond-per-call.
#[test]
#[ignore = "microbenchmark; run with --ignored --nocapture"]
fn benchmark_unpack() {
    use std::{hint::black_box, time::Instant};
    let values: Vec<u32> = (0..4096).map(|i| (i % 12) as u32).collect();
    let list = PalettedContainer::from_values(BLOCKS, &values);
    let global = PalettedContainer::from_values(
        BLOCKS,
        &(0..4096).map(|i| (i * 3) as u32).collect::<Vec<_>>(),
    );
    for (name, container) in [("list", &list), ("global", &global)] {
        let mut expected = vec![0u32; 4096];
        baseline_unpack(container, &mut expected);
        let mut out = vec![0u32; 4096];
        container.unpack(&mut out);
        assert_eq!(expected, out, "unpack {name} differs");
        let mut times = [Vec::new(), Vec::new()];
        for round in 0..25 {
            for optimized in [round % 2 == 0, round % 2 != 0] {
                let started = Instant::now();
                for _ in 0..2000 {
                    if optimized {
                        container.unpack(black_box(&mut out));
                    } else {
                        baseline_unpack(black_box(container), black_box(&mut out));
                    }
                }
                if round > 0 {
                    times[usize::from(optimized)]
                        .push(started.elapsed().as_secs_f64() * 1e6 / 2000.0);
                }
            }
        }
        for values in &mut times {
            values.sort_by(f64::total_cmp);
        }
        let median = |v: &[f64]| v[v.len() / 2];
        println!(
            "unpack {name} median: baseline {:.3} us optimized {:.3} us speedup {:.3}x",
            median(&times[0]),
            median(&times[1]),
            median(&times[0]) / median(&times[1])
        );
        println!(
            "unpack {name} best:   baseline {:.3} us optimized {:.3} us speedup {:.3}x",
            times[0][0],
            times[1][0],
            times[0][0] / times[1][0]
        );
    }
}

/// The pre-optimization `palette_ids` (scan, then a SipHash map), kept to
/// compare against.
fn baseline_palette_ids(values: &[u32]) -> (Vec<u32>, Vec<u32>) {
    let mut palette: Vec<u32> = Vec::new();
    let mut small = true;
    let mut lookup: std::collections::HashMap<u32, u32> = std::collections::HashMap::new();
    let mut ids = Vec::with_capacity(values.len());
    for &value in values {
        let id = if small {
            match palette.iter().position(|&v| v == value) {
                Some(id) => id as u32,
                None => {
                    palette.push(value);
                    if palette.len() > 16 {
                        small = false;
                        lookup.extend(palette.iter().enumerate().map(|(i, &v)| (v, i as u32)));
                    }
                    palette.len() as u32 - 1
                }
            }
        } else {
            *lookup.entry(value).or_insert_with(|| {
                palette.push(value);
                palette.len() as u32 - 1
            })
        };
        ids.push(id);
    }
    (palette, ids)
}
/// Sections like the ones saved: stone, ores and caves under dirt, grass
/// and air; a mixed surface (plants, leaves, logs) in short runs; a
/// global palette; ids past the table; and biomes.
fn palette_cases() -> Vec<(&'static str, Vec<u32>)> {
    let mut state = 0x9E37_79B9_7F4A_7C15u64;
    let mut terrain = Vec::with_capacity(4096);
    for i in 0..4096u32 {
        let y = i >> 8;
        let r = rng(&mut state);
        terrain.push(match y {
            0..=8 if r.is_multiple_of(40) => 2000 + (r >> 8) as u32 % 6,
            0..=8 if r.is_multiple_of(13) => 0,
            0..=8 => 1,
            9..=10 => 10,
            11 => 9,
            _ => 0,
        });
    }
    let mut surface = Vec::with_capacity(4096);
    while surface.len() < 4096 {
        let r = rng(&mut state);
        let run = 1 + (r % 8) as usize;
        let v = 100 + ((r >> 8) % 24) as u32 * 37;
        surface.extend(std::iter::repeat_n(v, run.min(4096 - surface.len())));
    }
    let global: Vec<u32> = (0..4096)
        .map(|_| (rng(&mut state) % 600) as u32 * 41)
        .collect();
    let large: Vec<u32> = (0..4096)
        .map(|i| {
            if i % 3 == 0 {
                70_000 + (rng(&mut state) % 40) as u32
            } else {
                i as u32 % 20
            }
        })
        .collect();
    let biomes: Vec<u32> = (0..64).map(|i| [3, 3, 3, 7, 7, 12][i % 6]).collect();
    vec![
        ("terrain", terrain),
        ("surface", surface),
        ("global", global),
        ("large ids", large),
        ("biomes", biomes),
        ("single", vec![5; 4096]),
        ("seventeen", (0..4096).map(|i| (i % 17) as u32).collect()),
    ]
}
#[test]
fn palette_ids_match_first_occurrence_order() {
    for (name, values) in palette_cases() {
        assert_eq!(
            palette_ids(&values),
            baseline_palette_ids(&values),
            "{name}"
        );
        // The table is left clean for the next section.
        assert_eq!(
            palette_ids(&values),
            baseline_palette_ids(&values),
            "{name} again"
        );
    }
    DENSE_IDS.with(|t| assert!(t.borrow().iter().all(|&slot| slot == 0)));
}
/// `palette_ids` against the pre-optimization implementation over the
/// cases above. 25 measured rounds alternate order and report the median
/// microseconds per call.
#[test]
#[ignore = "microbenchmark; run with --ignored --nocapture"]
fn benchmark_palette_ids() {
    use std::{hint::black_box, time::Instant};
    for (name, values) in palette_cases() {
        let mut times = [Vec::new(), Vec::new()];
        for round in 0..25 {
            for optimized in [round % 2 == 0, round % 2 != 0] {
                let started = Instant::now();
                for _ in 0..500 {
                    if optimized {
                        black_box(palette_ids(black_box(&values)));
                    } else {
                        black_box(baseline_palette_ids(black_box(&values)));
                    }
                }
                if round > 0 {
                    times[usize::from(optimized)]
                        .push(started.elapsed().as_secs_f64() * 1e6 / 500.0);
                }
            }
        }
        for t in &mut times {
            t.sort_by(f64::total_cmp);
        }
        let (base, new) = (times[0][times[0].len() / 2], times[1][times[1].len() / 2]);
        println!(
            "palette_ids {name}: baseline {base:.2} us, new {new:.2} us, {:.2}x",
            base / new
        );
    }
}
/// `pack` as it was before a section of one value was answered from its
/// words: every cell unpacked, then the distinct values found again.
fn baseline_pack(c: &PalettedContainer) -> Packed {
    let entries = c.strategy.entries();
    let mut values = vec![0u32; entries];
    c.unpack(&mut values);
    let (palette, ids) = palette_ids(&values);
    let bits = match c.strategy.config(ceil_log2(palette.len())) {
        Config::Single => 0,
        Config::List(bits) => bits,
        Config::Global => ceil_log2(palette.len()),
    };
    let data = (bits != 0).then(|| pack_words(bits, entries, ids.iter().copied()));
    Packed {
        palette,
        bits,
        data,
    }
}
/// A container holding `value` in every cell while stored in the global
/// palette's width, which `set` keeps once a section has used it.
fn global_uniform(value: u32) -> PalettedContainer {
    let bits = BLOCKS.global_bits;
    let width = usize::from(bits);
    let per = 64 / width;
    let pattern = (0..per).fold(0u64, |word, slot| word | u64::from(value) << (slot * width));
    PalettedContainer {
        strategy: BLOCKS,
        bits,
        palette: Palette::Global,
        words: vec![pattern; BLOCKS.entries().div_ceil(per)],
        identity: Identity::default(),
        saved_nbt: OnceLock::new(),
    }
}
/// Sections saved from a column: one value (a single palette, and a list
/// palette whose other entry died), mixed terrain, the global palette
/// holding one value, and biomes.
fn pack_cases() -> Vec<(&'static str, PalettedContainer)> {
    let mixed: Vec<u32> = (0..4096).map(|i| (i % 12) as u32).collect();
    let mut dead = PalettedContainer::new(BLOCKS, 5);
    dead.set(100, 9);
    dead.set(100, 5);
    let biomes: Vec<u32> = (0..64).map(|i| (i % 6) as u32 * 2).collect();
    vec![
        ("single", PalettedContainer::new(BLOCKS, 5)),
        ("dead slot", dead),
        ("mixed", PalettedContainer::from_values(BLOCKS, &mixed)),
        (
            "terrain",
            PalettedContainer::from_values(BLOCKS, &palette_cases()[0].1),
        ),
        ("global uniform", global_uniform(3000)),
        ("biomes", PalettedContainer::from_values(BIOMES, &biomes)),
    ]
}
#[test]
fn pack_matches_unpacking_every_section() {
    for (name, container) in pack_cases() {
        assert_eq!(container.pack(), baseline_pack(&container), "{name}");
    }
}
/// A section of one value saves as that value and no indices, whatever its
/// stored palette looks like: a single palette, a list whose other entry was
/// overwritten, or the global palette.
#[test]
fn uniform_sections_pack_as_one_entry() {
    let mut dead = PalettedContainer::new(BLOCKS, 5);
    dead.set(100, 9);
    dead.set(100, 5);
    for (name, container, value) in [
        ("single", PalettedContainer::new(BLOCKS, 5), 5),
        ("dead slot", dead, 5),
        ("global", global_uniform(3000), 3000),
    ] {
        assert_eq!(
            container.pack(),
            Packed {
                palette: vec![value],
                bits: 0,
                data: None,
            },
            "{name}"
        );
    }
}
/// `pack` against the unpacking implementation, over the cases above. 25
/// measured rounds alternate order and report the median microseconds per
/// call.
#[test]
#[ignore = "microbenchmark; run with --ignored --nocapture"]
fn benchmark_pack() {
    use std::{hint::black_box, time::Instant};
    for (name, container) in pack_cases() {
        let reps = if container.uniform().is_some() {
            2000
        } else {
            200
        };
        let mut times = [Vec::new(), Vec::new()];
        for round in 0..25 {
            for optimized in [round % 2 == 0, round % 2 != 0] {
                let started = Instant::now();
                for _ in 0..reps {
                    if optimized {
                        black_box(container.pack());
                    } else {
                        black_box(baseline_pack(black_box(&container)));
                    }
                }
                if round > 0 {
                    times[usize::from(optimized)]
                        .push(started.elapsed().as_secs_f64() * 1e6 / f64::from(reps));
                }
            }
        }
        for t in &mut times {
            t.sort_by(f64::total_cmp);
        }
        let (base, new) = (times[0][times[0].len() / 2], times[1][times[1].len() / 2]);
        println!(
            "pack {name}: baseline {base:.2} us, new {new:.2} us, {:.2}x",
            base / new
        );
    }
}
