//! The `enchant_power_bonus` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The enchanting power the block `state` at `pos` gives the enchanting table at
    /// `table_pos` (`IBlockExtension.getEnchantPowerBonus`), asked for each of the table's
    /// bookshelf offsets (`EnchantingTableBlock.BOOKSHELF_OFFSETS`) whose block between it and the
    /// table is in `#minecraft:enchantment_power_transmitter`. `power` starts at the block's own
    /// answer, 1.0 in `#minecraft:enchantment_power_provider` and 0.0 for the rest: set 2.0 to
    /// count as two bookshelves, 0.5 for half of one, 0.0 to give nothing. The table adds the
    /// answers, truncates the sum to a whole number as NeoForge does and caps it at 15 for the
    /// offers; a negative sum counts as 0, where NeoForge would throw, and a non-finite answer is
    /// ignored. Asked as the offers are made, in a single-player world and on a LAN host for each
    /// guest's table, with the level read-only. Client particles use the same answer, queried once
    /// per state at world entry, parts reload and received-server-tag publication, with `pos`
    /// and `table_pos` both at the origin. Server tags and their tag-update listeners are
    /// published before the client snapshot. Any nonzero answer emits particles when the
    /// intervening block transmits. Particle answers must be state-only; changing a mod's
    /// answer requires a parts/tag reload.
    /// No hook is called per tick or frame. With no part attached, the only dispatch cost is one
    /// block subscription lookup before event conversion. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        version = 2,
        contract = "collect",
        out = "power",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getEnchantPowerBonus, IBlockStateExtension.getEnchantPowerBonus"
    )]
    fn enchant_power_bonus();
}
