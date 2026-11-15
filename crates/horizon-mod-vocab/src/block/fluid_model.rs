//! Client world-fluid model declarations.
/// Atlas sprites to use for a world fluid during this resource load.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct FluidModel {
    /// Canonical world fluid registry name.
    pub fluid: String,
    /// Surface-at-rest sprite; absent keeps the content look.
    pub still: Option<String>,
    /// Flowing and side sprite; absent uses the still sprite.
    pub flowing: Option<String>,
    /// Side sprite against glass or leaves; absent uses the flowing sprite.
    pub overlay: Option<String>,
}
