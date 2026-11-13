//! The chunks a `/forceload` or a native mod's ticket holds, as `Ranges` reads them, and their
//! distance map.
use super::{FORCED_FEATURES_RING, FORCED_FULL_RING, FORCED_TERRAIN_RING, Ranges};
use crate::fast_hash::FxHashMap;
use std::collections::HashSet;

/// `Streaming::forced` and `mod_forced` as the Chebyshev distance from each column within
/// `REACH` of one to the nearest: `Ranges::in_main_range` answers by one lookup instead of going
/// through every forced chunk (it is asked for every column, several times a tick). Past `MOST`
/// forced chunks, and for a reach past `REACH`, their tiles answer.
pub(super) struct ForcedCover {
    forced: HashSet<(i32, i32)>,
    mod_forced: HashSet<(i32, i32)>,
    /// None past `MOST`.
    near: Option<FxHashMap<(i32, i32), u8>>,
    tiles: Tiles,
}
impl ForcedCover {
    /// The widest ring `in_main_range` asks of a forced chunk (margins 1 to 4).
    const REACH: i32 = FORCED_TERRAIN_RING + 1;
    /// Past this many forced chunks the distance map is not made (it holds up to
    /// `(2 * REACH + 1)^2` entries a forced chunk): the tiles answer.
    const MOST: usize = 4096;
    pub(super) fn refresh(
        cover: &mut Option<Self>,
        forced: &HashSet<(i32, i32)>,
        mod_forced: &HashSet<(i32, i32)>,
    ) {
        if cover
            .as_ref()
            .is_some_and(|c| c.forced == *forced && c.mod_forced == *mod_forced)
        {
            return;
        }
        let near = (forced.len() + mod_forced.len() <= Self::MOST).then(|| {
            let mut near = FxHashMap::default();
            for &(fx, fz) in forced.iter().chain(mod_forced) {
                for dx in -Self::REACH..=Self::REACH {
                    for dz in -Self::REACH..=Self::REACH {
                        let d = dx.abs().max(dz.abs()) as u8;
                        near.entry((fx + dx, fz + dz))
                            .and_modify(|e: &mut u8| *e = (*e).min(d))
                            .or_insert(d);
                    }
                }
            }
            near
        });
        *cover = Some(Self {
            forced: forced.clone(),
            mod_forced: mod_forced.clone(),
            near,
            tiles: Tiles::new(forced.iter().chain(mod_forced)),
        });
    }
    /// Whether a forced chunk lies within `reach` of `(x, z)` (Chebyshev).
    pub(super) fn within(&self, x: i32, z: i32, reach: i32) -> bool {
        match &self.near {
            Some(near) if reach <= Self::REACH => {
                near.get(&(x, z)).is_some_and(|&d| i32::from(d) <= reach)
            }
            _ => self.tiles.within(x, z, reach),
        }
    }
}

/// The forced chunks as bits, 8 by 8 columns a word: `(x, z)`'s bit is `(z & 7) * 8 + (x & 7)`
/// of the word at `(x >> 3, z >> 3)`. A question reads only the words its square overlaps (four
/// at most for a reach up to 4, nine up to 8), however many chunks are forced. It holds one word
/// and its key for each tile with a forced chunk in it.
struct Tiles(FxHashMap<(i32, i32), u64>);
impl Tiles {
    fn new<'a>(chunks: impl Iterator<Item = &'a (i32, i32)>) -> Self {
        let mut tiles = FxHashMap::default();
        for &(x, z) in chunks {
            *tiles.entry((x >> 3, z >> 3)).or_insert(0) |= 1u64 << ((z & 7) * 8 + (x & 7));
        }
        Self(tiles)
    }
    /// Whether a chunk's bit is set within `reach` of `(x, z)` (Chebyshev).
    fn within(&self, x: i32, z: i32, reach: i32) -> bool {
        if reach < 0 {
            return false;
        }
        let (x0, x1) = (x.saturating_sub(reach), x.saturating_add(reach));
        let (z0, z1) = (z.saturating_sub(reach), z.saturating_add(reach));
        // The square's bits in the tile at (tx, tz).
        let hit = |(tx, tz): (i32, i32), bits: u64| {
            let (bx, bz) = (i64::from(tx) * 8, i64::from(tz) * 8);
            let lx0 = (i64::from(x0) - bx).max(0);
            let lx1 = (i64::from(x1) - bx).min(7);
            let lz0 = (i64::from(z0) - bz).max(0);
            let lz1 = (i64::from(z1) - bz).min(7);
            if lx0 > lx1 || lz0 > lz1 {
                return false;
            }
            // Columns lx0..=lx1 of one row, copied to every row, then rows lz0..=lz1.
            let row = (0xFFu64 >> (7 - (lx1 - lx0))) << lx0;
            let rows = (u64::MAX >> (8 * (7 - (lz1 - lz0)))) << (8 * lz0);
            bits & row.wrapping_mul(0x0101_0101_0101_0101) & rows != 0
        };
        let (tx0, tx1, tz0, tz1) = (x0 >> 3, x1 >> 3, z0 >> 3, z1 >> 3);
        let overlapped = (i64::from(tx1 - tx0) + 1) * (i64::from(tz1 - tz0) + 1);
        // A square wider than the tiles held (a far reach): go through the tiles instead.
        if overlapped > self.0.len() as i64 {
            return self.0.iter().any(|(&t, &bits)| hit(t, bits));
        }
        (tz0..=tz1).any(|tz| {
            (tx0..=tx1).any(|tx| {
                self.0
                    .get(&(tx, tz))
                    .is_some_and(|&bits| hit((tx, tz), bits))
            })
        })
    }
}

impl Ranges<'_> {
    /// Whether a `/forceload` or a native mod's ticket holds the chunk.
    pub(super) fn is_forced(&self, x: i32, z: i32) -> bool {
        self.forced.contains(&(x, z)) || self.mod_forced.contains(&(x, z))
    }
    /// Every chunk a `/forceload` or a native mod's ticket holds, once each.
    pub(super) fn forced_chunks(&self) -> impl Iterator<Item = &(i32, i32)> + '_ {
        self.forced
            .iter()
            .chain(self.mod_forced.iter().filter(|c| !self.forced.contains(c)))
    }
    /// `in_main_range` past the player's circle: whether a forced chunk lies within the ring
    /// `margin` asks for.
    pub(super) fn near_forced(&self, x: i32, z: i32, margin: i32) -> bool {
        // A forced chunk holds a FORCED ticket at level 31
        // (`ChunkMap.FORCED_TICKET_LEVEL`), and the level rises one per ring
        // (`ChunkTracker.computeLevelFromNeighbor`): the margin is the
        // vanilla status ring the caller asks for.
        let reach = match margin {
            1 => FORCED_FULL_RING,
            2 => FORCED_FEATURES_RING,
            3 => FORCED_TERRAIN_RING,
            _ => margin + 1,
        };
        self.forced_within(x, z, reach)
    }
    /// Whether a forced chunk lies within `reach` of `(x, z)` (Chebyshev): from the cover while
    /// `update` runs, else by a scan of every forced chunk.
    pub(super) fn forced_within(&self, x: i32, z: i32, reach: i32) -> bool {
        if let Some(cover) = self.forced_cover {
            return cover.within(x, z, reach);
        }
        self.forced_chunks()
            .any(|f| (x - f.0).abs() <= reach && (z - f.1).abs() <= reach)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chunks(seed: &mut u64, n: usize, spread: i32) -> HashSet<(i32, i32)> {
        let mut next = |m: i32| {
            *seed ^= *seed << 13;
            *seed ^= *seed >> 7;
            *seed ^= *seed << 17;
            (*seed % (2 * m as u64 + 1)) as i32 - m
        };
        (0..n).map(|_| (next(spread), next(spread))).collect()
    }

    /// With the forced chunks by distance, every column gets the answer the scan of every forced
    /// chunk gives, at every margin streaming asks (and those past the map's reach), after the
    /// forced chunks change too.
    #[test]
    fn the_distances_answer_as_the_scan() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        let mut cover = None;
        for case in 0..24 {
            let spread = [3, 12, 40][case % 3];
            let forced = chunks(&mut seed, 1 + case % 9, spread);
            // Some mod tickets on forced chunks, some apart.
            let mut mod_forced = chunks(&mut seed, case % 4, spread);
            mod_forced.extend(forced.iter().take(case % 2));
            ForcedCover::refresh(&mut cover, &forced, &mod_forced);
            let made = cover.as_ref().expect("a few forced chunks get a map");
            let ranges = |forced_cover| Ranges {
                // The player's circle far away: only the forced chunks answer.
                center: (10_000, 10_000),
                radius: 2,
                extra: &[],
                forced: &forced,
                mod_forced: &mod_forced,
                cover: None,
                forced_cover,
                band: None,
            };
            let (scan, map) = (ranges(None), ranges(Some(made)));
            let span = spread + 10;
            for z in -span..=span {
                for x in -span..=span {
                    for margin in 0..=7 {
                        assert_eq!(
                            map.in_main_range(x, z, margin),
                            scan.in_main_range(x, z, margin),
                            "case {case} ({x}, {z}) margin {margin}"
                        );
                    }
                    for d in [false, true] {
                        assert_eq!(map.requested(x, z, d), scan.requested(x, z, d));
                        assert_eq!(map.chunk_kept(x, z, d), scan.chunk_kept(x, z, d));
                        assert_eq!(map.unloaded(x, z, d), scan.unloaded(x, z, d));
                    }
                }
            }
        }
    }

    /// Too many forced chunks for a map: none is made, and the tiles answer.
    #[test]
    fn past_the_most_no_map_is_made() {
        let forced: HashSet<(i32, i32)> = (0..=ForcedCover::MOST as i32).map(|x| (x, 0)).collect();
        let mut cover = None;
        ForcedCover::refresh(&mut cover, &HashSet::new(), &HashSet::from([(0, 0)]));
        assert!(cover.as_ref().is_some_and(|c| c.near.is_some()));
        ForcedCover::refresh(&mut cover, &forced, &HashSet::new());
        let made = cover
            .as_ref()
            .expect("a cover is made however many are forced");
        assert!(made.near.is_none());
        assert!(made.within(-3, 3, 3) && !made.within(-3, 4, 3));
    }

    /// Forced chunks in a disc, in clusters and scattered: `n` of them (the clusters a few more).
    fn sets(seed: &mut u64, n: usize) -> [HashSet<(i32, i32)>; 3] {
        let r = (n as f64 / std::f64::consts::PI).sqrt().ceil() as i32;
        let disc: HashSet<_> = (-r..=r)
            .flat_map(|z| (-r..=r).map(move |x| (x, z)))
            .filter(|&(x, z)| x * x + z * z <= r * r)
            .take(n)
            .collect();
        let mut clusters = HashSet::new();
        while clusters.len() < n {
            let (cx, cz) = chunks(seed, 1, 400).into_iter().next().unwrap();
            clusters.extend(
                chunks(seed, 64, 6)
                    .into_iter()
                    .map(|(x, z)| (cx + x, cz + z)),
            );
        }
        let spread = (n as f64).sqrt() as i32 * 6;
        let mut scattered = HashSet::new();
        while scattered.len() < n {
            scattered.extend(chunks(seed, n - scattered.len(), spread));
        }
        [disc, clusters, scattered]
    }

    /// Well past `MOST`, and either side of it, the cover answers every reach as the scan of
    /// every forced chunk does: around forced chunks, between them, on tile edges and far off,
    /// with some held by both a `/forceload` and a mod's ticket.
    #[test]
    fn past_the_most_the_tiles_answer_as_the_scan() {
        let mut seed = 0x9E37_79B9_7F4A_7C15u64;
        let m = ForcedCover::MOST;
        // Some held both ways past the boundary, none at it (a chunk held both ways counts twice
        // towards `MOST`, as `refresh` counts them).
        for (n, both) in [(m, false), (m + 1, false), (6000, true), (20_000, true)] {
            for (kind, all) in sets(&mut seed, n).into_iter().enumerate() {
                // Every third chunk a mod's ticket; with `both`, every fifth held both ways.
                let (mut forced, mut mod_forced) = (HashSet::new(), HashSet::new());
                for (i, &c) in all.iter().enumerate() {
                    if i % 3 == 0 || (both && i % 5 == 0) {
                        mod_forced.insert(c);
                    }
                    if i % 3 != 0 {
                        forced.insert(c);
                    }
                }
                let mut cover = None;
                ForcedCover::refresh(&mut cover, &forced, &mod_forced);
                let made = cover.as_ref().unwrap();
                let held = forced.len() + mod_forced.len();
                assert_eq!(made.near.is_some(), held <= m);
                let ranges = |forced_cover| Ranges {
                    center: (10_000_000, 10_000_000),
                    radius: 2,
                    extra: &[],
                    forced: &forced,
                    mod_forced: &mod_forced,
                    cover: None,
                    forced_cover,
                    band: None,
                };
                let (scan, map) = (ranges(None), ranges(Some(made)));
                // The scan goes through every forced chunk: fewer columns for more of them.
                let near: Vec<_> = all.iter().step_by(all.len() / 40).copied().collect();
                let offsets = chunks(&mut seed, 6, 12);
                let far = chunks(&mut seed, 12, 3000);
                let columns = near
                    .iter()
                    .flat_map(|&(x, z)| offsets.iter().map(move |&(dx, dz)| (x + dx, z + dz)))
                    .chain(far);
                for (x, z) in columns {
                    for reach in [-1, 0, 1, 2, 3, 4, 5, 6, 8, 11, 40] {
                        assert_eq!(
                            map.forced_within(x, z, reach),
                            scan.forced_within(x, z, reach),
                            "{n} forced, set {kind}, ({x}, {z}) reach {reach}"
                        );
                    }
                    for margin in 1..=4 {
                        assert_eq!(
                            map.in_main_range(x, z, margin),
                            scan.in_main_range(x, z, margin)
                        );
                    }
                }
            }
        }
    }

    /// Each tile's bits answer for every square that overlaps it in part: corners, edges and
    /// across negative coordinates.
    #[test]
    fn a_tiles_edges_answer_as_the_scan() {
        let mut seed = 0xD1B5_4A32_D192_ED03u64;
        for _ in 0..200 {
            let set = chunks(&mut seed, 3, 9);
            let tiles = Tiles::new(set.iter());
            for z in -14..=14 {
                for x in -14..=14 {
                    for reach in 0..=9 {
                        let scan = set
                            .iter()
                            .any(|f| (x - f.0).abs() <= reach && (z - f.1).abs() <= reach);
                        assert_eq!(
                            tiles.within(x, z, reach),
                            scan,
                            "{set:?} ({x}, {z}) {reach}"
                        );
                    }
                }
            }
        }
    }

    /// What a question and the cover cost by how many chunks are forced: run with
    /// `cargo test --release -p horizon-client-render forced_cover_cost -- --ignored --nocapture`.
    #[test]
    #[ignore = "timings; run with --release --ignored --nocapture"]
    fn forced_cover_cost() {
        let mut seed = 0x2545_F491_4F6C_DD1Du64;
        for n in [1000, 4000, 4097, 6000, 20_000] {
            for (kind, forced) in sets(&mut seed, n).into_iter().enumerate() {
                let none = HashSet::new();
                let started = std::time::Instant::now();
                let mut cover = None;
                ForcedCover::refresh(&mut cover, &forced, &none);
                let made_in = started.elapsed();
                let made = cover.as_ref().unwrap();
                let ranges = |forced_cover| Ranges {
                    center: (10_000_000, 10_000_000),
                    radius: 2,
                    extra: &[],
                    forced: &forced,
                    mod_forced: &none,
                    cover: None,
                    forced_cover,
                    band: None,
                };
                let spread = forced
                    .iter()
                    .map(|f| f.0.abs().max(f.1.abs()))
                    .max()
                    .unwrap()
                    + 20;
                let columns = chunks(&mut seed, 200_000, spread);
                let time = |r: &Ranges, take: usize| {
                    let started = std::time::Instant::now();
                    let mut hits = 0usize;
                    for &(x, z) in columns.iter().take(take) {
                        for margin in 1..=4 {
                            hits += usize::from(r.in_main_range(x, z, margin));
                        }
                    }
                    (
                        started.elapsed().as_nanos() as f64 / (take * 4) as f64,
                        hits,
                    )
                };
                let (cover_ns, _) = time(&ranges(Some(made)), columns.len());
                let (scan_ns, _) = time(&ranges(None), 2000);
                let bytes = made.tiles.0.capacity() * (16 + 1)
                    + made.near.as_ref().map_or(0, |m| m.capacity() * (12 + 1));
                println!(
                    "{n:>6} forced ({}): cover {cover_ns:6.1} ns a question, scan {scan_ns:8.0} ns; \
                     made in {made_in:?}, {} tiles, distance map {}, ~{} KiB",
                    ["disc", "clusters", "scattered"][kind],
                    made.tiles.0.len(),
                    made.near
                        .as_ref()
                        .map_or("none".to_string(), |m| m.len().to_string()),
                    bytes / 1024
                );
            }
        }
    }
}
