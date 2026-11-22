//! The `mod_slot_will_change` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A mod's own equipment slot is about to change: the cancellable moment,
    /// before the mod writes its stack. `holder` is the entity, `slot` the
    /// bare slot id (`ring`), `index` the slot's index and `cosmetic` whether
    /// it is a cosmetic copy. `old` and `new` are the stacks before and after
    /// and `source` is the caller's own hint about why (`equip`, `unequip`,
    /// `reload`, `death` are the usual names; any string is accepted). Set
    /// `cancel` to refuse the change; listeners run by `#[listener(priority)]`.
    /// The mod calls `item::mod_slots::native_mod_slot_will_change` with its
    /// host from its own transaction; the game does not know these slots.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        cancellable = true,
        host = "read",
        neoforge = "none (Curios' SlotChangeEvent.Pre)",
        target = "horizon_client::game::native_hooks::mod_slot_events::Game<'_>::native_mod_slot_will_change"
    )]
    fn mod_slot_will_change(
        holder: EntityRef,
        slot: &str,
        index: usize,
        cosmetic: bool,
        old: Option<&ItemStack>,
        new: Option<&ItemStack>,
        source: &str,
        cancel: &mut bool,
    );
}
