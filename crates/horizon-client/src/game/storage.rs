//! Shulker boxes (`ShulkerBoxBlockEntity`: 27 slots that refuse other boxes,
//! kept in the dropped item's `container` component) and ender chests (the
//! player's `PlayerEnderChestContainer`, saved as `EnderItems`).
use super::Game;
use horizon_client::{containers::BlockEntity, items::ItemStack};

pub(super) fn is_shulker_box(name: &str) -> bool {
    name.ends_with("shulker_box")
}
impl Game<'_> {
    /// Block-entity contents on removal: shulker boxes keep theirs for the
    /// dropped box (`copy_components`), other containers spill them.
    pub(super) fn release_contents(&mut self, pos: [i32; 3], entity: BlockEntity, block: &str) {
        if let BlockEntity::Banner(banner) = entity {
            self.pending_banner.insert(pos, banner);
            return;
        }
        if is_shulker_box(block) {
            if let BlockEntity::Chest(items) = entity {
                let kept = items
                    .into_iter()
                    .enumerate()
                    .filter_map(|(i, s)| Some((i as u8, s?)))
                    .collect();
                self.pending_shulker.insert(pos, kept);
            }
            return;
        }
        for stack in entity.contents() {
            self.pop_resource(pos, stack);
        }
    }
    /// The contents a shulker box dropped at `pos` carries.
    pub(super) fn attach_shulker_contents(&mut self, pos: [i32; 3], stack: &mut ItemStack) {
        if !is_shulker_box(&stack.item) {
            return;
        }
        if let Some(contents) = self.pending_shulker.remove(&pos)
            && !contents.is_empty()
        {
            stack.data_mut().container = contents;
        }
    }
    /// A placed shulker box takes the item's contents.
    pub(super) fn fill_placed_box(&mut self, pos: [i32; 3], stack: &ItemStack) {
        let Some(data) = stack.data() else { return };
        if !is_shulker_box(&stack.item) || data.container.is_empty() {
            return;
        }
        if let Some(BlockEntity::Chest(items)) =
            self.world.block_entities.get_mut(&(pos[0], pos[1], pos[2]))
        {
            for (slot, s) in &data.container {
                if let Some(cell) = items.get_mut(usize::from(*slot)) {
                    *cell = Some(s.clone());
                }
            }
        }
    }
    /// ShulkerBoxSlot.mayPlace: no shulker box inside a shulker box.
    pub(super) fn refuses(&self, slot: usize, stack: &ItemStack) -> bool {
        (300..400).contains(&slot)
            && is_shulker_box(&stack.item)
            && self.container.is_some_and(|k| {
                is_shulker_box(&self.res.facts.get(self.world.block(k.0, k.1, k.2)).name)
            })
    }
    /// Opening an ender chest shows the player's ender inventory in its place.
    pub(super) fn open_ender_chest(&mut self, key: (i32, i32, i32)) {
        self.world
            .block_entities
            .insert(key, BlockEntity::Chest(self.ender_items.clone()));
        self.container = Some(key);
        self.ender_open = Some(key);
    }
    /// Closing it copies the items back and leaves no block-entity contents.
    pub(super) fn close_ender_chest(&mut self) {
        let Some(key) = self.ender_open.take() else {
            return;
        };
        if let Some(BlockEntity::Chest(items)) = self.world.block_entities.remove(&key) {
            self.ender_items = items;
        }
    }
}
