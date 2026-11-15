//! Physical identity of logical menu slots, including linked storage.
use super::*;
impl Slots {
    /// The actual storage of a logical menu slot. Computed output has no
    /// physical backing; a linked slot belongs to its linked container.
    pub fn logical_backing(self, world: &World, guest: &Guest, n: usize) -> Option<SlotBacking> {
        if guest
            .menu
            .as_ref()
            .filter(|menu| menu.slots == self)
            .and_then(|menu| menu.mod_type?.crafting)
            .is_some_and(|grid| grid.result == n)
        {
            return None;
        }
        if let Some((range, links)) = self.links_of(guest)
            && range.contains(&n)
        {
            let mut offset = n - range.start;
            for &pos in links {
                let count = shown(world, pos).map_or(0, <[Option<ItemStack>]>::len);
                if offset < count {
                    return Some(SlotBacking::Block(pos, offset));
                }
                offset -= count;
            }
            return None;
        }
        self.backing(world, n)
    }
}
