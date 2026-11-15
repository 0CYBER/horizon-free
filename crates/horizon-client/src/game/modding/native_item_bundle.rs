//! Bundle-backed ITEM_ITEMS operations; the place handler owns transactional writes.
use super::Game;
use horizon_client::items::ItemStack;
use horizon_client::menu::container_items::{can_be_in_bundle, fits_inside_container_items};
use horizon_items::bundle::Contents as Bundle;

impl Game<'_> {
    pub(in crate::game) fn item_bundle_slots(
        &self,
        stack: &ItemStack,
    ) -> Option<Vec<Option<ItemStack>>> {
        if stack.count == 0 {
            return None;
        }
        if !stack.carries_component(
            self.res.items().get(&stack.item),
            "minecraft:bundle_contents",
        ) {
            return None;
        }
        Some(
            stack
                .data()
                .map_or(&[][..], |data| &data.bundle_contents[..])
                .iter()
                .cloned()
                .map(Some)
                .collect(),
        )
    }
    pub(in crate::game) fn item_bundle_capacity(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> u32 {
        if slot != 0 || !can_be_in_bundle(&self.loop_context, item) {
            return 0;
        }
        Bundle::room_in_stack(self.res.items(), stack, item).unwrap_or(0)
    }
    pub(in crate::game) fn item_bundle_insert_slot(
        &self,
        stack: &ItemStack,
        slot: usize,
        item: &ItemStack,
    ) -> Option<(u32, ItemStack)> {
        let mut bundle = Bundle::of(self.res.items(), stack)?;
        if stack.count == 0 {
            return None;
        }
        if slot != 0 {
            return Some((0, stack.clone()));
        }
        let fits = fits_inside_container_items(&self.loop_context, item);
        let moved = bundle.try_insert(self.res.items(), &mut item.clone(), fits);
        let mut changed = stack.clone();
        changed.count = 1;
        bundle.store(&mut changed);
        changed.data_mut().wire = Default::default();
        changed.tidy();
        Some((moved, changed))
    }
    pub(in crate::game) fn item_bundle_extract(
        &self,
        stack: &ItemStack,
        slot: usize,
        max: u32,
    ) -> Option<(Option<ItemStack>, ItemStack)> {
        if !stack.carries_component(
            self.res.items().get(&stack.item),
            "minecraft:bundle_contents",
        ) {
            return None;
        }
        if stack.count == 0 {
            return None;
        }
        if slot != 0 || max == 0 {
            return Some((None, stack.clone()));
        }
        // Selection belongs to the UI. Automation always takes from the front.
        let mut changed = stack.clone();
        changed.count = 1;
        changed.data_mut().bundle_selected = horizon_items::item_data::BundleSelected(-1);
        let mut bundle = Bundle::of(self.res.items(), &changed)?;
        let Some(mut out) = bundle.remove_one(self.res.items()) else {
            return Some((None, changed));
        };
        let take = out.count.min(max);
        bundle.store(&mut changed);
        if take < out.count {
            let mut rest = out.clone();
            rest.count -= take;
            // Preserve the remainder even for command-created overfull contents.
            changed.data_mut().bundle_contents.insert(0, rest);
        }
        out.count = take;
        changed.data_mut().wire = Default::default();
        changed.tidy();
        Some((Some(out), changed))
    }
}
