//! Storage that is not a block entity behind a mod menu's slots (arismoko/horizon#3385):
//! NeoForge 26.3's `StackCopySlot` (a stack held inside another item) and `ResourceHandlerSlot`
//! (an index of an item handler). A mod opens its menu type with `PlayerRef::open_mod_menu`
//! (arismoko/horizon#3384) and then backs its container slots with [`PlayerRef::back_mod_menu_slots`]; a click
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
    /// closes once the hand holds no such container, and the player's inventory
    /// slot that holds it takes no click while the menu is open, so the stack can't go into
    /// itself. The stack is bound when the slots are backed: the menu closes, before the next
    /// click or at the next change notice, once that inventory slot no longer holds it (another
    /// stack put in its place, the selected hotbar slot changed).
    HeldItem {
        /// The hand holding the container.
        hand: Hand,
        /// The slot's index in the container.
        slot: usize,
    },
    /// Slot `slot` of `entity`'s [`ENTITY_ITEMS`] handler, as a mod's provider answers it for that
    /// entity (`ResourceHandlerSlot`). The menu closes once the entity is gone, found before the
    /// next click or at the next change notice.
    EntityItems {
        /// The entity holding the inventory.
        entity: EntityRef,
        /// The slot's index in the handler.
        slot: usize,
    },
    /// Slot `slot` of the container inside the stack at the mod place
    /// [`ItemPlace::Mod { provider, key }`](ItemPlace::Mod), through that stack's [`ITEM_ITEMS`]
    /// handler: a backpack worn in an accessory slot (`StackCopySlot` over a mod's
    /// `ItemAccess`). The stack is bound when the slots are backed, as [`HeldItem`](Self::HeldItem)'s:
    /// the menu closes, before the next click or at the next change notice, once the place holds
    /// no container or another stack. A click that would take the stack out of its place (the
    /// place is one of the menu's slots, such as a provider that keeps it in the player's
    /// inventory) is undone whole, so the stack can't go into itself.
    ModItem {
        /// The place provider's registered name.
        provider: &'static str,
        /// The provider's own key for the place.
        key: u64,
        /// The slot's index in the container.
        slot: usize,
    },
}

/// Why [`PlayerRef::back_mod_menu_slots`] backed nothing; the menu and its slots stay as they
/// were.
#[non_exhaustive]
#[derive(Clone, Copy, Debug, PartialEq, Eq, horizon_mod_api_macros::Wire)]
pub enum MenuSlotBackRefusal {
    /// The player is not online here (also a host with no game behind it), or `window` is not
    /// the menu they have open.
    NoPlayerOrWindow,
    /// The open menu is not a mod menu opened with no block (its slots are a block entity's,
    /// an entity's or the game's own), so its slots cannot show another storage.
    UnsupportedMenu,
    /// More places than the menu has container slots.
    TooManyPlaces,
    /// `places[index]` names no storage now (no container in that hand or at that mod place,
    /// the entity gone or without an item handler, a slot past the handler's size).
    PlaceUnresolved {
        /// The place's index in `places`.
        index: usize,
    },
}

impl crate::NoBackend for MenuSlotBackRefusal {
    /// No game behind the host: no player is online there.
    fn no_backend() -> Self {
        Self::NoPlayerOrWindow
    }
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
                // The inventory slot holding the hand's stack (`forPlayerSlot`), not the hand
                // as an interaction reaches it (`forPlayerInteraction`): a creative player's
                // hand would keep the old stack and take the rewritten one as another.
                let index = match hand {
                    Hand::Off => -106,
                    _ => i32::try_from(player.selected_slot(host)?).ok()?,
                };
                let held = ItemPlace::PlayerSlot {
                    player: player.uuid(),
                    slot: index,
                };
                Some((held.capability(host, &ITEM_ITEMS)?, slot))
            }
            Self::EntityItems { entity, slot } => {
                Some((entity.capability(host, &ENTITY_ITEMS, ())?, slot))
            }
            Self::ModItem {
                provider,
                key,
                slot,
            } => {
                let place = ItemPlace::Mod { provider, key };
                Some((place.capability(host, &ITEM_ITEMS)?, slot))
            }
        }
    }

    /// Whether this place still names storage, for the menu of `player`: false closes the menu.
    pub fn resolves(self, host: &mut dyn ServerHost, player: PlayerRef) -> bool {
        self.handler(host, player)
            .is_some_and(|(handler, slot)| slot < handler.size(host))
    }
}
