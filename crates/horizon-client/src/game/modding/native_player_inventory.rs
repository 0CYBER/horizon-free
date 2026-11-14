//! A server player's inventory by vanilla's slot numbers (`Inventory.getItem`/`setItem`), for
//! mods that copy or edit it whole: player data sync, `/invsee`.
use super::{Game, commands::items::Slot, native_server::PlayerRead};
use horizon_client::items::ItemStack;
use horizon_items::equipment::{BODY, Equipment, SADDLE};
use mod_api::{PlayerDataRefusal, PlayerRef};

/// `Inventory`'s slot numbers past the 36 main slots (`EQUIPMENT_SLOT_MAPPING`).
const SLOTS: u32 = 43;

/// 0..35 main (0..8 hotbar), 36..39 feet to head, 40 offhand, 41 body, 42 saddle.
/// ItemPlace uses 100..103 for armour and -106 for offhand; InventoryMenu has no 41/42.
/// The slot the game keeps `Inventory` slot `slot` in.
fn inventory_slot(slot: u32) -> Option<Slot> {
    Some(match slot {
        0..36 => Slot::Inv(slot as usize),
        // `armor` is head first: 36 feet is its last.
        36..40 => Slot::Armor((39 - slot) as usize),
        40 => Slot::Offhand,
        41 => Slot::Equipment(BODY),
        42 => Slot::Equipment(SADDLE),
        _ => return None,
    })
}

/// The non-empty slots, by number, of an inventory laid out as the game keeps it.
fn numbered(
    inventory: &[Option<ItemStack>],
    armor: &[Option<ItemStack>; 4],
    offhand: &Option<ItemStack>,
    equipment: &Equipment,
) -> Vec<(u32, ItemStack)> {
    (0..SLOTS)
        .filter_map(|slot| {
            let stack = match inventory_slot(slot)? {
                Slot::Inv(i) => inventory.get(i).cloned().flatten(),
                Slot::Armor(i) => armor[i].clone(),
                Slot::Offhand => offhand.clone(),
                Slot::Equipment(i) => equipment.get(i).cloned(),
                _ => None,
            };
            Some((slot, stack.filter(|s| s.count > 0)?))
        })
        .collect()
}

impl Game<'_> {
    /// `Inventory.getItem` for every slot that holds something.
    pub(super) fn player_inventory_slots(
        &self,
        player: PlayerRef,
    ) -> Option<Vec<(u32, ItemStack)>> {
        if self.remote.is_some() {
            return None;
        }
        Some(match self.player_handle_read(player)? {
            PlayerRead::Acting { .. } => {
                numbered(&self.inventory, &self.armor, &self.offhand, &self.equipment)
            }
            PlayerRead::Guest(g) => numbered(&g.inventory, &g.armor, &g.offhand, &g.equipment),
        })
    }

    /// Why a write to this player's inventory, ender chest, hands or saved data is refused
    /// before it starts: a client, a handle with no online player, a dead player.
    pub(super) fn player_write_gate(&self, player: PlayerRef) -> Result<(), PlayerDataRefusal> {
        if self.remote.is_some() {
            return Err(PlayerDataRefusal::Client);
        }
        match self.player_handle_read(player) {
            None => Err(PlayerDataRefusal::Stale),
            Some(PlayerRead::Guest(g)) if g.dead => Err(PlayerDataRefusal::Dead),
            Some(_) => Ok(()),
        }
    }

    /// `Inventory.setItem`, in the player's turn: the tick's inventory sync tells the client.
    pub(super) fn player_set_inventory_slot(
        &mut self,
        player: PlayerRef,
        slot: u32,
        stack: Option<ItemStack>,
    ) -> Result<(), PlayerDataRefusal> {
        self.player_write_gate(player)?;
        let target = inventory_slot(slot).ok_or(PlayerDataRefusal::SlotOutOfRange {
            slot,
            last: SLOTS - 1,
        })?;
        let mut set = false;
        let ran = self.native_as_player_handle(player, |game| {
            set = game.slot_set(&target, stack.filter(|s| s.count > 0));
            if set && game.player.is_some() {
                game.after_commit(super::transactions::After::InventoryTrigger(player.uuid()));
            }
        });
        if ran && set {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }
}
