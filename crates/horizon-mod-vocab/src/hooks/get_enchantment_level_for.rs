//! The `get_enchantment_level_for` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `GetEnchantmentLevelForEvent`: the same enchantment-level read as
    /// `get_enchantment_level`, but only asked when the caller knows who
    /// wears or holds the stack. `owner` is that entity and `mod_slot` the
    /// caller's own place name (`curios:ring`), `None` on a vanilla slot;
    /// `target` and `enchantments` are as in `get_enchantment_level`. A mod
    /// may use them to give a worn item a per-wearer level (Fortune or Looting
    /// on a curio). The game never fires this on its own: a mod calls
    /// `item::enchantments::native_get_enchantment_level_for` with its owner
    /// and place from its own equip moment, so this change alone does not make
    /// per-wearer levels work out of the box. Both sides.
    #[hook(
        side = "both",
        contract = "listen",
        host = "read",
        neoforge = "GetEnchantmentLevelEvent",
        target = "horizon_client::game::native_hooks::get_enchantment_level_for::Game<'_>::native_get_enchantment_level_for"
    )]
    fn get_enchantment_level_for();
}
