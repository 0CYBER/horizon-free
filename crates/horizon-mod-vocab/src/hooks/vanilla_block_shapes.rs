//! The `vanilla_block_shapes` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Entity-context shapes for vanilla blocks, declared in code: push a
    /// `mod_api::VanillaBlockShapes` per group of blocks, the same declaration a
    /// `data/<id>/horizon/vanilla_block_shapes/<name>.json` file holds. Each names
    /// `minecraft:` blocks and, per entity class (`ShapeEntityKind`), the outline or
    /// collision those blocks have for that class: the vanilla shape clipped to a box
    /// (`VanillaShape::Clip`) or a table of boxes by state (`VanillaShape::Table`).
    /// Players then stand on a fence clipped at one block while mobs stay penned.
    /// Asked once at start, on every side, before any resource loads: push the same
    /// ones every time, from nothing but your code. The tables compile once per resource
    /// load, as a mod block's do (docs/modding/src/how-to/content-mods.md). When two mods
    /// declare the same block, class and channel, the mod whose id sorts first keeps it
    /// and the other's is left out with a warning naming both. Declarations that don't
    /// pass the files' checks are logged and that mod's are left out.
    #[hook(
        side = "both",
        contract = "collect",
        out = "shapes",
        answered_by = "mod",
        context = "registration",
        neoforge = "none (NeoForge mods override Block.getCollisionShape in a mixin)"
    )]
    fn vanilla_block_shapes();
}
