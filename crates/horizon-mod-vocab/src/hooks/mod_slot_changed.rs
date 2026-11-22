//! The `mod_slot_changed` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mod's own equipment slot has changed: the committed notice, after
    /// the mod wrote its stack. The fields are `mod_slot_will_change`'s
    /// without `cancel`: `holder`, `slot`, `index`, `cosmetic`, `old`, `new`
    /// and `source`, the caller's own hint about why (`equip`, `unequip`,
    /// `reload`, `death` are the usual names; any string is accepted). The mod calls
    /// `item::mod_slots::native_mod_slot_changed` with its host once its own
    /// write is committed. Server-side; clients get the stacks from the mod's
    /// own sync payload.
    #[hook(
        side = "server",
        contract = "listen",
        host = "read",
        neoforge = "none (Curios' SlotChangeEvent)",
        target = "horizon_client::game::native_hooks::mod_slot_events::Game<'_>::native_mod_slot_changed"
    )]
    fn mod_slot_changed(
        holder: EntityRef,
        slot: &str,
        index: usize,
        cosmetic: bool,
        old: Option<&ItemStack>,
        new: Option<&ItemStack>,
        source: &str,
    );
}
