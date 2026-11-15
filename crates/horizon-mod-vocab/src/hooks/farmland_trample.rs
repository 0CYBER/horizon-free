//! The `farmland_trample` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Called on the server before a landing converts farmland to its base block. It is given the loop `context` of the side that ticks the entity (its registries, settings and mod list); it fires inside the tick's own landing, before the block is written, so it takes no `Host`.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        context = "loop",
        neoforge = "BlockEvent.FarmlandTrampleEvent",
        cancellable = true,
        version = 2,
        target = "horizon_client_lib::landing::native_farmland_trample"
    )]
    fn farmland_trample();
}

/// Original upstream notes; the existing fork event above owns this moment.
pub trait UpstreamNotes {
    #[notes = r###"farmland trampling: `native_trample(dimension, entity, kind, pos, fall_distance, cancel)` is heard when a player or mob that fell onto farmland is about to turn it to dirt, after vanilla's chance and checks (NeoForge's `FarmlandTrampleEvent`, Bukkit's physical `PlayerInteractEvent` and `EntityInteractEvent`). Cancel and the farmland and its crop stay. No `self`. Bundles that use it need `"hooks": @API:trample@`."###]
    fn trample();
}
