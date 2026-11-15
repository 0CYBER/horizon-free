//! Selector version 1 of a multi-noise preset's weighted mod regions (jittered cells), and the
//! biome source that resolves each quart in its owning region's own climate index.
use super::{BiomeSourceError, regions::RegionLayout};
use crate::worldgen::{
    biome::{BiomeError, BiomeResolver, MultiNoiseBiomeSource},
    climate::{ClimateSampler, ClimateVolume},
};
use horizon_core::{
    density::SamplerContext,
    position::BlockPos,
    random::{PositionalRandom, RandomSource},
};

/// The widest cell: squared distances across a 3x3 cell neighbourhood stay inside `i64`.
const MAX_CELL: u32 = 1 << 30;

/// Why a saved layout's selector cannot run.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegionError {
    /// A selector version this build does not know.
    Version(u32),
    /// A cell width outside `1..=2^30` quarts.
    Cell(u32),
    /// A mod weight is outside 1..=1000, or the weights sum outside 1..=i32::MAX.
    Weight,
}
impl std::fmt::Display for RegionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Version(version) => write!(f, "unknown region selector version {version}"),
            Self::Cell(cell) => write!(f, "region cell width {cell} is outside 1..={MAX_CELL}"),
            Self::Weight => {
                f.write_str("mod region weights must be 1..=1000 and total weight 1..=2147483647")
            }
        }
    }
}
impl std::error::Error for RegionError {}

type CellDraw = dyn Fn(i32, i32) -> [i32; 3] + Send + Sync;

/// Which slot owns each column: slot 0 is vanilla, slot `i + 1` the layout's region `i`.
pub struct RegionSelector {
    cell: i32,
    /// Each slot's exclusive end on the cumulative weight line.
    ends: Vec<i32>,
    /// A cell's jitter x, jitter z and weight pick, drawn in that order.
    draw: Box<CellDraw>,
}
impl std::fmt::Debug for RegionSelector {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RegionSelector")
            .field("cell", &self.cell)
            .field("ends", &self.ends)
            .finish_non_exhaustive()
    }
}
impl RegionSelector {
    /// Reads `root` (the generator's positional random) without advancing it.
    pub fn new<P>(layout: &RegionLayout, root: &P) -> Result<Self, RegionError>
    where
        P: PositionalRandom,
        <P::Source as RandomSource>::Positional: Send + Sync + 'static,
    {
        let params = &layout.selector;
        if params.version != 1 {
            return Err(RegionError::Version(params.version));
        }
        if !(1..=MAX_CELL).contains(&params.cell) {
            return Err(RegionError::Cell(params.cell));
        }
        if layout
            .regions
            .iter()
            .any(|region| !(1..=1000).contains(&region.weight))
        {
            return Err(RegionError::Weight);
        }
        let cell = params.cell as i32;
        let mut total = 0i32;
        let mut ends = Vec::with_capacity(layout.regions.len() + 1);
        for weight in
            std::iter::once(params.vanilla_weight).chain(layout.regions.iter().map(|r| r.weight))
        {
            total = i32::try_from(weight)
                .ok()
                .and_then(|weight| total.checked_add(weight))
                .ok_or(RegionError::Weight)?;
            ends.push(total);
        }
        if total == 0 {
            return Err(RegionError::Weight);
        }
        let stream = root.from_hash_of(&params.salt).fork_positional();
        let draw = move |x, z| {
            let mut random = stream.at(BlockPos::new(x, 0, z));
            let mut next = |bound| {
                random
                    .next_int_bound(bound)
                    .expect("cell and total weight are checked positive")
            };
            [next(cell), next(cell), next(total)]
        };
        Ok(Self {
            cell,
            ends,
            draw: Box::new(draw),
        })
    }
    /// The cell's jitter point in quarts and the slot its pick chose.
    fn seed(&self, cell_x: i64, cell_z: i64) -> (i64, i64, usize) {
        let [x, z, pick] = (self.draw)(cell_x as i32, cell_z as i32);
        let cell = i64::from(self.cell);
        (
            cell_x * cell + i64::from(x),
            cell_z * cell + i64::from(z),
            self.ends.partition_point(|&end| end <= pick),
        )
    }
    /// The slot a cell's pick chose (cell coordinates, not quarts).
    pub fn cell_slot(&self, cell_x: i32, cell_z: i32) -> usize {
        self.seed(cell_x.into(), cell_z.into()).2
    }
    /// The slot owning the column: the nearest jitter point among the 3x3 cells around it,
    /// the first one on a tie.
    pub fn owner(&self, quart_x: i32, quart_z: i32) -> usize {
        let (qx, qz) = (i64::from(quart_x), i64::from(quart_z));
        let cell = i64::from(self.cell);
        let (cx, cz) = (qx.div_euclid(cell), qz.div_euclid(cell));
        let mut best = (i64::MAX, 0);
        for dz in -1..=1 {
            for dx in -1..=1 {
                let (x, z, slot) = self.seed(cx + dx, cz + dz);
                let distance = (x - qx) * (x - qx) + (z - qz) * (z - qz);
                if distance < best.0 {
                    best = (distance, slot);
                }
            }
        }
        best.1
    }
}

/// A multi-noise preset with a world's regions: each column's owner picks the climate index
/// its quarts are searched in.
#[derive(Debug)]
pub struct RegionBiomeSource {
    /// Vanilla's index first, then each region's that is not substituted.
    indices: Vec<MultiNoiseBiomeSource>,
    /// Per slot, its index in `indices`; a substituted region routes to vanilla's.
    routes: Vec<usize>,
    selector: RegionSelector,
}
impl RegionBiomeSource {
    pub fn new<P>(
        vanilla: MultiNoiseBiomeSource,
        layout: &RegionLayout,
        root: &P,
    ) -> Result<Self, BiomeSourceError>
    where
        P: PositionalRandom,
        <P::Source as RandomSource>::Positional: Send + Sync + 'static,
    {
        let selector = RegionSelector::new(layout, root)?;
        let mut indices = vec![vanilla];
        let mut routes = vec![0];
        for region in &layout.regions {
            if region.substituted {
                routes.push(0);
                continue;
            }
            let entries = region
                .points
                .iter()
                .map(|point| Ok((point.parameter_point()?, point.biome.clone())))
                .collect::<Result<_, BiomeError>>()?;
            routes.push(indices.len());
            indices.push(MultiNoiseBiomeSource::new(entries)?);
        }
        Ok(Self {
            indices,
            routes,
            selector,
        })
    }
    pub fn selector(&self) -> &RegionSelector {
        &self.selector
    }
    /// Vanilla's biomes in its parameter order, then each resolving region's.
    pub fn biomes(&self) -> impl Iterator<Item = &str> {
        self.indices.iter().flat_map(|index| {
            index
                .parameters()
                .entries()
                .iter()
                .map(|(_, name)| name.as_str())
        })
    }
    /// One search cache per index; keep one resolver per worker.
    pub fn resolver<'a>(&'a self, climate: &'a ClimateSampler) -> RegionResolver<'a> {
        RegionResolver {
            source: self,
            searches: self
                .indices
                .iter()
                .map(|index| index.resolver(climate))
                .collect(),
            columns: None,
        }
    }
}
#[derive(Debug)]
pub struct RegionResolver<'a> {
    source: &'a RegionBiomeSource,
    searches: Vec<BiomeResolver<'a>>,
    /// One chunk's 16 quart columns, retained across interleaved height visits.
    columns: Option<([i32; 2], [Option<usize>; 16])>,
}
impl<'a> RegionResolver<'a> {
    fn search(&mut self, quart: [i32; 3]) -> &mut BiomeResolver<'a> {
        let chunk = [quart[0].div_euclid(4), quart[2].div_euclid(4)];
        if self.columns.as_ref().is_none_or(|(last, _)| *last != chunk) {
            self.columns = Some((chunk, [None; 16]));
        }
        let column = (quart[0].rem_euclid(4) + 4 * quart[2].rem_euclid(4)) as usize;
        let columns = &mut self.columns.as_mut().expect("chunk cache initialized").1;
        let search = *columns[column].get_or_insert_with(|| {
            self.source.routes[self.source.selector.owner(quart[0], quart[2])]
        });
        &mut self.searches[search]
    }
    pub fn sample(
        &mut self,
        context: &mut SamplerContext,
        quart: [i32; 3],
    ) -> Result<&'a str, BiomeError> {
        self.search(quart).sample(context, quart)
    }
    /// Every index reads the same climate, so one volume serves them all.
    pub fn prepare_volume(
        &self,
        context: &mut SamplerContext,
        minimum: [i32; 3],
        extents: [i32; 3],
    ) -> Result<ClimateVolume, BiomeError> {
        self.searches[0].prepare_volume(context, minimum, extents)
    }
    pub fn sample_prepared(
        &mut self,
        volume: &ClimateVolume,
        quart: [i32; 3],
    ) -> Result<&'a str, BiomeError> {
        self.search(quart).sample_prepared(volume, quart)
    }
}

#[cfg(test)]
mod cache_tests {
    use super::*;
    use crate::worldgen::climate::{Parameter, ParameterPoint};
    use horizon_core::density::{ConstantSampler, DensitySampler};
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };

    #[test]
    fn chunk_columns_are_reused_across_heights_and_reset_at_chunk_edges() {
        let draws = Arc::new(AtomicUsize::new(0));
        let calls = draws.clone();
        let source = RegionBiomeSource {
            indices: vec![
                MultiNoiseBiomeSource::new(vec![(
                    ParameterPoint {
                        coordinates: [Parameter::new(0, 0); 6],
                        offset: 0,
                    },
                    "minecraft:plains".into(),
                )])
                .unwrap(),
            ],
            routes: vec![0],
            selector: RegionSelector {
                cell: 8,
                ends: vec![10],
                draw: Box::new(move |_, _| {
                    calls.fetch_add(1, Ordering::Relaxed);
                    [0, 0, 0]
                }),
            },
        };
        let climate = ClimateSampler::new(std::array::from_fn(|_| {
            Arc::new(ConstantSampler(0.)) as Arc<dyn DensitySampler>
        }));
        let mut resolver = source.resolver(&climate);
        for x in -4..0 {
            for y in -8..8 {
                for z in -4..0 {
                    resolver.search([x, y, z]);
                }
            }
        }
        assert_eq!(draws.load(Ordering::Relaxed), 16 * 9);
        resolver.search([0, 0, 0]);
        resolver.search([0, 20, 0]);
        assert_eq!(draws.load(Ordering::Relaxed), 17 * 9);
        resolver.search([-1, 0, -1]);
        assert_eq!(draws.load(Ordering::Relaxed), 18 * 9);
    }

    #[test]
    fn a_tie_keeps_the_first_cell_and_a_slot_end_belongs_to_the_next_slot() {
        let selector = RegionSelector {
            cell: 8,
            ends: vec![10, 11],
            draw: Box::new(|x, _| [0, 0, if x < 0 { 10 } else { 9 }]),
        };
        assert_eq!(selector.seed(-1, 0), (-8, 0, 1));
        assert_eq!(selector.seed(0, 0), (0, 0, 0));
        assert_eq!(selector.owner(-4, 0), 1);
    }
}
