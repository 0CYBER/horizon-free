//! The `armor_stand_manipulate` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0239

pub trait Hooks {
    /// A player takes an item from an armor stand or puts one on it
    /// (`ArmorStand.interactAt`, after the stand's disabled-slot checks and
    /// `swapItem`'s take/put locks, before anything moves): `stand_id` is the
    /// stand's `Entity.getId` and `slot` the one the click reached
    /// (`mainhand`, `offhand`, `feet`, `legs`, `chest` or `head`). Set
    /// `cancel` and nothing changes: neither the stand nor the player's hand,
    /// and the click is spent. The player's state is swapped in for the call;
    /// `player` is the UUID the helpers take. Not told of a click that changes
    /// nothing (an empty hand on an empty slot). Not covered, other ways a
    /// stand's equipment moves: breaking or blasting the stand (its items
    /// drop), a dispenser putting armour on one, a mod's own changes.
    /// Server-side, for every player of the world.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "none (Bukkit's PlayerArmorStandManipulateEvent)",
        cancellable = true
    )]
    fn armor_stand_manipulate();
}
