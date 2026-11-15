//! The `mob_split` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A slime, magma cube or sulfur cube is dying and about to split into
    /// smaller ones (NeoForge's `MobSplitEvent`, vanilla's
    /// `AbstractCubeMob.remove`): `parent` is its entity id, `kind` its type,
    /// `size` its size, `child_size` the size of the children (half of it)
    /// and `count` how many vanilla made (2 to 4 for a slime or magma cube,
    /// 2 for a sulfur cube; a primed one makes none and isn't asked). A mod
    /// may change `count` (0 spawns none, at most 64) or set `cancel` to
    /// spawn no children. Only for a parent above size 1, once the child
    /// count is drawn. NeoForge's list of child mobs is this count:
    /// changing a child or adding one of another kind needs a handle to a
    /// mob not yet in the world, which this hook does not expose. On the
    /// server.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "MobSplitEvent",
        paper = "SlimeSplitEvent",
        cancellable = true
    )]
    fn mob_split();
}
