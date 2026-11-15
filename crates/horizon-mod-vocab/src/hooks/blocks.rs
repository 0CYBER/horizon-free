//! The `blocks` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0152

pub trait Hooks {
    /// Blocks this mod adds, declared in code rather than one file each
    /// (NeoForge's `DeferredRegister.Blocks` drained by `RegisterEvent`):
    /// push a `mod_api::Block` per block, the same declaration
    /// a content mod's `data/<id>/horizon/block/<path>.json` holds (a
    /// vanilla `template`, `properties`, `state_properties`, `shapes`...),
    /// with its id in a namespace owned by your mod's manifest. It has a block
    /// item, as a file's does, unless `item` is false (`Block::with_item(false)`).
    /// They join the content mods' blocks: the one whose
    /// id is that namespace when it is on (your models, textures, loot and
    /// recipes are its files), else one of their own, with the template's
    /// look. Ids, saves and the check that a joining client has the same
    /// blocks are the files' (docs/modding/src/how-to/content-mods.md). Asked once at start, on every
    /// side, before any resource loads: push the same ones every time, from
    /// nothing but your code. Unowned namespaces refuse registration, naming the mod
    /// and namespace. Each mod receives its own declaration batch. Declarations
    /// that don't pass the other files' checks
    /// are logged and that namespace's are left out.
    #[notes = r#"## Villager farming

Farmers recognize mod blocks made from farmland and `CropBlock` templates, including declarations from
`blocks`. A crop block's generated item is its seed: add that item to
`minecraft:villager_plantable_seeds` to let farmers plant its default block state from their inventory.
Untagged items and tagged items without a block are skipped. Crop maturity follows the template's class:
beetroots mature at age 3, ordinary crops at 7; torchflower crops never reach their maximum age 2 while
still crops. Pitcher crops are not `CropBlock`s and are not harvested by this behavior. Farming still
requires a farmer at work and `mob_griefing` enabled."#]
    #[hook(
        side = "both",
        contract = "collect",
        out = "blocks",
        context = "registration",
        neoforge = "RegisterEvent"
    )]
    fn blocks();
}
