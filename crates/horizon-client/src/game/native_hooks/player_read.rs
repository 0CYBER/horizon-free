use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// `Inventory.items`: the non-empty main inventory stacks in slot order, including
    /// their components. None for a stale or unavailable player; on clients only the
    /// local player's inventory is available.
    pub(in crate::game) fn player_inventory(&self, player: PlayerRef) -> Option<Vec<ItemStack>> {
        let inventory = match self.player_read_both(player)? {
            crate::game::native_server::PlayerRead::Acting { .. } => &self.inventory,
            crate::game::native_server::PlayerRead::Guest(guest) => &guest.inventory,
        };
        Some(inventory.iter().flatten().cloned().collect())
    }
}
