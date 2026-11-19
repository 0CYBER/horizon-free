//! Density function types a mod registers, handed over once at startup and looked up when density
//! JSON names one (`{"type": "modid:plates", ...}`). A decoded one is a node of the density graph,
//! so every reader of the graph sees it. Like a mod's biome source it runs on the worldgen worker
//! threads, so it keeps no mutable state and has no `Host`.
use horizon_core::{
    density::{DensitySampler, SampleError, SamplerContext, graph::SeededDensity},
    noise::{DensityError, DensityVolume},
};
use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock};

/// A density function type's engine side, as the executable's adapter sees it.
pub trait ModDensityFunctionType: Send + Sync {
    /// The `[mod] id` of the mod that registered it, which a broken value's panic names.
    fn owner(&self) -> &str;
    /// One `{"type": "modid:name", ...}` object, as JSON text, into its function; `Err` refuses
    /// the density JSON that holds it.
    fn decode(&self, json: &str) -> Result<Arc<dyn ModDensityFunction>, String>;
}

/// A decoded node: a pure function of the world seed and the block position. A panic in it is not
/// caught: it unwinds out of whatever samples, to that caller's panic policy. A value that is NaN
/// or outside `range` (in `fill`, a slot left unwritten) panics the same way, naming the mod and
/// the type.
pub trait ModDensityFunction: Send + Sync {
    /// `[min, max]` of every value, finite and within ±[`MAX_REASONABLE`].
    fn range(&self) -> [f32; 2];
    /// `false`: the value never depends on y.
    fn varies_with_y(&self) -> bool;
    fn sample(&self, position: [i32; 3], seed: i64) -> f32;
    /// `count` positions from `origin`, `step` apart, into `out` (exactly that many values, y
    /// fastest, then x, then z).
    fn fill(&self, origin: [i32; 3], step: [i32; 3], count: [usize; 3], seed: i64, out: &mut [f32]);
}

/// The installed density function types, `modid:plates` → type. Set once, before any world opens
/// (the executable hands the mods' registrations over).
pub type ModDensityFunctionTypes = BTreeMap<String, Arc<dyn ModDensityFunctionType>>;

static MOD_DENSITY_FUNCTION_TYPES: OnceLock<ModDensityFunctionTypes> = OnceLock::new();
static EMPTY: ModDensityFunctionTypes = BTreeMap::new();

/// Install the process-wide table once; a second installation keeps the first.
pub fn install_mod_density_function_types(
    types: ModDensityFunctionTypes,
) -> Result<(), ModDensityFunctionTypes> {
    MOD_DENSITY_FUNCTION_TYPES.set(types)
}

/// The installed density function types: empty until the executable installs the mods'.
pub fn mod_density_function_types() -> &'static ModDensityFunctionTypes {
    MOD_DENSITY_FUNCTION_TYPES.get().unwrap_or(&EMPTY)
}

/// The largest magnitude of a declared range: vanilla's `MAX_REASONABLE_DOUBLE`, the bound the
/// decoder puts on a density JSON's constants. Within it a value can't overflow the integer
/// arithmetic of the readers (`find_top_surface`'s cell steps).
pub const MAX_REASONABLE: f32 = 1e6;

/// A decoded mod node in the graph, waiting for the compiler's seed.
pub(super) struct ModDensity {
    pub(super) id: String,
    /// `ModDensityFunctionType::owner`.
    pub(super) owner: String,
    /// The declared range, checked against `MAX_REASONABLE` at decode.
    pub(super) range: [f32; 2],
    pub(super) function: Arc<dyn ModDensityFunction>,
}
impl SeededDensity for ModDensity {
    fn sampler(&self, seed: i64) -> Arc<dyn DensitySampler> {
        Arc::new(ModDensitySampler {
            id: self.id.clone(),
            owner: self.owner.clone(),
            range: self.range,
            function: self.function.clone(),
            seed,
        })
    }
}
impl std::fmt::Debug for ModDensity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModDensity")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

/// A mod node bound to one world's seed: a volume is one call of the mod's `fill`.
struct ModDensitySampler {
    id: String,
    owner: String,
    range: [f32; 2],
    function: Arc<dyn ModDensityFunction>,
    seed: i64,
}
impl ModDensitySampler {
    /// Within the declared range, which NaN never is.
    fn admits(&self, value: f32) -> bool {
        self.range[0] <= value && value <= self.range[1]
    }
    /// A broken value is the mod's bug. It is raised as a panic (the caller's panic policy, as for
    /// a panic in the mod's own code), so it never reaches terrain, a cache or an error the game
    /// recovers from as a failed chunk.
    #[cold]
    #[inline(never)]
    fn reject_slot(&self, slot: usize, value: f32) -> ! {
        let (mod_id, id, [min, max]) = (&self.owner, &self.id, self.range);
        if value.is_nan() {
            panic!("native mod {mod_id} density type {id} left slot {slot} unwritten or wrote NaN")
        }
        panic!(
            "native mod {mod_id} density type {id} wrote {value} at slot {slot}, outside its declared range [{min}, {max}]"
        )
    }
    #[cold]
    #[inline(never)]
    fn reject_point(&self, position: [i32; 3], value: f32) -> ! {
        let (mod_id, id, [min, max]) = (&self.owner, &self.id, self.range);
        if value.is_nan() {
            panic!("native mod {mod_id} density type {id} returned NaN at {position:?}")
        }
        panic!(
            "native mod {mod_id} density type {id} returned {value} at {position:?}, outside its declared range [{min}, {max}]"
        )
    }
}
impl DensitySampler for ModDensitySampler {
    fn sample_value(&self, _: &mut SamplerContext, position: [i32; 3]) -> Result<f32, SampleError> {
        let value = self.function.sample(position, self.seed);
        if !self.admits(value) {
            self.reject_point(position, value);
        }
        Ok(value)
    }
    fn sample_volume(
        &self,
        _: &mut SamplerContext,
        buffer: &mut [f32],
        volume: DensityVolume,
    ) -> Result<(), SampleError> {
        let out = buffer
            .get_mut(..volume.size())
            .ok_or(DensityError::BufferTooShort)?;
        // The buffer can be another node's or another chunk's scratch: a slot the mod skips must
        // not keep that density.
        out.fill(f32::NAN);
        self.function.fill(
            volume.minimum(),
            volume.steps(),
            volume.sizes(),
            self.seed,
            out,
        );
        if let Some(slot) = out.iter().position(|&value| !self.admits(value)) {
            self.reject_slot(slot, out[slot]);
        }
        Ok(())
    }
    /// Each value depends on its own position (and the fixed seed) only, so a lattice sampled
    /// once can be reused, as vanilla's noises are.
    fn pointwise_inputs(&self) -> Option<Vec<&Arc<dyn DensitySampler>>> {
        Some(Vec::new())
    }
}
impl std::fmt::Debug for ModDensitySampler {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ModDensitySampler")
            .field("id", &self.id)
            .field("seed", &self.seed)
            .finish_non_exhaustive()
    }
}
