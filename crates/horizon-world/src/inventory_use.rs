//! Living item-use lifecycle bound to player inventory, shared stack identity
//! and synchronized using/offhand bytes. Item effects and level services remain
//! explicit so their callbacks can observe or mutate the same stored state.
use crate::{
    item_stack_refs::StackRef,
    item_use_lifecycle::{UseLifecycleWorld, UseState},
    living_use_flags,
    player_equipment::{self, InteractionHand},
    player_inventory::{EquipmentSlot, PlayerInventory},
    synced_values::{DataError, Observer, SyncedValues},
};
pub type ActiveStack<I, K, V> = UseState<StackRef<I, K, V>>;
pub struct UseView<'a, I, K, V> {
    pub inventory: &'a mut PlayerInventory<I, K, V>,
    pub flags: &'a mut SyncedValues<u8>,
}
pub trait UseServices<I, K, V> {
    type Error;
    type Level: Copy;
    fn level(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
    ) -> Result<Self::Level, Self::Error>;
    fn is_client(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
        level: Self::Level,
    ) -> Result<bool, Self::Error>;
    fn remaining_ticks(
        &mut self,
        _view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
    ) -> Result<i32, Self::Error> {
        Ok(state.remaining)
    }
    fn item_tick(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
        item: StackRef<I, K, V>,
        level: Self::Level,
        remaining: i32,
    ) -> Result<(), Self::Error>;
    fn use_on_release(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
        item: StackRef<I, K, V>,
    ) -> Result<bool, Self::Error>;
    fn release_item(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
        item: StackRef<I, K, V>,
        level: Self::Level,
        remaining: i32,
    ) -> Result<(), Self::Error>;
    fn finish_item(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
        item: StackRef<I, K, V>,
        level: Self::Level,
    ) -> Result<StackRef<I, K, V>, Self::Error>;
    fn finish_vibration(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
        item: StackRef<I, K, V>,
    ) -> Result<(), Self::Error>;
    fn on_equip(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
        slot: EquipmentSlot,
        old: StackRef<I, K, V>,
        new: StackRef<I, K, V>,
    ) -> Result<(), Self::Error>;
    fn on_synced(
        &mut self,
        view: UseView<'_, I, K, V>,
        state: &mut ActiveStack<I, K, V>,
        id: u8,
    ) -> Result<(), Self::Error>;
}
pub struct InventoryUse<I, K, V, S> {
    pub inventory: PlayerInventory<I, K, V>,
    pub flags: SyncedValues<u8>,
    pub services: S,
    empty: StackRef<I, K, V>,
}
impl<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, S> InventoryUse<I, K, V, S> {
    pub fn new(
        inventory: PlayerInventory<I, K, V>,
        flags: SyncedValues<u8>,
        services: S,
    ) -> Result<Self, u8> {
        flags.get(living_use_flags::FLAGS_ID)?;
        let empty = inventory.stacks().empty();
        Ok(Self {
            inventory,
            flags,
            services,
            empty,
        })
    }
}
struct FlagObserver<'a, I, K, V, S> {
    inventory: &'a mut PlayerInventory<I, K, V>,
    state: &'a mut ActiveStack<I, K, V>,
    services: &'a mut S,
}
impl<I, K, V, S: UseServices<I, K, V>> Observer<u8> for FlagObserver<'_, I, K, V, S> {
    type Error = S::Error;
    fn updated(&mut self, flags: &mut SyncedValues<u8>, id: u8) -> Result<(), Self::Error> {
        self.services.on_synced(
            UseView {
                inventory: self.inventory,
                flags,
            },
            self.state,
            id,
        )
    }
}
impl<I: Copy + Eq, K: Clone + Eq, V: Clone + PartialEq, S: UseServices<I, K, V>> UseLifecycleWorld
    for InventoryUse<I, K, V, S>
{
    type Error = S::Error;
    type Item = StackRef<I, K, V>;
    type Hand = InteractionHand;
    type Level = S::Level;
    fn empty_item(&self) -> Self::Item {
        self.empty.clone()
    }
    fn is_empty(&self, item: Self::Item) -> bool {
        item.read().is_empty()
    }
    fn same_item_type(&self, a: Self::Item, b: Self::Item) -> bool {
        a.read().same_item(&b.read())
    }
    fn using_item(&mut self, _: &mut ActiveStack<I, K, V>) -> Result<bool, Self::Error> {
        Ok(living_use_flags::using_item(&self.flags).expect("validated flags"))
    }
    fn used_hand(&mut self, _: &mut ActiveStack<I, K, V>) -> Result<InteractionHand, Self::Error> {
        Ok(living_use_flags::used_hand(&self.flags).expect("validated flags"))
    }
    fn hand_item(
        &mut self,
        _: &mut ActiveStack<I, K, V>,
        hand: InteractionHand,
    ) -> Result<Self::Item, Self::Error> {
        Ok(player_equipment::get_hand(&self.inventory, hand))
    }
    fn set_hand_item(
        &mut self,
        state: &mut ActiveStack<I, K, V>,
        hand: InteractionHand,
        item: Self::Item,
    ) -> Result<(), Self::Error> {
        let old = player_equipment::set(&mut self.inventory, hand.slot(), item.clone());
        self.services.on_equip(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
            hand.slot(),
            old,
            item,
        )
    }
    fn level(&mut self, state: &mut ActiveStack<I, K, V>) -> Result<Self::Level, Self::Error> {
        self.services.level(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
        )
    }
    fn is_client(
        &mut self,
        state: &mut ActiveStack<I, K, V>,
        level: Self::Level,
    ) -> Result<bool, Self::Error> {
        self.services.is_client(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
            level,
        )
    }
    fn remaining_ticks(&mut self, state: &mut ActiveStack<I, K, V>) -> Result<i32, Self::Error> {
        self.services.remaining_ticks(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
        )
    }
    fn item_tick(
        &mut self,
        state: &mut ActiveStack<I, K, V>,
        item: Self::Item,
        level: Self::Level,
        remaining: i32,
    ) -> Result<(), Self::Error> {
        self.services.item_tick(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
            item,
            level,
            remaining,
        )
    }
    fn use_on_release(
        &mut self,
        state: &mut ActiveStack<I, K, V>,
        item: Self::Item,
    ) -> Result<bool, Self::Error> {
        self.services.use_on_release(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
            item,
        )
    }
    fn release_item(
        &mut self,
        state: &mut ActiveStack<I, K, V>,
        item: Self::Item,
        level: Self::Level,
        remaining: i32,
    ) -> Result<(), Self::Error> {
        self.services.release_item(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
            item,
            level,
            remaining,
        )
    }
    fn finish_item(
        &mut self,
        state: &mut ActiveStack<I, K, V>,
        item: Self::Item,
        level: Self::Level,
    ) -> Result<Self::Item, Self::Error> {
        self.services.finish_item(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
            item,
            level,
        )
    }
    fn set_using_flag(
        &mut self,
        state: &mut ActiveStack<I, K, V>,
        using: bool,
    ) -> Result<(), Self::Error> {
        let mut observer = FlagObserver {
            inventory: &mut self.inventory,
            state,
            services: &mut self.services,
        };
        living_use_flags::set_flag(&mut self.flags, 1, using, &mut observer).map_err(|error| {
            match error {
                DataError::Observer(e) => e,
                DataError::UnknownId(_) => unreachable!("validated flags"),
            }
        })
    }
    fn finish_vibration(
        &mut self,
        state: &mut ActiveStack<I, K, V>,
        item: Self::Item,
    ) -> Result<(), Self::Error> {
        self.services.finish_vibration(
            UseView {
                inventory: &mut self.inventory,
                flags: &mut self.flags,
            },
            state,
            item,
        )
    }
}
