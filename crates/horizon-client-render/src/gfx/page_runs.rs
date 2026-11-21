//! Solid draws grouped by vertex arena page within distance bands.
//!
//! Sections sit in arena pages of 256 MiB, so render distance 32 spreads
//! them over half a dozen pages, and a list sorted only by distance switches
//! page every few sections: about a thousand indirect multi-draws of two or
//! three entries a frame. Mesa's Intel driver (and wgpu-core, for each
//! vertex buffer change) pays per call, and below four entries Mesa unrolls
//! the call on the CPU entry by entry. Grouping the sections of one band of
//! distance by page keeps the list front to back to within the band (what
//! the depth test needs to reject hidden fragments early) with one call per
//! page and band.
//!
//! The picture does not change: solid draws write depth and are tested
//! against it, so their order only decides which of two fragments at the
//! same depth stays, and sections do not share faces.

/// The width of a band, in blocks.
const BAND: f64 = 64.0;
/// Exact reciprocal of `BAND` (power of two).
const INV_BAND: f64 = 1.0 / BAND;
/// Where a run's group sits in its sort key, above band and page.
const GROUP_SHIFT: u32 = 62;
/// Largest band index whose squared-distance bounds stay exact integers in `f64`.
const MAX_CACHED_BAND: u64 = 1 << 20;
/// Maximum `(group, band, page)` bucket count handled by O(N) counting sort.
const MAX_COUNTING_BUCKETS: usize = 4096;

/// One section's draws: `start..start + len` of the list.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Run {
    key: u64,
    pub start: u32,
    pub len: u32,
    /// The caller's data, carried along (occlusion culling's test inputs).
    pub tag: [u32; 3],
}

#[derive(Default)]
pub(super) struct PageRuns {
    runs: Vec<Run>,
    sorted_runs: Vec<Run>,
    counts: Vec<u32>,
    scratch_draws: Vec<[u32; 5]>,
    scratch_pages: Vec<(u32, Option<u32>)>,
    total_draws: usize,
    first_gp: u64,
    mixed_gp: bool,
    max_group: u8,
    max_band: u32,
    max_page: u32,
    cached_band: u64,
    band_lo_bits: u64,
    band_hi_bits: u64,
}

impl PageRuns {
    pub fn clear(&mut self) {
        self.runs.clear();
        self.total_draws = 0;
        self.first_gp = 0;
        self.mixed_gp = false;
        self.max_group = 0;
        self.max_band = 0;
        self.max_page = 0;
    }

    #[inline(always)]
    fn band_of(&mut self, distance: u64) -> u64 {
        if distance >= self.band_lo_bits && distance < self.band_hi_bits {
            return self.cached_band;
        }
        let band = (f64::from_bits(distance).sqrt() * INV_BAND) as u64;
        if band < MAX_CACHED_BAND {
            let lo = band as f64 * BAND;
            let hi = (band + 1) as f64 * BAND;
            self.cached_band = band;
            self.band_lo_bits = if band == 0 {
                0
            } else {
                (lo * lo).to_bits().saturating_add(2)
            };
            self.band_hi_bits = (hi * hi).to_bits().saturating_sub(2);
        }
        band
    }

    /// The draws `start..start + len` of a section `distance` from the
    /// camera (squared, as bits) in arena `page`; runs come nearest first.
    #[inline]
    pub fn push(&mut self, distance: u64, page: u32, start: usize, len: usize, tag: [u32; 3]) {
        self.push_in(0, distance, page, start, len, tag);
    }

    /// As `push`, in `group` (0 to 3): regrouping puts each group's runs
    /// before the next group's, then bands and pages within it.
    #[inline]
    pub fn push_in(
        &mut self,
        group: u8,
        distance: u64,
        page: u32,
        start: usize,
        len: usize,
        tag: [u32; 3],
    ) {
        if len > 0 {
            let band = self.band_of(distance);
            let gp = u64::from(group) << GROUP_SHIFT | u64::from(page);
            if self.runs.is_empty() {
                self.first_gp = gp;
            } else if gp != self.first_gp {
                self.mixed_gp = true;
            }
            self.max_group = self.max_group.max(group & 3);
            self.max_band = self.max_band.max(band.min(u64::from(u32::MAX)) as u32);
            self.max_page = self.max_page.max(page);
            self.total_draws += len;
            self.runs.push(Run {
                key: gp | band << 32,
                start: start as u32,
                len: len as u32,
                tag,
            });
        }
    }

    /// Reorders `draws[base..]` and their `pages`, which the pushed runs
    /// cover in order, so that each band's runs in one page are adjacent
    /// (in distance order); returns the runs at their new places.
    pub fn regroup(
        &mut self,
        draws: &mut [[u32; 5]],
        pages: &mut [(u32, Option<u32>)],
        base: usize,
    ) -> &[Run] {
        // Group and page: one of each needs no reordering.
        if !self.mixed_gp {
            return &self.runs;
        }
        // Stable O(N) counting sort over (group, band, page) when the bucket
        // space is compact (every real frame), keeping runs nearest first
        // within each bucket without per-frame heap allocation.
        let groups = usize::from(self.max_group) + 1;
        let bands = self.max_band as usize + 1;
        let pages_count = self.max_page as usize + 1;
        let num_buckets = if self.max_band < 1024 && self.max_page < 1024 {
            groups
                .checked_mul(bands)
                .and_then(|v| v.checked_mul(pages_count))
        } else {
            None
        };
        if let Some(num_buckets) = num_buckets
            && num_buckets <= MAX_COUNTING_BUCKETS
        {
            self.counts.clear();
            self.counts.resize(num_buckets, 0);
            for r in &self.runs {
                let g = (r.key >> GROUP_SHIFT) as usize;
                let b = ((r.key >> 32) & 0x3fff_ffff) as usize;
                let p = r.key as u32 as usize;
                self.counts[(g * bands + b) * pages_count + p] += 1;
            }
            let mut sum = 0u32;
            for count in &mut self.counts {
                let c = *count;
                *count = sum;
                sum += c;
            }
            let n_runs = self.runs.len();
            if self.sorted_runs.len() < n_runs {
                self.sorted_runs.resize(n_runs, self.runs[0]);
            } else {
                self.sorted_runs.truncate(n_runs);
            }
            for &r in &self.runs {
                let g = (r.key >> GROUP_SHIFT) as usize;
                let b = ((r.key >> 32) & 0x3fff_ffff) as usize;
                let p = r.key as u32 as usize;
                let bucket = (g * bands + b) * pages_count + p;
                let dst = self.counts[bucket] as usize;
                self.counts[bucket] = (dst + 1) as u32;
                self.sorted_runs[dst] = r;
            }
            std::mem::swap(&mut self.runs, &mut self.sorted_runs);
        } else {
            // Stable fallback for synthetic inputs with sparse bands or pages.
            self.runs.sort_by_key(|r| r.key);
        }
        self.scratch_draws.clear();
        self.scratch_pages.clear();
        self.scratch_draws.reserve(self.total_draws);
        self.scratch_pages.reserve(self.total_draws);
        for r in &mut self.runs {
            let s = r.start as usize;
            let len = r.len as usize;
            r.start = (base + self.scratch_draws.len()) as u32;
            if len == 1 {
                self.scratch_draws.push(draws[s]);
                self.scratch_pages.push(pages[s]);
            } else {
                let e = s + len;
                self.scratch_draws.extend_from_slice(&draws[s..e]);
                self.scratch_pages.extend_from_slice(&pages[s..e]);
            }
        }
        let n = self.scratch_draws.len();
        draws[base..base + n].copy_from_slice(&self.scratch_draws);
        pages[base..base + n].copy_from_slice(&self.scratch_pages);
        &self.runs
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bits(blocks: f64) -> u64 {
        (blocks * blocks).to_bits()
    }

    /// Runs of (distance in blocks, page, draws), as the list builder pushes them.
    fn list(runs: &[(f64, u32, u32)]) -> (PageRuns, Vec<[u32; 5]>, Vec<(u32, Option<u32>)>) {
        let mut grouped = PageRuns::default();
        let (mut draws, mut pages) = (Vec::new(), Vec::new());
        for (n, &(distance, page, count)) in runs.iter().enumerate() {
            let start = draws.len();
            for k in 0..count {
                draws.push([n as u32, k, 0, 0, 0]);
                pages.push((page, None));
            }
            grouped.push(
                bits(distance),
                page,
                start,
                count as usize,
                [n as u32, 0, 0],
            );
        }
        (grouped, draws, pages)
    }

    #[test]
    fn a_band_draws_each_page_once_keeping_the_order_inside_it() {
        let (mut grouped, mut draws, mut pages) = list(&[
            (10.0, 1, 2),
            (20.0, 0, 1),
            (30.0, 1, 3),
            (40.0, 0, 2),
            (70.0, 1, 1),
        ]);
        let runs: Vec<_> = grouped
            .regroup(&mut draws, &mut pages, 0)
            .iter()
            .map(|r| (r.tag[0], r.start, r.len))
            .collect();
        assert_eq!(
            runs,
            [(1, 0, 1), (3, 1, 2), (0, 3, 2), (2, 5, 3), (4, 8, 1)]
        );
        let order: Vec<_> = draws.iter().map(|d| (d[0], d[1])).collect();
        assert_eq!(
            order,
            [
                (1, 0),
                (3, 0),
                (3, 1),
                (0, 0),
                (0, 1),
                (2, 0),
                (2, 1),
                (2, 2),
                (4, 0)
            ]
        );
        let pages: Vec<_> = pages.iter().map(|p| p.0).collect();
        assert_eq!(pages, [0, 0, 0, 1, 1, 1, 1, 1, 1]);
    }

    #[test]
    fn bands_stay_in_distance_order() {
        let (mut grouped, mut draws, mut pages) =
            list(&[(10.0, 1, 1), (100.0, 0, 1), (200.0, 0, 1)]);
        grouped.regroup(&mut draws, &mut pages, 0);
        let order: Vec<_> = draws.iter().map(|d| d[0]).collect();
        assert_eq!(order, [0, 1, 2]);
    }

    /// Metal's two solid passes draw a prefix and a suffix of the list:
    /// groups come whole and in order, whatever their distance and page.
    #[test]
    fn groups_come_before_bands_and_pages() {
        let mut grouped = PageRuns::default();
        let mut draws: Vec<_> = (0..5).map(|n| [n, 0, 0, 0, 0]).collect();
        let mut pages = vec![(0, None); 5];
        for (n, (group, distance)) in [(2, 10.0), (0, 300.0), (1, 20.0), (0, 30.0), (2, 5.0)]
            .into_iter()
            .enumerate()
        {
            grouped.push_in(group, bits(distance), 0, n, 1, [0; 3]);
        }
        grouped.regroup(&mut draws, &mut pages, 0);
        let order: Vec<_> = draws.iter().map(|d| d[0]).collect();
        assert_eq!(order, [3, 1, 2, 0, 4]);
    }

    #[test]
    fn one_page_and_an_offset_list() {
        let (mut grouped, mut draws, mut pages) = list(&[(10.0, 3, 2), (90.0, 3, 1)]);
        let before = draws.clone();
        assert_eq!(grouped.regroup(&mut draws, &mut pages, 0)[1].start, 2);
        assert_eq!(draws, before);
        // A list after other draws: starts are absolute.
        let mut grouped = PageRuns::default();
        let mut draws = vec![[9; 5], [7, 0, 0, 0, 0], [8, 0, 0, 0, 0]];
        let mut pages = vec![(5, None), (1, None), (0, None)];
        grouped.push(bits(5.0), 1, 1, 1, [0; 3]);
        grouped.push(bits(6.0), 0, 2, 1, [1, 0, 0]);
        let runs = grouped.regroup(&mut draws, &mut pages, 1).to_vec();
        assert_eq!(draws, [[9; 5], [8, 0, 0, 0, 0], [7, 0, 0, 0, 0]]);
        assert_eq!(pages, [(5, None), (0, None), (1, None)]);
        assert_eq!((runs[0].tag[0], runs[0].start), (1, 1));
        // Empty runs are left out.
        grouped.clear();
        grouped.push(bits(1.0), 0, 0, 0, [0; 3]);
        assert!(grouped.regroup(&mut [], &mut [], 0).is_empty());
    }

    #[test]
    fn counting_sort_and_fallback_match_stable_key_order() {
        for max_page in [8u32, 2048u32] {
            let mut grouped = PageRuns::default();
            let (mut draws, mut pages) = (Vec::new(), Vec::new());
            let mut expected = Vec::new();
            let mut seed = 0x1234_5678u32;
            let mut next = || {
                seed ^= seed << 13;
                seed ^= seed >> 17;
                seed ^= seed << 5;
                seed
            };
            for i in 0..400u32 {
                let group = (next() % 3) as u8;
                let dist = f64::from((i / 2) * 3) + (next() % 100) as f64 * 0.01;
                let page = if max_page > 1024 && i % 17 == 0 {
                    max_page
                } else {
                    next() % 6
                };
                let count = 1 + (next() % 3);
                let start = draws.len();
                for k in 0..count {
                    draws.push([i, k, 0, 0, 0]);
                    pages.push((page, None));
                }
                let dist_bits = bits(dist);
                let band = (f64::from_bits(dist_bits).sqrt() / BAND) as u64;
                let key = u64::from(group) << GROUP_SHIFT | band << 32 | u64::from(page);
                expected.push((key, i, count));
                grouped.push_in(group, dist_bits, page, start, count as usize, [i, 0, 0]);
            }
            expected.sort_by_key(|e| e.0);
            let runs = grouped.regroup(&mut draws, &mut pages, 0);
            assert_eq!(runs.len(), expected.len());
            for (r, &(_, id, count)) in runs.iter().zip(&expected) {
                assert_eq!((r.tag[0], r.len), (id, count));
                for k in 0..count {
                    assert_eq!(draws[(r.start + k) as usize][0], id);
                    assert_eq!(draws[(r.start + k) as usize][1], k);
                }
            }
        }
    }
}
