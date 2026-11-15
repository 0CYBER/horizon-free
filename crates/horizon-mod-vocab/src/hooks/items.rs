//! The `items` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0153

pub trait Hooks {
    /// Items this mod adds, declared in code (NeoForge's
    /// `DeferredRegister.Items`), as `blocks` declares blocks: a
    /// `mod_api::Item` per item, a content mod's
    /// `data/<id>/horizon/item/<path>.json` (a vanilla `template`,
    /// `max_stack_size`, `rarity`, `components`, `crafting_remainder`). A
    /// block's item comes with its block (`item: true`), not from here.
    /// Each mod receives its own batch; IDs must use a namespace owned by its
    /// manifest (bare paths use its mod ID). Unowned namespaces refuse registration.
    /// Asked once at start, on every side.
    #[hook(
        side = "both",
        contract = "collect",
        out = "items",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn items();
}
