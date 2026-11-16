//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `advancements` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait AdvancementsServerHost {
    /// Side: server.
    /// `ServerAdvancementManager.get(id)`: the loaded advancement `id`, from the
    /// set the last data-pack load or `/reload` built (loaded on first use).
    /// None when no advancement has that id.
    #[access(WorldOnly)]
    fn advancement(&mut self, id: &Identifier) -> Option<AdvancementInfo>;
}
