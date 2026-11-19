//! Density function types a native mod registers: a `{"type": "modid:plates", ...}` node any
//! density JSON (a data pack's `worldgen/density_function`, a noise router or noise settings) can
//! name next to vanilla's, as
//! [`register_density_function_type`](crate::Registrar::register_density_function_type) hands it
//! over once at startup. The node is part of the one density graph, so chunk fill, far terrain,
//! structure heights, climate and biomes, spawn finding, aquifers and ore veins all read the same
//! values.
//!
//! There is no `Host` here: densities are sampled on the worldgen worker threads, many at once,
//! and a function keeps no mutable state.
use std::sync::Arc;

/// A mod's density function type: it reads one `{"type": "modid:name", ...}` object of density
/// JSON into a [`DensityFunction`]. Implemented once, kept by the game.
pub trait DensityFunctionType: Send + Sync + 'static {
    /// The whole JSON object (its `type` field included) as text, asked each time worldgen data
    /// loads. An `Err` refuses the noise settings that name it, with the type's id and the
    /// message.
    fn decode(&self, json: &str) -> Result<Arc<dyn DensityFunction>, String>;
}

/// One decoded density function node. Its values must be a pure function of the world seed and
/// the block position: the same seed and position give the same value on every thread, every
/// run and every machine, or chunks, far terrain and structures disagree. It must not panic: a
/// panic is not caught and goes to whatever asked for the value, as one in any native worldgen
/// hook does (on the worldgen workers, a crash report naming the mod and the type, then exit). A
/// NaN or a value outside [`range`](Self::range) panics the same way.
pub trait DensityFunction: Send + Sync + 'static {
    /// `[min, max]`, finite, `min <= max`, within ±1,000,000 (vanilla's bound on density values;
    /// beyond it the density JSON fails to load): every value [`sample`](Self::sample) and
    /// [`fill`](Self::fill) give lies in it, or the game panics naming the type. Read once, when the node decodes; vanilla's interval
    /// arithmetic (spline ranges, range choices, aquifer and vein bounds) works from it, so a
    /// wrong range gives wrong terrain.
    fn range(&self) -> [f32; 2];
    /// `false` when the value never depends on y: the game then asks for it at y = 0, once per
    /// column. Read once, when the node decodes.
    fn varies_with_y(&self) -> bool {
        true
    }
    /// The value at block position `[x, y, z]` of a world with `seed`. Asked where the reader
    /// samples single points: spawn finding and `/locate biome`, and inside the volume readers
    /// too (aquifers, `find_top_surface`, `interpolated` off the cell lattice). Chunk fill, far
    /// terrain and structure heights otherwise ask [`fill`](Self::fill). NaN is rejected.
    fn sample(&self, position: [i32; 3], seed: i64) -> f32;
    /// The values of `count[0] × count[1] × count[2]` positions: `origin + i * step` along x, y
    /// and z. `out` holds exactly that many, y fastest, then x, then z (the game's own buffer
    /// order, no copy). The default asks [`sample`](Self::sample) for each; override it to reuse
    /// work across a column or a volume. It must give what `sample` gives at each position and
    /// must write every slot: `out` may hold other values when called, and a slot left unwritten
    /// or NaN is rejected.
    fn fill(
        &self,
        origin: [i32; 3],
        step: [i32; 3],
        count: [usize; 3],
        seed: i64,
        out: &mut [f32],
    ) {
        // Wrapping, as the game's own volume coordinates are.
        let at =
            |axis: usize, i: usize| origin[axis].wrapping_add((i as i32).wrapping_mul(step[axis]));
        let mut index = 0;
        for z in 0..count[2] {
            for x in 0..count[0] {
                for y in 0..count[1] {
                    out[index] = self.sample([at(0, x), at(1, y), at(2, z)], seed);
                    index += 1;
                }
            }
        }
    }
}

/// A registered density function type.
pub(crate) struct DensityFunctionTypeInfo {
    pub(crate) id: Box<str>,
    /// The `[mod] id` of the mod that registered it, for a crash report to name.
    pub(crate) mod_id: &'static str,
    pub(crate) density_type: Arc<dyn DensityFunctionType>,
}

/// Registered density function types for the hidden game integration boundary: each id, the
/// `[mod] id` of the mod that registered it, and its type.
// hidden: the game's startup adapter reads the registered types; mods declare them through the registrar.
#[doc(hidden)]
pub fn density_function_type_declarations()
-> Vec<(&'static str, &'static str, Arc<dyn DensityFunctionType>)> {
    crate::parts::installed_density_function_type_list()
        .iter()
        .map(|info| (&*info.id, info.mod_id, info.density_type.clone()))
        .collect()
}
