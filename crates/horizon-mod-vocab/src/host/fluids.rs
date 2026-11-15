//! Reads of the current registered cauldron fluid contents.
#![allow(unused_variables)]
use crate::{BlockKey, BlockState, CauldronContent, FluidKey};
/// The host's current cauldron content table, including mod registrations.
#[horizon_mod_api_macros::host_area]
pub trait FluidsHost {
    /// Side: both.
    /// Contents registered for `block`, or none if absent or no table has been built.
    /// Reads the current resource generation, including after reload.
    #[access(WorldOnly)]
    fn cauldron_content_for_block(&self, block: BlockKey) -> Option<CauldronContent>;
    /// Side: both.
    /// Contents registered for `fluid`, or none if absent or no table has been built.
    #[access(WorldOnly)]
    fn cauldron_content_for_fluid(&self, fluid: FluidKey) -> Option<CauldronContent>;
    /// Side: both.
    /// Current level of `state`, or none without registered contents or a built table.
    /// Always-full contents answer their maximum level.
    #[access(WorldOnly)]
    fn cauldron_level(&self, state: BlockState) -> Option<u32>;
}
