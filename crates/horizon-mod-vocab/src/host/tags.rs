//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The host's methods both sides share of the `tags` area, implemented by the game and reached through [`Host`].
#[horizon_mod_api_macros::host_area]
pub trait TagsHost {
    /// The merged-tag service, using load-resolved item and tag keys.
    /// Not on wasm: a frame resource; its methods are the imports.
    #[access(Metadata)]
    #[service]
    fn tags(&self) -> &dyn Tags;
    /// Side: both.
    /// The saved name for an engine-issued unresolved fluid.
    #[access(WorldOnly)]
    fn unresolved_fluid_id(&self, fluid: UnresolvedFluid) -> String;
    /// Side: both.
    /// Whether the resolved block is in the world's merged block tag, on a client the tag its
    /// server sent. Unknown tags return false; unavailable tags or owning-thread block mappings
    /// return None.
    #[access(Metadata)]
    fn block_in_tag(&self, block: BlockKey, tag: &str) -> Option<bool>;
    /// Side: both.
    /// The outline of `state` as `BlockState.getShape` answers it with an empty getter at the
    /// origin (`EmptyBlockGetter.INSTANCE`, `BlockPos.ZERO`): its boxes, each `[min x, y, z,
    /// max x, y, z]` in the block's own space, with no random offset and none of a world's
    /// context (a state that reads its neighbours or block entity answers what its state alone
    /// gives). Empty for a state with no outline, such as air. None for an id no state has.
    #[access(Metadata)]
    fn state_outline(&self, state: BlockState) -> Option<Vec<[f64; 6]>>;
    /// Side: both.
    /// The fluid state `state` holds (`BlockState.getFluidState`): water for a waterlogged state,
    /// the empty fluid for most. None for an id no state has.
    #[access(Metadata)]
    fn state_fluid_state(&self, state: BlockState) -> Option<FluidState>;
    /// Side: both.
    /// Whether `fluid` is in the fluid tag `tag` (`FluidState.is(TagKey)`, `minecraft:water` or
    /// `water`), over the merged fluid tags, on a client those its server sent. An unknown tag
    /// is false. None as for `native_block_in_tag`.
    #[access(Metadata)]
    fn fluid_in_tag(&self, fluid: &str, tag: &str) -> Option<bool>;
    /// Side: both.
    /// Whether `fluid` and `other` are the same fluid (`Fluid.isSame`): a source and its
    /// flowing form are, so `minecraft:water` and `minecraft:flowing_water` are the same, water
    /// and lava not. Names are fluid ids, with or without `minecraft:`; false when either
    /// names no fluid.
    #[access(Metadata)]
    fn fluid_is_same(&self, fluid: &str, other: &str) -> bool;
    /// Side: both.
    /// The bucket item that picks `fluid` up (`Fluid.getBucket`), `minecraft:water_bucket` for
    /// water and flowing water. None for `minecraft:empty`, whose bucket is air, and for a name
    /// no fluid has.
    #[access(Metadata)]
    fn fluid_bucket(&self, fluid: &str) -> Option<ItemKey>;
    /// Side: both.
    /// The sound event a bucket plays as it picks `fluid` up (`Fluid.getPickupSound`),
    /// `minecraft:item.bucket.fill_lava` for lava and flowing lava. None for
    /// `minecraft:empty`, which has none, and for a name no fluid has.
    #[access(Metadata)]
    fn fluid_pickup_sound(&self, fluid: &str) -> Option<&'static str>;
    /// Side: both.
    /// The sound event a bucket plays as it empties `fluid` (`BucketItem.playEmptySound`, the
    /// `BUCKET_EMPTY` sound of NeoForge's `FluidType`), `minecraft:item.bucket.empty_lava` for
    /// lava and flowing lava and `minecraft:item.bucket.empty` for water and a mod's world
    /// fluid. None for `minecraft:empty`, for a tank-only fluid, which has no world form, and
    /// for a name no fluid has.
    #[access(Metadata)]
    fn fluid_empty_sound(&self, fluid: &str) -> Option<&'static str>;
}
