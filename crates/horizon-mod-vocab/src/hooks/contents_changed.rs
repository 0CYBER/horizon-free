//! The `contents_changed` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server-only, non-cancellable backing-block notice at Container.setChanged / BlockEntity.setChanged, after the write and before dirty/comparator updates. Game-held vanilla container transactions coalesce implicit notices at root commit. Explicit mod-issued set_changed notifies immediately, even for equal contents and inside a transaction; the notice survives abort. Raw mod-owned storage writes and restores synthesize no notice. Nested writes to the backing block whose notice is active still apply but do not re-enter its notice; another block notifies normally. Double containers notify each backing component, not the other half merely refreshed for comparators. Vehicle/menu placeholder coordinates are excluded. Replaces the Create port positional contents_changed mixin for depot, package and schematic storage; there is no matching NeoForge bus event.
    #[hook(
        side = "server",
        contract = "listen",
        scope = "block",
        key = "state",
        neoforge = "none (Container.setChanged has no bus event)",
        cancellable = false
    )]
    fn contents_changed();
}
