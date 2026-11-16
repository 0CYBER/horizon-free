pub use world::chunk_generator::{ChunkAccess, ChunkGenerator, VanillaStages};
mod __private {
    /// The chunk generators the mods registered, for the executable to hand to
    /// the engine before any world opens.
    pub use crate::world::chunk_generator::chunk_generator_declarations;
}
