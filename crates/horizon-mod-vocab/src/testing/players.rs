//! The players fake: who is online, their inventories, and what the game told them.
use super::FAKE_EPOCH;
use crate::{
    EntityRef, Hand, ItemPlace, ItemStack, LevelRef, PlayerRef, Players, Text, WorldEpoch,
};
use std::collections::BTreeMap;

/// The off hand's inventory slot (`ItemPlace::PlayerSlot`).
const OFF_HAND: i32 = -106;

// no wire: in-process testing harness state, not a mod boundary payload.
/// The players online in a test world, by UUID, and the one whose action is running.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakePlayers {
    players: BTreeMap<u128, FakePlayer>,
    /// The player whose command or action is running (`Players::acting_player`).
    pub acting: Option<u128>,
}

impl FakePlayers {
    /// Player `uuid` online as entity `entity` in `level`, joined if missing.
    pub fn join(&mut self, uuid: u128, entity: u32, level: LevelRef) -> &mut FakePlayer {
        self.players.entry(uuid).or_insert_with(|| FakePlayer {
            entity,
            level,
            name: format!("Player{entity}"),
            slots: BTreeMap::new(),
            selected_slot: 0,
            given: Vec::new(),
            action_bar: Vec::new(),
            infinite_materials: false,
        })
    }
    /// Player `uuid`, if online.
    pub fn get(&self, uuid: u128) -> Option<&FakePlayer> {
        self.players.get(&uuid)
    }
    /// Player `uuid` to change, if online.
    pub fn get_mut(&mut self, uuid: u128) -> Option<&mut FakePlayer> {
        self.players.get_mut(&uuid)
    }

    pub(super) fn handle(&self, uuid: u128) -> Option<PlayerRef> {
        let player = self.players.get(&uuid)?;
        Some(PlayerRef {
            entity: EntityRef::new(WorldEpoch::new(FAKE_EPOCH), player.entity),
            uuid,
        })
    }
    pub(super) fn handles(&self) -> Vec<PlayerRef> {
        self.players
            .keys()
            .filter_map(|&uuid| self.handle(uuid))
            .collect()
    }
    /// The online player who is entity `entity`, and their UUID.
    pub(super) fn by_entity(&self, entity: u32) -> Option<(u128, &FakePlayer)> {
        self.players
            .iter()
            .find(|(_, player)| player.entity == entity)
            .map(|(&uuid, player)| (uuid, player))
    }
    /// The online player `place` belongs to, and the slot it is.
    pub(super) fn place(&mut self, place: &ItemPlace) -> Option<(&mut FakePlayer, i32)> {
        match *place {
            ItemPlace::PlayerSlot { player, slot } => Some((self.players.get_mut(&player)?, slot)),
            ItemPlace::PlayerHand { player, hand } => {
                let player = self.players.get_mut(&player)?;
                let slot = match hand {
                    Hand::Main => player.selected_slot as i32,
                    Hand::Off => OFF_HAND,
                };
                Some((player, slot))
            }
            _ => None,
        }
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// An online player: their entity and level, their name, their inventory by
/// `ItemPlace::PlayerSlot` number, and what the game handed or showed them.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FakePlayer {
    /// Their entity id.
    pub entity: u32,
    /// The level they are in.
    pub level: LevelRef,
    /// Their name, `Player<entity>` until set.
    pub name: String,
    /// Their inventory by slot number: 0..36 (0..9 the hotbar), the armour 100..=103, the off
    /// hand -106.
    pub slots: BTreeMap<i32, ItemStack>,
    /// The hotbar slot their main hand holds.
    pub selected_slot: usize,
    /// What an insert into one of their places could not fit there, in order: the game puts it
    /// into their inventory or drops it.
    pub given: Vec<ItemStack>,
    /// The lines shown above their hotbar, in order.
    pub action_bar: Vec<Text>,
    /// Whether they have infinite materials (`Player.hasInfiniteMaterials`, creative's
    /// `instabuild`).
    pub infinite_materials: bool,
}

impl FakePlayer {
    /// The stack in their main hand.
    pub fn main_hand(&self) -> Option<&ItemStack> {
        self.slots.get(&(self.selected_slot as i32))
    }

    /// Puts up to `amount` into slot `slot`, up to `max_stack`; the rest is given to them.
    /// How much went: all of it. The give log records transfer remainders, not resident slots.
    pub(super) fn insert(
        &mut self,
        slot: i32,
        resource: &crate::ItemResource,
        amount: u32,
        max_stack: u32,
    ) -> u32 {
        let held = self.slots.get(&slot);
        let room = match held {
            Some(held) if !resource.matches(held) => 0,
            Some(held) => max_stack.saturating_sub(held.count()),
            None => max_stack,
        };
        let put = room.min(amount);
        if put > 0 {
            let count = held.map_or(0, ItemStack::count) + put;
            self.slots
                .extend(resource.to_stack(count).map(|stack| (slot, stack)));
        }
        self.given.extend(resource.to_stack(amount - put));
        amount
    }
    /// Takes up to `amount` of this resource out of slot `slot`; how many came.
    pub(super) fn extract(
        &mut self,
        slot: i32,
        resource: &crate::ItemResource,
        amount: u32,
    ) -> u32 {
        let std::collections::btree_map::Entry::Occupied(entry) = self.slots.entry(slot) else {
            return 0;
        };
        if !resource.matches(entry.get()) {
            return 0;
        }
        let (slot, stack) = entry.remove_entry();
        let split = stack.split(amount);
        self.slots
            .extend(split.remainder.map(|stack| (slot, stack)));
        split.taken.as_ref().map_or(0, ItemStack::count)
    }
}

impl Players for FakePlayers {
    fn server_players(&self) -> Vec<PlayerRef> {
        self.handles()
    }
    fn acting_player(&self) -> Option<PlayerRef> {
        self.handle(self.acting?)
    }
}
