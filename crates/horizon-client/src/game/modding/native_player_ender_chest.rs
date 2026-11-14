//! A server player's ender chest (`Player.getEnderChestInventory`) by slot, for mods that copy
//! or edit it whole: player data sync, `/endersee`.
use super::{Game, commands::items::Slot, native_server::PlayerRead};
use horizon_client::items::ItemStack;
use mod_api::{PlayerDataRefusal, PlayerRef};

/// `PlayerEnderChestContainer`'s size.
const SLOTS: usize = 27;

fn numbered(items: &[Option<ItemStack>]) -> Vec<(u32, ItemStack)> {
    items
        .iter()
        .take(SLOTS)
        .enumerate()
        .filter_map(|(i, s)| Some((i as u32, s.clone().filter(|s| s.count > 0)?)))
        .collect()
}

impl Game<'_> {
    /// `PlayerEnderChestContainer.getItem` for every slot that holds something.
    pub(super) fn player_ender_chest(&self, player: PlayerRef) -> Option<Vec<(u32, ItemStack)>> {
        if self.remote.is_some() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            // An open ender chest menu holds the live copy until it closes.
            PlayerRead::Acting { .. } => match self
                .ender_open
                .and_then(|key| self.world.block_entities.get(&key))
            {
                Some(horizon_client::containers::BlockEntity::Chest(items)) => numbered(items),
                _ => numbered(&self.ender_items),
            },
            PlayerRead::Guest(g) => numbered(&g.ender_items),
        })
    }

    /// `PlayerEnderChestContainer.setItem`, in the player's turn.
    pub(super) fn player_set_ender_chest_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal> {
        self.player_write_gate(player)?;
        if slot as usize >= SLOTS {
            return Err(PlayerDataRefusal::SlotOutOfRange {
                slot,
                last: SLOTS as u32 - 1,
            });
        }
        let mut set = false;
        let ran = self.native_as_player_handle(player, |game| {
            game.journal_player(player.uuid());
            if let Some(key) = game.ender_open {
                game.journal_block_entity(key);
            }
            if game.ender_items.len() < SLOTS {
                game.ender_items.resize(SLOTS, None);
            }
            set = game.slot_set(&Slot::Ender(slot as usize), stack.filter(|s| s.count > 0));
        });
        if ran && set {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }
}
