//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;

/// The host's methods both sides share of the `resources` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait ResourcesHost {
    /// Side: both.
    /// The winning bytes of resource `id` in `pack_type`'s stack, as
    /// `ResourceManager.getResource` answers it: the highest selected pack that
    /// supplies the file wins. Client resources are the selected asset packs over
    /// the game's assets; server data is the world's selected data packs over the
    /// game's and mods' data, with `minecraft:vanilla` kept unless disabled.
    /// `identifier` is the resource name without its `assets/<namespace>/` or
    /// `data/<namespace>/` prefix, e.g. `minecraft:models/block/stone.json` or
    /// `mymod:machines/furnace.json`. Missing, unreadable and files above
    /// [`MAX_RESOURCE_BYTES`](crate::MAX_RESOURCE_BYTES) return `None`; a client side (or one without that
    /// stack) also returns `None`. This read never reloads packs.
    #[access(PublishedResources)]
    fn resource(&self, pack_type: PackType, identifier: &Identifier) -> Option<Vec<u8>>;
}
