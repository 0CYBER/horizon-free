//! The `arrow_loose` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0167

pub trait Hooks {
    /// NeoForge's `ArrowLooseEvent`: the player releases a bow `stack` with
    /// `has_ammo` and `charge` ticks held. Change `charge` or set `canceled`
    /// to prevent the shot; negative charge also prevents it. Unlike
    /// `living_entity_use_item_stop`, this does not skip the whole item
    /// release; `entity_shoot_bow` reports a spawned arrow afterward
    /// with read-only force. Fires on the Game running the bow release, including
    /// LAN guests; a remote client's own release path is not covered. Crossbows
    /// are not covered; `arrow_nock` is the bow's start.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "canceled",
        neoforge = "ArrowLooseEvent",
        cancellable = true
    )]
    fn arrow_loose();
}
