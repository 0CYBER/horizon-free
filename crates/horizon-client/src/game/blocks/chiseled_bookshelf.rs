//! Chiseled bookshelves: `ChiseledBookShelfBlock.useItemOn` / `useWithoutItem`
//! (a book into the slot under the crosshair, or out of it) and
//! `ChiseledBookShelfBlockEntity.updateState`, which also follows the books a
//! hopper or dropper moved (`machines::container_changed`). The slots' books
//! show through the block state's `slot_N_occupied`, which reaches a server's
//! clients with the block update.
use super::{Game, Hit, machines::dir_from_name};
use horizon_client::{containers::BlockEntity, items::ItemStack, mobs::Who};
use horizon_core::{
    geometry::{Direction, Vec3},
    position::BlockPos,
};
use horizon_world::level::{flags, hit_slot};

const BLOCK: &str = "minecraft:chiseled_bookshelf";

impl Game<'_> {
    /// The bookshelf's item or empty-hand method, on the captured block state.
    pub(super) fn use_chiseled_bookshelf(
        &mut self,
        hit: &Hit,
        state: horizon_world::registry::BlockStateId,
        held: Option<&ItemStack>,
        branch: horizon_world::level::UseBranch,
    ) -> Option<mod_api::InteractionResult> {
        use mod_api::InteractionResult as Answer;
        let pos = hit.pos;
        let registry = self.pack.assets.registry;
        let info = registry.state(state).filter(|s| s.name == BLOCK)?;
        let item_phase = matches!(branch, horizon_world::level::UseBranch::ItemOn);
        if self.bookshelf_items(pos).is_none() {
            return Some(Answer::Pass);
        }
        let book = held.filter(|s| {
            self.res
                .items()
                .item_tags
                .get("minecraft:bookshelf_books")
                .is_some_and(|m| m.contains(&*s.item))
        });
        if item_phase && book.is_none() {
            return Some(Answer::TryWithEmptyHand);
        }
        let facing = info
            .properties
            .get("facing")
            .and_then(|f| dir_from_name(f))?;
        let face = Direction::ALL
            .into_iter()
            .find(|d| d.normal() == hit.face)?;
        let at = Vec3::new(hit.point[0], hit.point[1], hit.point[2]);
        let Some(slot) = hit_slot(
            facing,
            face,
            BlockPos::new(pos[0], pos[1], pos[2]),
            at,
            2,
            3,
        ) else {
            return Some(Answer::Pass);
        };
        let occupied = info
            .properties
            .get(&format!("slot_{slot}_occupied"))
            .map(String::as_str)
            == Some("true");
        let centre = pos.map(|v| f64::from(v) + 0.5);
        if item_phase {
            if occupied {
                return Some(Answer::TryWithEmptyHand);
            }
            let book = book.unwrap().clone();
            self.award_stat("used", &book.item, 1);
            let sound = if &*book.item == "minecraft:enchanted_book" {
                "minecraft:block.chiseled_bookshelf.insert.enchanted"
            } else {
                "minecraft:block.chiseled_bookshelf.insert"
            };
            self.bookshelf_items(pos)?[slot] = Some(ItemStack {
                count: 1,
                ..book.clone()
            });
            self.consume_held();
            self.chiseled_bookshelf_update_state(pos, slot);
            self.audio.play(sound, Some(centre), 1.0, 1.0);
            return Some(Answer::Success);
        }
        if !occupied {
            return Some(Answer::Consume);
        }
        // removeBook: the book goes to the inventory, or is dropped. A slot the
        // state calls occupied but that holds nothing still plays the pickup
        // and posts `BLOCK_CHANGE`; only `removeItem`'s update needs a book.
        let retrieved = self.bookshelf_items(pos)?[slot].take();
        if retrieved.is_some() {
            self.chiseled_bookshelf_update_state(pos, slot);
        }
        let sound = if retrieved
            .as_ref()
            .is_some_and(|book| &*book.item == "minecraft:enchanted_book")
        {
            "minecraft:block.chiseled_bookshelf.pickup.enchanted"
        } else {
            "minecraft:block.chiseled_bookshelf.pickup"
        };
        self.audio.play(sound, Some(centre), 1.0, 1.0);
        if let Some(book) = retrieved {
            self.give_or_drop(book);
        }
        self.post_block_game_event("minecraft:block_change", pos, Some(Who::Player), None);
        Some(Answer::Success)
    }

    /// The bookshelf's six slots (made if the block has no block entity yet).
    fn bookshelf_items(&mut self, pos: [i32; 3]) -> Option<&mut [Option<ItemStack>; 6]> {
        if &*self
            .res
            .facts()
            .get(self.world.block(pos[0], pos[1], pos[2]))
            .name
            != BLOCK
        {
            return None;
        }
        let entity = self
            .world
            .block_entities
            .get_or_insert_with((pos[0], pos[1], pos[2]), || {
                BlockEntity::ChiseledBookshelf(Box::default())
            });
        match entity {
            BlockEntity::ChiseledBookshelf(b) => Some(&mut b.items),
            _ => None,
        }
    }

    /// `ChiseledBookShelfBlockEntity.updateState`: `slot` becomes the last
    /// one touched, the block state takes the slots' occupancy
    /// (`setBlockAndUpdate`) and `BLOCK_CHANGE` goes out.
    pub(super) fn chiseled_bookshelf_update_state(&mut self, pos: [i32; 3], slot: usize) {
        let key = (pos[0], pos[1], pos[2]);
        self.journal_block_entity(key);
        let Some(BlockEntity::ChiseledBookshelf(b)) = self.world.block_entities.get_mut(&key)
        else {
            return;
        };
        b.last_slot = slot as i32;
        let occupied: Vec<bool> = b.items.iter().map(Option::is_some).collect();
        let registry = self.pack.assets.registry;
        let Some(mut state) = registry
            .state(self.world.block(pos[0], pos[1], pos[2]))
            .cloned()
        else {
            return;
        };
        for (i, occupied) in occupied.into_iter().enumerate() {
            state
                .properties
                .insert(format!("slot_{i}_occupied"), occupied.to_string());
        }
        let Some(new) = registry.id(&state) else {
            return;
        };
        if self.transactions.is_empty() {
            let at = BlockPos::new(pos[0], pos[1], pos[2]);
            self.with_level(|b, level, ticks| {
                b.set_block(level, ticks, at, new, flags::UPDATE_ALL)
            });
        } else {
            self.journal_set_block(pos, new);
        }
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.post_block_game_event("minecraft:block_change", pos, None, Some(new));
    }

    /// The books a hopper or dropper put in or took out: the slot whose
    /// occupancy no longer matches the block state is the one `setItem` or
    /// `removeItem` updated.
    pub(super) fn chiseled_bookshelf_synced(&mut self, pos: (i32, i32, i32)) {
        let Some(BlockEntity::ChiseledBookshelf(b)) = self.world.block_entities.get(&pos) else {
            return;
        };
        let Some(info) = self
            .pack
            .assets
            .registry
            .state(self.world.block(pos.0, pos.1, pos.2))
        else {
            return;
        };
        let moved = (0..6).rev().find(|i| {
            let shown = info
                .properties
                .get(&format!("slot_{i}_occupied"))
                .map(String::as_str);
            b.items[*i].is_some() != (shown == Some("true"))
        });
        if let Some(slot) = moved {
            self.chiseled_bookshelf_update_state([pos.0, pos.1, pos.2], slot);
        }
    }
}

#[cfg(test)]
#[path = "tests/chiseled_bookshelf_tests.rs"]
mod tests;
