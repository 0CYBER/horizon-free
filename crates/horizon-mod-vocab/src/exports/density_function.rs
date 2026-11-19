pub use world::density_function::{DensityFunction, DensityFunctionType};
mod __private {
    /// The density function types the mods registered, for the executable to hand to the engine
    /// before any world opens.
    pub use crate::world::density_function::density_function_type_declarations;
}
