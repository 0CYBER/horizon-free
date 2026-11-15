//! The `enchanted_block_loot` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `EnchantedBlockLootEvent`: a block's loot table is reading the level of `enchantment` on `tool` to decide what `state` at `pos` drops, in `ApplyBonusCount.run` or `BonusLevelTableCondition.test`. `level` starts as the tool's level, after `get_enchantment_level`; what is left in it is what that one count or condition uses, and the tool keeps its own. A `u32` can't be negative, the case `setEnchantmentLevel` refuses; one above 1024 counts as 1024, as vanilla's levels stop at 255 and a level sizes a formula's loop. `tool` is none for an empty hand or a break with no tool, and the read is still asked, at level 0. `entity` is `LootContextParams.THIS_ENTITY`: the player mining, the mob breaking the block, the blast's direct source (primed TNT, TNT cart, exploding mob, fireball, wither skull, end crystal), the enderman whose carried block drops, the source of `/loot ... mine`, the projectile that broke a chorus flower or a decorated pot, the villager harvesting, the ravager walking through a crop, or none for a break with no entity (a neighbour update, a piston, water, a trident through a speleothem). A boat through a lily pad also gives none, though vanilla names it. Asked once per read, so a table may ask several times per break, or not at all, and the table's draws stay in vanilla's order. It runs inside the roll, so it gets the read-only level host: no writes, no draws, and `entity` is a handle a listener can keep and compare but not read until it has a `Host`. Authoritative servers only. Use `get_enchantment_level` when the block doesn't matter. Tick-time effect: one `listened` test per roll when no mod listens.
    #[notes = r#"## Enchanted block loot

- **`enchanted_block_loot(level, dimension, pos, state, tool, enchantment, level, entity)`** (NeoForge's `EnchantedBlockLootEvent`): a block's loot table reads the level of `enchantment` on `tool`, at `ApplyBonusCount.run` or `BonusLevelTableCondition.test`. Change `level` and that one count or condition uses it: Fortune-style mods change it per block, breaker or dimension, without touching the stack. `level` starts after `get_enchantment_level`. `tool` is none for an empty hand or a toolless break, which is still asked at level 0. `entity` (`LootContextParams.THIS_ENTITY`) is the player mining, the breaking mob, the blast's direct source (primed TNT, TNT cart, exploding mob, fireball, wither skull, end crystal), the enderman dropping its carried block, the source of `/loot ... mine`, the projectile breaking a chorus flower or a decorated pot, the villager harvesting, the ravager in a crop, or none (a neighbour update, a piston, water, a trident through a speleothem, whose `destroyBlock` names nobody; also a boat through a lily pad, though vanilla names it). A `u32` level can't be negative, as `setEnchantmentLevel` refuses; above 1024 it counts as 1024, so a mod's value can't size a loop. A table reaches it once per read, in the roll's order, so a break can ask several times or never. Only block tables ask: chests, fishing, entity tables and `block_interact` tables do not. It runs in the roll with the read-only level host, so a listener can't move the table's random; the level random and the sequences are the roll's until it ends. The host can't read `entity`: a break can come from inside a level write, where the game is split up and there is no `Host`, so `getEntity`'s entity is a handle here, to keep or compare. Authoritative servers only. Tick-time effect: one `listened` test per roll with no listener; with one, each roll builds the level host."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "EnchantedBlockLootEvent",
        cancellable = false,
        target = "horizon_client::game::native_hooks::enchanted_block_loot::GameLevel<'_>::native_enchanted_block_loot"
    )]
    fn enchanted_block_loot(
        dimension: LevelDimension,
        pos: BlockPos,
        state: BlockState,
        tool: Option<&ItemStack>,
        enchantment: &Identifier,
        level: &mut u32,
        entity: Option<EntityRef>,
    );
}
