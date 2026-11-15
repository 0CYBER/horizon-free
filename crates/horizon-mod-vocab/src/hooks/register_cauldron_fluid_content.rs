//! Register the fluid a mod's cauldron block holds.
pub trait Hooks {
    /// Say which fluid, and how much, one of your cauldron blocks holds (NeoForge
    /// `RegisterCauldronFluidContentEvent`), so the fluid capability (`FLUIDS`) fills and
    /// drains it as it does vanilla's water and lava cauldrons. Asked once the block registry
    /// is resolved, before any world opens, and again on a resource reload. Give
    /// `event.contents.register(block, fluid, total, level)` the block and fluid keys your
    /// `register` hook requested, the millibuckets the cauldron holds when full, and the name
    /// of its integer level property, or none for a cauldron only ever full. The engine
    /// refuses, with one logged warning naming both sides, a block that already has a content,
    /// a fluid that already has a cauldron, an unknown block or fluid, a level property the
    /// block lacks or that isn't an integer, and a total of 0 or one that doesn't divide
    /// evenly by the levels; the rest of a mod's contents still apply.
    #[notes = r#"## Cauldron fluid content

`register_cauldron_fluid_content` runs once the block registry is resolved, before any world
opens, and again when resources reload. It collects a `CauldronFluidContents` registrar:
`register(block, fluid, total, level)` says that `block` holds `total` millibuckets of `fluid`
when full, over the levels of its integer property `level` (`"level"` for a block built like
`LayeredCauldronBlock`), or only ever full when `level` is none, as a lava cauldron. The block
and the fluid are the keys `register` requested (`block_key`, `fluid_key`, `declare_fluid`).

Vanilla's water (`minecraft:water_cauldron`, 1000 mB over `level` 1..3) and lava
(`minecraft:lava_cauldron`, 1000 mB, always full) are built into the same table, so the fluid
capability answers every cauldron that has a content alike: each level holds `total` ÷ the
number of levels, a fill adds whole levels and turns an empty `minecraft:cauldron` into the
content's block at the level the amount reaches, and a drain takes whole levels and turns the
last one back into `minecraft:cauldron`. Powder snow is no fluid, and its cauldron has no
content.

The engine refuses, with one logged warning naming both sides, a block that already has a
content (vanilla's or an earlier mod's), a fluid that already has a cauldron (one content per
fluid, as NeoForge keeps), an unknown block or fluid, a level property the block doesn't have
or that isn't an integer property, and a total of 0 or one that doesn't divide evenly by the
number of levels. The rest of a mod's contents still apply. What a cauldron does when an item
is used on it is not part of this hook."#]
    #[hook(
        side = "server",
        contract = "collect",
        out = "contents",
        context = "registration",
        neoforge = "RegisterCauldronFluidContentEvent"
    )]
    fn register_cauldron_fluid_content();
}
