use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
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
    /// mob not yet in the world, which Horizon doesn't have yet. On the
    /// server. API 396.
    pub(in crate::game) fn native_mob_split(
        &mut self,
        parent: u32,
        kind: &str,
        size: i32,
        child_size: i32,
        count: &mut u32,
        cancel: &mut bool,
    ) {
        self.fire_mob_split(parent, kind, size, child_size, count, cancel);
    }
}
