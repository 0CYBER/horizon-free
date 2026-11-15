//! Whole stacks for server mods: one given to a player as `/give` gives it
//! (`player_give_stack`), and one read from SNBT as a saved item is (`item_from_snbt`).
use super::Game;
use horizon_client::{items::ItemStack, lan::playerdata};
use mod_api::{PlayerDataRefusal, PlayerRef};

/// The most SNBT text `item_from_snbt` reads: `NbtAccounter`'s default quota, 2 MiB.
const MAX_SNBT_BYTES: usize = 2 * 1024 * 1024;

impl Game<'_> {
    /// `GiveCommand.giveItem` for one stack: `Inventory.add`, the rest thrown for the player
    /// alone to pick up, in the player's turn.
    pub(super) fn player_give_stack(
        &mut self,
        player: PlayerRef,
        stack: ItemStack,
    ) -> Result<(), PlayerDataRefusal> {
        self.player_write_gate(player)?;
        let owner = Some(playerdata::uuid_ints(player.uuid()));
        let ran = self.native_as_player_handle(player, |game| {
            game.give_acting_stack(stack, owner);
            // containerMenu.broadcastChanges: InventoryChangeTrigger.
            if game.player.is_some() {
                game.after_commit(super::transactions::After::InventoryTrigger(player.uuid()));
            }
        });
        if ran {
            Ok(())
        } else {
            Err(PlayerDataRefusal::Stale)
        }
    }

    /// `ItemStack.CODEC` reading the compound `snbt` stands for (`TagParser`): only that
    /// compound and whitespace. At most 2 MiB and 512 levels: vanilla's SNBT parser has
    /// neither limit (deep text ends in a stack overflow); they are this API's bound so
    /// mod-supplied text cannot overflow the parser.
    pub(super) fn item_from_snbt(&self, snbt: &str) -> Option<ItemStack> {
        if snbt.len() > MAX_SNBT_BYTES {
            return None;
        }
        let (tag, end) = horizon_core::nbt::parse_snbt(snbt).ok()?;
        if !snbt.as_bytes()[end..].iter().all(u8::is_ascii_whitespace) {
            return None;
        }
        // `ItemStack.CODEC`'s `count` is 1 to 99 and is refused outside it: a stack read here
        // is never a different count from the one written.
        if let Some(count) = tag.get("count")
            && horizon_client_core::nbt_value::number(Some(count))
                .is_none_or(|n| !(1..=99).contains(&n))
        {
            return None;
        }
        let stack = horizon_items::item_nbt::item_from_nbt(&tag)?;
        self.items().get(&stack.item).is_some().then_some(stack)
    }
}
