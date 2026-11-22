//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;

/// The server host's methods of the `mod_slots` area, implemented by the game
/// and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait ModSlotsServerHost {
    /// Side: server. Ask every `mod_slot_will_change` listener in priority
    /// order whether the mod's pending slot change may go ahead; false when
    /// one of them cancelled it. Call this before the mod writes its stack.
    #[access(WorldOnly)]
    fn mod_slot_will_change(
        &mut self,
        holder: EntityRef,
        slot: &str,
        index: usize,
        cosmetic: bool,
        old: Option<ItemStack>,
        new: Option<ItemStack>,
        source: &str,
    ) -> bool;

    /// Side: server. Tell every `mod_slot_changed` listener that the mod's
    /// write is committed. Call this after the mod has written its stack.
    #[access(WorldOnly)]
    fn mod_slot_changed(
        &mut self,
        holder: EntityRef,
        slot: &str,
        index: usize,
        cosmetic: bool,
        old: Option<ItemStack>,
        new: Option<ItemStack>,
        source: &str,
    );
}
