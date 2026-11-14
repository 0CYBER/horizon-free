//! Storage that is not a block entity behind a mod menu's slots (arismoko/horizon#3385):
//! NeoForge 26.3's `StackCopySlot` (a stack held inside another item) and `ResourceHandlerSlot`
//! (an index of an item handler). A mod opens its menu type with `PlayerRef::open_mod_menu`
//! (#3384) and then backs its container slots with [`PlayerRef::back_mod_menu_slots`]; a click
//! or shift-click on such a slot, and an [`ItemPlace::MenuSlot`] write to it, go through that
//! storage's own handler in one journalled transaction, as a block entity's slots do.
use crate::{
    Capability, ENTITY_ITEMS, EntityRef, Hand, ITEM_ITEMS, ItemPlace, ItemResource, PlayerRef,
    ResourceHandler, ServerHost,
};

/// The storage one container slot of a mod menu opened with no block shows
/// ([`MenuSlotBacking::Place`](crate::MenuSlotBacking::Place)).
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum MenuSlotPlace {
    /// Slot `slot` of the container inside the stack the menu's player holds in `hand`, through
    /// that stack's [`ITEM_ITEMS`] handler: a backpack's contents (`StackCopySlot`). The menu
    /// closes at the next tick once the hand holds no such container, and the player's inventory
    /// slot that holds it takes no click while the menu is open, so the stack can't go into
    /// itself.
    HeldItem {
        /// The hand holding the container.
        hand: Hand,
        /// The slot's index in the container.
        slot: usize,
    },
    /// Slot `slot` of `entity`'s [`ENTITY_ITEMS`] handler, as a mod's provider answers it for that
    /// entity (`ResourceHandlerSlot`). The menu closes at the next tick once the entity is gone.
    EntityItems {
        /// The entity holding the inventory.
        entity: EntityRef,
        /// The slot's index in the handler.
        slot: usize,
    },
}

impl MenuSlotPlace {
    /// The handler this place reads and writes, and the index in it, for the menu of `player`;
    /// None once it no longer resolves. It builds no [`ItemPlace`] on the way.
    pub(crate) fn handler(
        self,
        host: &mut dyn ServerHost,
        player: PlayerRef,
    ) -> Option<(Capability<dyn ResourceHandler<ItemResource>>, usize)> {
        match self {
            Self::HeldItem { hand, slot } => {
                let held = ItemPlace::PlayerHand {
                    player: player.uuid(),
                    hand,
                };
                Some((held.capability(host, &ITEM_ITEMS)?, slot))
            }
            Self::EntityItems { entity, slot } => {
                Some((entity.capability(host, &ENTITY_ITEMS, ())?, slot))
            }
        }
    }

    /// Whether this place still names storage, for the menu of `player`: false closes the menu.
    pub fn resolves(self, host: &mut dyn ServerHost, player: PlayerRef) -> bool {
        self.handler(host, player)
            .is_some_and(|(handler, slot)| slot < handler.size(host))
    }
}
