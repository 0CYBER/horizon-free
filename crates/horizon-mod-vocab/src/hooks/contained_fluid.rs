//! The `contained_fluid` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The fluid a stack of a mod's own container item holds, as its
    /// `neoforge:fluid_container` item model draws it
    /// (`FluidUtil.getFirstStackContained`): `fluid` arrives `None` (holds
    /// none); set it to the fluid's key, taken once at registration with
    /// `r.fluid_key`. Asked on the client each time a stack of an item outside
    /// `minecraft:` is drawn through a fluid container model; vanilla's items
    /// keep their own answer and never ask. A fluid with no look in an item
    /// draws the missing texture; a key no fluid is registered under draws the
    /// stack as holding none, and the game logs it once per item.
    /// Answer from the stack alone, and fast.
    #[notes = r##"### What a mod's container item holds

`contained_fluid` is the client's read of NeoForge's `FluidUtil.getFirstStackContained` for a mod's own item: an
`ItemPart` attached to a container item (a tank, a canister, a modded bucket) says which registered fluid a stack
holds, and the item's `neoforge:fluid_container` model draws it as it draws a water bucket's: the fluid's still
sprite through the model's `fluid` mask, tinted as a stack holding that fluid, under the cover. Leaving
`event.fluid` at `None` draws the stack as holding none: the model's declared fluid untinted, or no fluid layer
when that is `minecraft:empty`.

The answer is a `FluidKey`, never a sprite or a colour: take it once in `register` (`r.fluid_key("minecraft:water")`,
or the key `declare_fluid` returned) and keep it in the part. The stack is an owned snapshot, so a part reads its
own component (an amount, the fluid it was filled with) from `event.stack`. Only items outside `minecraft:` ask:
a part attached to a vanilla item by key or tag is never asked, and vanilla's buckets keep their closed answer.

It is asked once per draw of a stack whose model is a fluid container, so at most once per stack per frame, and
only for an item whose parts subscribe: that check is a lookup by the item's key, and the stack snapshot is built
only after it. A registered fluid the item renderer has no look for (a tank-only fluid with no `horizon/fluid`
data) draws the missing texture through the mask, so the mistake shows. A key no fluid is registered under draws
the stack as holding none and is logged once for that item. The hook is
client-only; the server keeps the fluid in the stack's own data, through `ITEM_FLUIDS` and the item's
components, as before."##]
    #[hook(
        side = "client",
        contract = "listen",
        scope = "item",
        key = "stack",
        neoforge = "none (FluidUtil.getFirstStackContained over Capabilities.Fluid.ITEM, read by DynamicFluidContainerModel; Horizon's ITEM_FLUIDS needs a server place, so the client model asks the item)"
    )]
    fn contained_fluid();
}
