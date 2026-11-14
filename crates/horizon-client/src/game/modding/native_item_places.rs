//! The game's item places for native mods (`mod_api::ItemPlace`, arismoko/horizon#41 §7): a
//! player's slots, hands and cursor, entities' slots, item entities and the guest storage a
//! menu slot shows (ender chest, crafting grid, a mod's chest menu). Mods read and write
//! them through `ItemPlace`'s `stack`, `insert` and `extract`, which come here for these places; a block entity's slots,
//! a stack inside an item and a mod's own places go through their handlers.
use super::Game;
use super::transactions::After;
use horizon_client::{first_person::Hand, items::ItemStack};
use mod_api::__private::game::*;
use mod_api::{EquipmentSlot, ItemPlace, LivingRef, Transaction};

impl Game<'_> {
    /// The player a place belongs to, if a player's: a cursor's only while its menu is open.
    fn place_player(&self, place: &ItemPlace) -> Option<u128> {
        match place {
            ItemPlace::PlayerSlot { player, .. } | ItemPlace::PlayerHand { player, .. } => {
                Some(*player)
            }
            ItemPlace::PlayerCursor { menu } => self
                .menu_carried(*menu)
                .map(|_| menu.fields().player.uuid()),
            _ => None,
        }
    }
    /// The inventory of entity `id` in the level in place, if it has one: a chest or hopper
    /// minecart's (the open menu's copy while one shows it, as `cart_slots`), a chest boat's, or
    /// a chested donkey's, mule's or llama's (`AbstractHorse.inventory`, `getInventoryColumns`
    /// rows of three). With where it stands, for what a split throws.
    pub(super) fn edit_entity_slots<R>(
        &mut self,
        id: u32,
        edit: impl FnOnce(&mut [Option<ItemStack>], [f64; 3]) -> R,
    ) -> Option<R> {
        let key = (i32::MAX - 15, super::minecarts::CART_KEY_Y, id as i32);
        let at = |p: &horizon_core::geometry::Vec3| [p.x, p.y, p.z];
        let container = |c: &super::minecarts::Minecart| c.id == id && c.kind.slots() > 0;
        let position = if let Some(c) = self.carts.iter().find(|c| container(c)) {
            at(&c.position)
        } else if let Some(b) = self.boats.iter().find(|b| b.id == id && b.chest) {
            at(&b.position)
        } else {
            let m = self
                .mobs
                .iter_mut()
                .find(|m| m.id == id && m.extra.chested)?;
            let size = super::mount_menu::mount_columns(m) * 3;
            if m.extra.items.len() < size {
                if let Some(level) = self.transactions.last_mut() {
                    level.keep_mob_length(id, m.extra.items.len());
                }
                m.extra.items.resize(size, None);
            }
            return Some(edit(&mut m.extra.items[..size], at(&m.position)));
        };
        if self.carts.iter().any(container) {
            return self.edit_cart_slots(id, |slots| edit(slots, position));
        }
        if let Some(super::BlockEntity::Chest(open)) = self.world.block_entities.get_mut(&key) {
            return Some(edit(open, position));
        }
        let i = self.boats.iter().position(|b| b.id == id)?;
        Some(self.boats.update(i, |b| edit(&mut b.items, position)))
    }
    /// Whether this game simulates a world: a client's copy has no places to write.
    fn places_simulated(&self) -> bool {
        self.is_logical_server()
    }
    /// The slot of the player in place that `place` names.
    fn player_place(&mut self, place: &ItemPlace) -> Option<&mut Option<ItemStack>> {
        match place {
            ItemPlace::PlayerSlot { slot, .. } => self.native_carried_slot(*slot),
            ItemPlace::PlayerHand {
                hand: Hand::Main, ..
            } => self.inventory.get_mut(self.selected),
            ItemPlace::PlayerHand {
                hand: Hand::Off, ..
            } => Some(&mut self.offhand),
            ItemPlace::PlayerCursor { .. } => Some(&mut self.carried),
            _ => None,
        }
    }
    /// All player place mutations use the command/host slot writer (and its journal).
    fn write_player_place(&mut self, place: &ItemPlace, stack: Option<ItemStack>) {
        use super::commands::items::Slot;
        let slot = match place {
            ItemPlace::PlayerSlot {
                slot: slot @ 0..=35,
                ..
            } => Slot::Inv(*slot as usize),
            ItemPlace::PlayerSlot {
                slot: slot @ 100..=103,
                ..
            } => Slot::Armor(103 - *slot as usize),
            ItemPlace::PlayerSlot { slot: -106, .. } => Slot::Offhand,
            ItemPlace::PlayerHand {
                hand: Hand::Main, ..
            } => Slot::Inv(self.selected),
            ItemPlace::PlayerHand {
                hand: Hand::Off, ..
            } => Slot::Offhand,
            ItemPlace::PlayerCursor { .. } => {
                self.carried = stack;
                return;
            }
            _ => return,
        };
        self.slot_set(&slot, stack);
    }
    /// How many slots the inventory of entity `entity` in the level in place has, as
    /// `entity_slots` reaches them: none for an entity without one, and on a client.
    pub(super) fn entity_container_size(&self, entity: u32) -> Option<usize> {
        self.entity_automation_size(entity).or_else(|| {
            let m = self
                .mobs
                .iter()
                .find(|m| m.id == entity && m.extra.chested)?;
            Some(super::mount_menu::mount_columns(m) * 3)
        })
    }
    /// How many slots the container entity `entity` of the level in place shows automation
    /// (`EntitySelector.CONTAINER_ENTITY_SELECTOR`: chest and hopper minecarts, chest boats; a
    /// chested mount is not one): none for any other entity, and on a client.
    pub(super) fn entity_automation_size(&self, entity: u32) -> Option<usize> {
        if self.remote.is_some() {
            return None;
        }
        if let Some(c) = self.carts.iter().find(|c| c.id == entity) {
            let container = c.kind.slots() > 0;
            return container.then_some(c.items.len());
        }
        let b = self.boats.iter().find(|b| b.id == entity)?;
        b.chest.then_some(b.items.len())
    }
    /// How many equipment slots living entity `entity`'s [`ENTITY_ITEMS`](mod_api::ENTITY_ITEMS)
    /// answers (#932, NeoForge's `LivingEntityEquipmentWrapper`): a mob's or an armor stand's
    /// seven (main hand, off hand, feet, legs, chest, head and body, by `EquipmentSlot`'s
    /// ordinal; the saddle is not automation's to move), none for any other entity, and on a
    /// client.
    pub(super) fn entity_equipment_size(&self, entity: u32) -> Option<usize> {
        // The handler must not exist on a client: the same gate every read
        // and write of a slot answers (`equipment_stack`).
        if !self.places_simulated() {
            return None;
        }
        (self.mobs.iter().any(|m| m.id == entity && !m.removed)
            || self.armor_stands.iter().any(|a| a.id == entity))
        .then_some(7)
    }
    /// `EntityType.is(TagKey)`: whether `entity_type` (`minecraft:player`) is in the entity
    /// type tag `tag` (`minecraft:cannot_be_pushed_onto_boats`).
    pub(super) fn entity_type_in_tag(&self, entity_type: &str, tag: &str) -> bool {
        let tag = horizon_client::command::args::normalize(tag);
        self.res.mob_data().entity_has(&tag, entity_type)
    }
    /// The stack at a player's or an item entity's place; none for an empty one, a player who
    /// isn't here or is dead, an item entity not in the level in place, any other place
    /// (`ItemPlace::stack` reads those), and on a client.
    pub(super) fn game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack> {
        if let ItemPlace::Scratch(id) = place {
            return self.scratch_held(*id);
        }
        if self.block_entity_tick_items.is_some() {
            return self.scoped_game_stack(place);
        }
        self.unscoped_game_stack(place)
    }
    /// `ItemAccess.insert` at a player's place, an item entity's or an entity's slot: up to
    /// `stack.count` of `stack`, into the place where it's empty or holds the same item and
    /// components, up to the item's stack size. The rest goes where vanilla puts an item split
    /// off it: a player's into their inventory, else dropped as `Player.drop(stack, false)` at
    /// the root commit (a creative player's hand takes anything, and gives them one if they have
    /// none, as `ItemUtils.createFilledResult`); an item entity's dropped beside it at the root
    /// commit; an entity slot's into the entity's other slots (`forHandlerIndex`). How much
    /// went: 0 where `game_stack` reads nothing for want of the place. In transaction
    /// `tx`, which keeps each place as it was. The player's screen follows with the tick's sync.
    pub(super) fn game_insert(
        &mut self,
        place: &ItemPlace,
        resource: &ItemStack,
        amount: u32,
        tx: &Transaction,
    ) -> u32 {
        let stack = ItemStack {
            count: amount,
            ..resource.clone()
        };
        if !self.transaction_holds(tx) {
            return 0;
        }
        if !self.places_simulated() || stack.count == 0 {
            return 0;
        }
        if let ItemPlace::Scratch(id) = place {
            return self.scratch_insert(*id, &stack);
        }
        let max = self.containers().max_stack(&stack);
        // Puts what of `rest` fits into `slot`.
        let fill = |slot: &mut Option<ItemStack>, rest: &mut ItemStack| {
            let held = slot.as_ref().map_or(0, |held| held.count);
            let room = match slot {
                Some(h) if held > 0 && !h.same_item_same_components(rest) => 0,
                _ => max.saturating_sub(held),
            };
            let put = room.min(rest.count);
            if put > 0 {
                *slot = Some(ItemStack {
                    count: held + put,
                    ..rest.clone()
                });
                rest.count -= put;
            }
        };
        let mut rest = stack.clone();
        if let ItemPlace::MenuSlot { menu, slot } = place {
            // The slot alone: what doesn't fit stays out.
            if self.native_menu_storage(*menu, *slot).is_none() {
                return 0;
            }
            self.journal_menus(menu.fields().player);
            if let Some(held) = self.native_menu_storage(*menu, *slot) {
                fill(held, &mut rest);
            }
            return stack.count - rest.count;
        }
        if let ItemPlace::ItemEntity { id } = place {
            let Some(e) = self.dropped.iter().find(|e| e.id == *id) else {
                return 0;
            };
            let at = [e.position.x, e.position.y, e.position.z];
            let mut held = Some(e.stack.clone());
            fill(&mut held, &mut rest);
            if let Some(held) = held.filter(|_| rest.count < stack.count) {
                self.journal_set_item_entity(*id, held);
            }
            if rest.count > 0 {
                self.after_commit(After::Spawn(at, rest));
            }
            return stack.count;
        }
        if let ItemPlace::EntitySlot { entity, slot } = place {
            if !self
                .edit_entity_slots(*entity, |slots, _| *slot < slots.len())
                .unwrap_or(false)
            {
                return 0;
            }
            self.journal_entity(*entity);
            return self
                .edit_entity_slots(*entity, |slots, _| {
                    fill(&mut slots[*slot], &mut rest);
                    for (i, held) in slots.iter_mut().enumerate() {
                        if i != *slot {
                            fill(held, &mut rest);
                        }
                    }
                    stack.count - rest.count
                })
                .unwrap_or(0);
        }
        if let ItemPlace::Equipment { entity, slot } = place {
            // One slot only: what doesn't fit stays out (the handler's
            // capacity already refused what the kind cannot wear).
            let Some(mut held) = self.equipment_stack(*entity, *slot) else {
                return 0;
            };
            fill(&mut held, &mut rest);
            if rest.count == stack.count {
                return 0;
            }
            let written = held.filter(|h| h.count > 0);
            // The journal keeps what was, only for a write that really lands:
            // a refused one (a client's copy, no epoch) moved nothing.
            self.journal_equipment(*entity);
            if !self.write_entity_equipment(*entity, *slot, written) {
                return 0;
            }
            return stack.count - rest.count;
        }
        let Some(player) = self.place_player(place) else {
            return 0;
        };
        let mut went = 0;
        self.native_as_player(player, |g| {
            if g.player_place(place).is_none() {
                return;
            }
            went = stack.count;
            g.journal_player(player);
            if g.creative && matches!(place, ItemPlace::PlayerHand { .. }) {
                let have = g.inventory.iter().flatten();
                if !have.clone().any(|h| h.same_item_same_components(&stack)) {
                    g.add_stack_to_inventory(ItemStack { count: 1, ..stack });
                }
                return;
            }
            let mut written = g.player_place(place).cloned().flatten();
            fill(&mut written, &mut rest);
            g.write_player_place(place, written);
            if let Some(rest) = (rest.count > 0)
                .then(|| g.add_stack_to_inventory(rest))
                .flatten()
            {
                g.drop_after_commit(player, rest);
            }
        });
        went
    }
    /// `ItemAccess.extract` at those places: up to `stack.count` of the item and components of
    /// `stack`, how many came out. An item entity emptied stays, empty, until the root commit
    /// removes it. A creative player's hand gives without losing any (`forInfiniteMaterials`).
    pub(super) fn game_extract(
        &mut self,
        place: &ItemPlace,
        resource: &ItemStack,
        amount: u32,
        tx: &Transaction,
    ) -> u32 {
        let stack = ItemStack {
            count: amount,
            ..resource.clone()
        };
        if !self.transaction_holds(tx) {
            return 0;
        }
        if !self.places_simulated() {
            return 0;
        }
        if let ItemPlace::Scratch(id) = place {
            return self.scratch_extract(*id, &stack);
        }
        // How many of `stack` `held` gives.
        let taken = |held: Option<&ItemStack>| {
            held.filter(|held| held.same_item_same_components(&stack))
                .map_or(0, |held| held.count.min(stack.count))
        };
        let left = |held: &ItemStack, took: u32| ItemStack {
            count: held.count - took,
            ..held.clone()
        };
        if let ItemPlace::MenuSlot { menu, slot } = place {
            let Some(held) = self.native_menu_storage(*menu, *slot) else {
                return 0;
            };
            let took = taken(held.as_ref());
            if took == 0 {
                return 0;
            }
            self.journal_menus(menu.fields().player);
            if let Some(held) = self.native_menu_storage(*menu, *slot) {
                *held = held.as_ref().map(|h| left(h, took)).filter(|h| h.count > 0);
            }
            return took;
        }
        if let ItemPlace::ItemEntity { id } = place {
            let Some(e) = self.dropped.iter().find(|e| e.id == *id) else {
                return 0;
            };
            let took = taken(Some(&e.stack));
            let kept = left(&e.stack, took);
            if took > 0 {
                self.journal_set_item_entity(*id, kept);
            }
            return took;
        }
        if let ItemPlace::EntitySlot { entity, slot } = place {
            let took = self
                .edit_entity_slots(*entity, |slots, _| {
                    taken(slots.get(*slot).and_then(Option::as_ref))
                })
                .unwrap_or(0);
            if took == 0 {
                return 0;
            }
            self.journal_entity(*entity);
            return self
                .edit_entity_slots(*entity, |slots, _| {
                    let kept = slots[*slot].as_ref().map(|held| left(held, took));
                    slots[*slot] = kept.filter(|kept| kept.count > 0);
                    took
                })
                .unwrap_or(0);
        }
        if let ItemPlace::Equipment { entity, slot } = place {
            let Some(held) = self.equipment_stack(*entity, *slot) else {
                return 0;
            };
            let took = taken(held.as_ref());
            if took == 0 {
                return 0;
            }
            let kept = held
                .as_ref()
                .map(|held| left(held, took))
                .filter(|kept| kept.count > 0);
            self.journal_equipment(*entity);
            if !self.write_entity_equipment(*entity, *slot, kept) {
                return 0;
            }
            return took;
        }
        let Some(player) = self.place_player(place) else {
            return 0;
        };
        let mut took = 0;
        self.native_as_player(player, |g| {
            let creative = g.creative && matches!(place, ItemPlace::PlayerHand { .. });
            let Some(slot) = g.player_place(place) else {
                return;
            };
            took = taken(slot.as_ref());
            if took == 0 || creative {
                return;
            }
            g.journal_player(player);
            let kept = g
                .player_place(place)
                .and_then(|slot| slot.as_ref().map(|held| left(held, took)));
            g.write_player_place(place, kept.filter(|kept| kept.count > 0));
        });
        took
    }
    /// Writes mob `entity`'s equipment `slot` (#932): the same write
    /// `native_living_set_equipment` makes — stack, attribute map, equip sound and game event
    /// — with the effects waiting for the root commit of the open transaction, so an abort of
    /// it is quiet. The equipment goes back then too (`journal_equipment`).
    /// The equipment `slot` of living entity `entity` (#932): a mob's
    /// (`equipment_stacks`, the worn stacks overlaid) or an armor stand's
    /// own, `None` where the slot is empty. None where neither kind is
    /// there — and on a client, whose copy writes nothing.
    fn equipment_stack(&self, entity: u32, slot: EquipmentSlot) -> Option<Option<ItemStack>> {
        if !self.places_simulated() {
            return None;
        }
        if let Some(mob) = self.mobs.iter().find(|m| m.id == entity && !m.removed) {
            return Some(mob.equipment_stacks().get(slot as usize).cloned());
        }
        let stand = self.armor_stands.iter().find(|a| a.id == entity)?;
        Some(stand.equipment.get(slot as usize).cloned())
    }
    /// The equipment write `set_equipment` makes (#932), deferred: whether it
    /// landed. False where the entity is gone or a client's copy refuses.
    fn write_entity_equipment(
        &mut self,
        entity: u32,
        slot: EquipmentSlot,
        stack: Option<ItemStack>,
    ) -> bool {
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return false;
        };
        let living = LivingRef::new(mod_api::EntityRef::new(epoch, entity));
        self.write_living_equipment(living, slot, stack, true)
    }
    /// Drops `stack` at the middle of the block at `pos`, as a block entity's handler gives up
    /// what it can't put back.
    pub(super) fn drop_at(&mut self, pos: [i32; 3], stack: ItemStack) {
        if self.entity_ids_exhausted() {
            return;
        }
        if self.places_simulated() && stack.count > 0 {
            // `Block.popResource`'s default pickup delay, at the root commit.
            self.after_commit(After::Drop(pos, stack));
        }
    }
}

include!("native_item_places/tick_item_read.rs");

#[cfg(test)]
pub(super) mod tests {
    use super::super::BlockEntity;
    use super::super::api_values::TryIntoApi;
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::{
        items::ItemStack,
        lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig},
    };
    use horizon_core::geometry::Direction;
    use horizon_protocol::packets::configuration::ClientInformation;
    use mod_api::HostTransaction;
    use mod_api::ServerHost;
    use mod_api::{ENTITY_AUTOMATION, ENTITY_ITEMS, ItemPlace, ItemResource, MenuRef};
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    /// A world open to LAN on a dedicated server, as a guest's actions run.
    pub(in crate::game) fn open_to_lan(game: &mut super::Game<'_>) {
        game.options.dedicated = Some(Vec::new());
        let settings = super::super::lan::LanSettings {
            port: 0,
            guest_commands: false,
            force_game_mode: false,
        };
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "PlaceTest".into(), 2)
        };
        let config = HubConfig {
            default_game_mode: 0,
            guest_commands: false,
            force_game_mode: false,
            hardcore: false,
            view_distance: 10,
            simulation_distance: 10,
            dimension: "minecraft:overworld".into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
        };
        game.lan_host(settings, net, config).unwrap();
    }

    /// A placed guest on connection `conn`, with UUID and entity id `conn + 122` and
    /// `conn + 19`.
    pub(in crate::game) fn guest(conn: u32) -> Guest {
        let (tx, _rx) = mpsc::channel();
        let mut guest = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn) + 122,
                name: format!("guest{conn}"),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: String::new(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:1".parse().unwrap(),
                expected: false,
            },
            conn as i32 + 19,
            [0.0; 3],
            8,
        );
        guest.placed = true;
        guest
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn whole_stack_component_edit_preserves_slot_and_refuses_item_or_count_changes() {
        use mod_api::Tag;
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.inventory[3] = Some(ItemStack::new("minecraft:paper", 16));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let host_before = game.inventory.clone();
            let dropped_before = game.dropped.len();
            let inventory =
                |game: &super::Game<'_>| game.lan.as_ref().unwrap().hub.guests[0].inventory.clone();
            let mut expected = inventory(game);
            let name = Tag::String("named paper".into());
            expected[3].as_mut().unwrap().data_mut().custom_name = Some(name.clone());
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 3,
            };
            let result = game.transaction(|host, tx| {
                Ok::<_, ()>(slot.edit(host, tx, |stack| {
                    stack
                        .apply_components(&[("minecraft:custom_name".into(), name)].into())
                        .unwrap();
                }))
            });
            assert_eq!(result, Ok(true));
            assert_eq!(game.dropped.len(), dropped_before);
            assert_eq!(
                inventory(game),
                expected,
                "all 16 named paper stay in slot 3"
            );
            assert_eq!(game.inventory, host_before);

            // Refusing the edit must not need an outer abort to keep the slot intact.
            assert_eq!(
                game.transaction(|host, tx| {
                    Ok::<_, ()>(slot.edit(host, tx, |stack| stack.set_count(15).unwrap()))
                }),
                Ok(false)
            );
            assert_eq!(inventory(game), expected);
            let stone = ItemStack::new("minecraft:stone", 16)
                .try_into_api()
                .unwrap();
            assert_eq!(
                game.transaction(|host, tx| {
                    Ok::<_, ()>(slot.edit(host, tx, |stack| *stack = stone))
                }),
                Ok(false)
            );
            assert_eq!(inventory(game), expected);
            assert_eq!(game.inventory, host_before);

            let empty = ItemPlace::PlayerSlot {
                player: 123,
                slot: 4,
            };
            assert_eq!(
                game.transaction(|host, tx| {
                    Ok::<_, ()>(empty.edit(host, tx, |_| panic!("empty place ran editor")))
                }),
                Ok(false)
            );
            assert_eq!(inventory(game), expected);
            assert_eq!(game.dropped.len(), dropped_before);
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn whole_stack_component_edit_rolls_back_with_its_transaction() {
        use mod_api::Tag;
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.inventory[3] = Some(ItemStack::new("minecraft:paper", 16));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let inventory =
                |game: &super::Game<'_>| game.lan.as_ref().unwrap().hub.guests[0].inventory.clone();
            let before = inventory(game);
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 3,
            };
            let mut named = ItemStack::new("minecraft:paper", 1);
            named.data_mut().custom_name = Some(Tag::String("copied name".into()));
            let source = named.try_into_api().unwrap();
            let mut expected = before.clone();
            expected[3].as_mut().unwrap().data_mut().custom_name =
                Some(Tag::String("copied name".into()));
            let expected_stack = expected[3].as_ref().unwrap().try_into_api().unwrap();
            let original_stack = before[3].as_ref().unwrap().try_into_api().unwrap();
            assert_eq!(
                game.transaction(|host, tx| {
                    assert!(slot.edit(host, tx, |stack| {
                        stack.copy_component_from(&source, "minecraft:custom_name");
                    }));
                    assert_eq!(slot.stack(host), Some(expected_stack.clone()));
                    Err::<(), ()>(())
                }),
                Err(())
            );
            assert_eq!(inventory(game), before);

            // A child abort restores the slot even when the parent goes on to commit.
            assert_eq!(
                game.transaction(|host, _| {
                    let child = host.transaction(|host, tx| {
                        assert!(slot.edit(host, tx, |stack| {
                            stack.copy_component_from(&source, "minecraft:custom_name");
                        }));
                        Err::<(), ()>(())
                    });
                    assert_eq!(child, Err(()));
                    assert_eq!(slot.stack(host), Some(original_stack.clone()));
                    Ok::<(), ()>(())
                }),
                Ok(())
            );
            assert_eq!(inventory(game), before);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_bucket_fills_and_empties_whole_through_its_place_and_splits_off_its_stack() {
        use horizon_client::chunk::Chunk;
        use mod_api::{BlockUpdateFlags, FLUIDS, ITEM_FLUIDS, move_resources};
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let cauldron = |game: &super::Game<'_>, n: u8| {
                let registry = &game.pack.assets.registry;
                if n == 0 {
                    return registry.default_state("minecraft:cauldron").unwrap();
                }
                let state = horizon_world::chunk::BlockState {
                    name: "minecraft:water_cauldron".to_owned(),
                    properties: [("level".to_owned(), n.to_string())].into_iter().collect(),
                };
                registry.resolve(&state).unwrap()
            };
            let set = |game: &mut super::Game<'_>, pos, n| {
                let state = cauldron(game, n);
                let flags = BlockUpdateFlags::NEIGHBORS;
                assert_eq!(game.level_set_block(level, pos, state, flags), Ok(true));
            };
            let (a, b) = ([0, 64, 0], [2, 64, 0]);
            set(game, a, 3);
            set(game, b, 2);
            let mut player = guest(1);
            player.inventory[0] = Some(ItemStack::new("minecraft:bucket", 3));
            player.inventory[5] = Some(ItemStack::new("minecraft:lava_bucket", 1));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let slot = |slot| ItemPlace::PlayerSlot { player: 123, slot };
            // Up to a bucket from the cauldron at `at` into the bucket in `slot`, or back.
            let moved = |game: &mut super::Game<'_>, at, slot: &ItemPlace, into_bucket| {
                let host: &mut dyn ServerHost = game;
                let block = level.capability(host, &FLUIDS, at, None).unwrap();
                let bucket = slot.capability(host, &ITEM_FLUIDS).unwrap();
                let (from, to) = match into_bucket {
                    true => (&block, &bucket),
                    false => (&bucket, &block),
                };
                host.transaction(|host, tx| move_resources(host, from, to, |_| true, 1000, tx))
            };
            let inventory =
                |game: &super::Game<'_>| game.lan.as_ref().unwrap().hub.guests[0].inventory.clone();
            let block =
                |game: &super::Game<'_>, pos: [i32; 3]| game.world.block(pos[0], pos[1], pos[2]);
            // A full cauldron fills one of the three buckets, which splits off.
            assert_eq!(moved(game, a, &slot(0), true), Ok(1000));
            assert_eq!(block(game, a), cauldron(game, 0));
            let held = inventory(game);
            assert_eq!(held[0], Some(ItemStack::new("minecraft:bucket", 2)));
            let water = ItemStack::new("minecraft:water_bucket", 1);
            let filled = held
                .iter()
                .position(|s| s.as_ref() == Some(&water))
                .unwrap();
            // Two thirds of a bucket fill none: a bucket takes a whole one or nothing.
            assert_eq!(moved(game, b, &slot(0), true), Ok(0));
            assert_eq!(block(game, b), cauldron(game, 2));
            assert_eq!(inventory(game), held);
            // The water bucket empties into the empty cauldron and is a bucket again.
            let filled = slot(i32::try_from(filled).unwrap());
            assert_eq!(moved(game, a, &filled, false), Ok(1000));
            assert_eq!(block(game, a), cauldron(game, 3));
            assert_eq!(
                filled.stack(game),
                Some(
                    ItemStack::new("minecraft:bucket", 1)
                        .try_into_api()
                        .unwrap()
                )
            );
            // Lava doesn't go into a water cauldron, and the lava bucket stays full.
            assert_eq!(moved(game, a, &slot(5), false), Ok(0));
            let lava = Some(ItemStack::new("minecraft:lava_bucket", 1));
            assert_eq!(inventory(game)[5], lava);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_guests_place_is_written_as_theirs_and_an_abort_puts_it_back() {
        with_game(|game| {
            open_to_lan(game);
            let mut guest = guest(1);
            guest.inventory[0] = Some(ItemStack::new("minecraft:bucket", 3));
            game.lan.as_mut().unwrap().hub.guests.push(guest);
            let host_before = game.inventory.clone();
            let guest_inventory =
                |game: &super::Game<'_>| game.lan.as_ref().unwrap().hub.guests[0].inventory.clone();
            let before = guest_inventory(game);
            let slot = ItemPlace::PlayerSlot {
                player: 123,
                slot: 0,
            };
            let water = ItemResource::of(
                &ItemStack::new("minecraft:water_bucket", 1)
                    .try_into_api()
                    .unwrap(),
            );

            // Aborted, the guest's inventory is as it was, and the host's was never touched.
            let host: &mut dyn ServerHost = game;
            let _ = host.transaction(|host, tx| {
                assert_eq!(slot.exchange(host, tx, &water, 1), 1);
                assert_eq!(slot.amount(host), 2);
                Err::<(), ()>(())
            });
            assert_eq!(guest_inventory(game), before);
            assert_eq!(game.inventory, host_before);

            // Committed, the guest keeps two buckets and gets the filled one.
            let host: &mut dyn ServerHost = game;
            let _ = host.transaction(|host, tx| Ok::<_, ()>(slot.exchange(host, tx, &water, 1)));
            let after = guest_inventory(game);
            assert_eq!(after[0], Some(ItemStack::new("minecraft:bucket", 2)));
            let filled = after.iter().flatten();
            assert_eq!(
                filled
                    .filter(|s| &*s.item == "minecraft:water_bucket")
                    .count(),
                1
            );
            assert_eq!(game.inventory, host_before);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn an_armour_slot_holds_one() {
        with_game(|game| {
            open_to_lan(game);
            game.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(20)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let pumpkin = ItemResource::of(
                &ItemStack::new("minecraft:carved_pumpkin", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let wrote =
                host.transaction(|host, tx| Ok::<_, ()>(items.insert(host, tx, 39, &pumpkin, 64)));
            assert_eq!(wrote, Ok(1));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn stand_equipment_effects_wait_for_root_commit() {
        with_game(|game| {
            let mut stand = horizon_client::entities::ArmorStand::new(
                40,
                horizon_core::geometry::Vec3::ZERO,
                0.0,
            );
            stand.first_tick = false;
            game.armor_stands.push(stand);
            game.audio.capture = Some(Vec::new());
            let items = game
                .entity(40)
                .unwrap()
                .capability(game, &ENTITY_ITEMS, ())
                .unwrap();
            let chest = ItemResource::of(
                &ItemStack::new("minecraft:diamond_chestplate", 1)
                    .try_into_api()
                    .unwrap(),
            );
            super::super::dispense_fixture::start_recording();
            let result = game.transaction(|host, tx| {
                assert_eq!(items.insert(host, tx, 4, &chest, 1), 1);
                Err::<(), ()>(())
            });
            assert_eq!(result, Err(()));
            assert!(game.armor_stands[0].equipment.get(4).is_none());
            assert!(
                game.audio.capture.as_ref().unwrap().is_empty(),
                "aborted stand equip emitted sound"
            );
            assert!(super::super::dispense_fixture::stop_recording().is_empty());

            super::super::dispense_fixture::start_recording();
            let result = game.transaction(|host, _| {
                host.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 4, &chest, 1), 1);
                    Ok::<(), ()>(())
                })?;
                Err::<(), ()>(())
            });
            assert_eq!(result, Err(()));
            assert!(game.armor_stands[0].equipment.get(4).is_none());
            assert!(game.audio.capture.as_ref().unwrap().is_empty());
            assert!(super::super::dispense_fixture::stop_recording().is_empty());

            super::super::dispense_fixture::start_recording();
            let result = game.transaction(|host, _| {
                // An inner abort must not contribute effects to the root.
                let aborted = host.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 4, &chest, 1), 1);
                    Err::<(), ()>(())
                });
                assert_eq!(aborted, Err(()));
                host.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 4, &chest, 1), 1);
                    Ok::<(), ()>(())
                })?;
                assert!(super::super::dispense_fixture::stop_recording().is_empty());
                super::super::dispense_fixture::start_recording();
                Ok::<(), ()>(())
            });
            assert_eq!(result, Ok(()));
            assert_eq!(game.armor_stands[0].equipment.get(4).unwrap().count, 1);
            assert_eq!(game.audio.capture.as_ref().unwrap().len(), 1);
            let events = super::super::dispense_fixture::stop_recording();
            assert_eq!(events.len(), 1, "only one event at root commit: {events:?}");
            assert!(events[0].ends_with("minecraft:equip"));
        });
    }

    /// A zombie mob, standing at the origin.
    fn zombie(game: &mut super::Game<'_>) -> u32 {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:zombie")
            .unwrap()
            .clone();
        game.mobs.push(horizon_client::mobs::Mob::new(
            30,
            kind,
            horizon_core::geometry::Vec3::new(0.5, 64.0, 0.5),
            0.0,
        ));
        30
    }

    /// A living entity's [`ENTITY_ITEMS`] is its equipment (#932): seven slots by
    /// `EquipmentSlot`'s ordinal, the armour ones taking only what their `equippable`
    /// names for that kind, the writes transactional — an abort puts the equipment back
    /// and stays quiet, a commit is `set_equipment`'s write.
    #[test]
    #[ignore = "needs prepared resources"]
    fn living_equipment_moves_through_the_entity_items_handler() {
        with_game(|game| {
            let id = zombie(game);
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(id)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            assert_eq!(items.size(host), 7);
            let chestplate = ItemResource::of(
                &ItemStack::new("minecraft:diamond_chestplate", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let boots = ItemResource::of(
                &ItemStack::new("minecraft:diamond_boots", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let sword = ItemResource::of(
                &ItemStack::new("minecraft:iron_sword", 1)
                    .try_into_api()
                    .unwrap(),
            );
            // The armour slots take what names them for a zombie, one of it; the
            // chestplate is not for the feet, and a sword is not armour at all.
            assert_eq!(items.capacity(host, 4, &chestplate), 1);
            assert_eq!(items.capacity(host, 2, &chestplate), 0);
            assert_eq!(items.capacity(host, 2, &boots), 1);
            assert_eq!(items.capacity(host, 4, &sword), 0);
            // A hand takes a stack of anything.
            assert_eq!(items.capacity(host, 0, &sword), 1);

            // An aborted transaction puts the equipment back.
            let _ = host.transaction(|host, tx| {
                assert_eq!(items.insert(host, tx, 4, &chestplate, 1), 1);
                assert_eq!(
                    items.amount(host, 4),
                    1,
                    "the transaction sees its own write"
                );
                Err::<(), ()>(())
            });
            assert_eq!(items.amount(host, 4), 0, "the abort put it back");

            // A committed one wears it and hands it back out.
            let _ = host.transaction(|host, tx| Ok::<_, ()>(items.insert(host, tx, 0, &sword, 1)));
            let _ =
                host.transaction(|host, tx| Ok::<_, ()>(items.insert(host, tx, 4, &chestplate, 1)));
            assert_eq!(items.amount(host, 0), 1);
            assert_eq!(items.amount(host, 4), 1);
            let _ = host
                .transaction(|host, tx| Ok::<_, ()>(items.extract(host, tx, 4, &chestplate, 1)));
            assert_eq!(items.amount(host, 4), 0);
            assert_eq!(items.amount(host, 0), 1);

            // A gone entity answers nothing.
            game.mobs.clear();
            let host: &mut dyn ServerHost = game;
            assert_eq!(items.amount(host, 4), 0);
            assert_eq!(
                host.transaction(|host, tx| Ok::<_, ()>(items.insert(host, tx, 0, &sword, 1))),
                Ok(0)
            );
        });
    }

    /// A committed equipment write through the handler is `set_equipment`'s: the same
    /// stacks, and the same attribute map (#941's live map reads the worn armour).
    #[test]
    #[ignore = "needs prepared resources"]
    fn a_committed_equipment_write_matches_set_equipments() {
        use mod_api::EquipmentSlot;
        with_game(|game| {
            zombie(game);
            game.mobs.push(horizon_client::mobs::Mob::new(
                31,
                game.res
                    .mob_types()
                    .get("minecraft:zombie")
                    .unwrap()
                    .clone(),
                horizon_core::geometry::Vec3::new(2.5, 64.0, 0.5),
                0.0,
            ));
            let host: &mut dyn ServerHost = game;
            let chest = ItemStack::new("minecraft:diamond_chestplate", 1);
            // Through the handler, committed.
            let handler = host
                .entity(30)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let _ = host.transaction(|host, tx| {
                Ok::<_, ()>(handler.insert(
                    host,
                    tx,
                    4,
                    &ItemResource::of(&chest.clone().try_into_api().unwrap()),
                    1,
                ))
            });
            // Through `set_equipment`.
            let through_setter = game.living(game.entity(31).unwrap()).unwrap();
            assert!(game.living_set_equipment(
                through_setter,
                EquipmentSlot::Chest,
                Some(chest.clone())
            ));
            let (a, b) = (&game.mobs[0], &game.mobs[1]);
            assert_eq!(
                a.equipment_stacks().get(4).cloned(),
                b.equipment_stacks().get(4).cloned(),
                "the same worn stack"
            );
            // The attribute map answers the same armour for both (#941).
            let read = |game: &super::Game<'_>, id: u32| {
                let living = game.living(game.entity(id).unwrap()).unwrap();
                let host: &dyn mod_api::Host = game;
                living.attribute_value(host, "minecraft:armor")
            };
            assert_eq!(read(game, 30), read(game, 31));
            assert!(
                read(game, 30).unwrap_or(0.0) >= 8.0,
                "diamond chestplate armour"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_creative_players_selected_stack_takes_what_fits_and_keeps_it() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.game_mode = 1;
            player.inventory[0] = Some(ItemStack::new("minecraft:stick", 60));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(20)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            assert_eq!(
                host.transaction(|host, tx| Ok::<_, ()>(items.insert_any(host, tx, &sticks, 4))),
                Ok(4)
            );
            assert_eq!(
                game.lan.as_ref().unwrap().hub.guests[0].inventory[0]
                    .as_ref()
                    .unwrap()
                    .count,
                64
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_kept_handler_reaches_nothing_once_its_cart_is_removed() {
        use super::super::{
            commands::Target,
            minecarts::{CartKind, Minecart},
        };
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.carts.push(Minecart::new(
                500,
                CartKind::Chest,
                horizon_core::geometry::Vec3::ZERO,
            ));
            let host: &mut dyn ServerHost = game;
            let entity = host.entity(500).unwrap();
            let items = entity.capability(host, &ENTITY_ITEMS, ()).unwrap();
            game.persist.removals.push(Target::Cart(500));
            assert!(entity.capability(game, &ENTITY_ITEMS, ()).is_none());
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let host: &mut dyn ServerHost = game;
            let wrote =
                host.transaction(|host, tx| Ok::<_, ()>(items.insert(host, tx, 0, &sticks, 1)));
            assert_eq!(wrote, Ok(0));
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn bound_armour_comes_off_only_in_creative() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            let mut helmet = ItemStack::new("minecraft:iron_helmet", 1);
            helmet
                .data_mut()
                .enchantments
                .push(("minecraft:binding_curse".into(), 1));
            player.armor[0] = Some(helmet.clone());
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(20)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let helmet = ItemResource::of(&helmet.try_into_api().unwrap());
            assert_eq!(items.amount(host, 39), 1);
            let take = |host: &mut dyn ServerHost| {
                host.transaction(|host, tx| Ok::<_, ()>(items.extract(host, tx, 39, &helmet, 1)))
            };
            assert_eq!(take(host), Ok(0));
            // A creative player takes it off (`ArmorSlot.mayPickup`'s `isCreative`).
            {
                let value = 1;
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .guests
                    .update(0, |guest| guest.game_mode = value);
            }
            assert_eq!(take(game), Ok(1));
            assert_eq!(game.lan.as_ref().unwrap().hub.guests[0].armor[0], None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn an_equippable_whose_entity_tag_holds_the_player_fits_its_slot() {
        use mod_api::{ComponentId, Tag};
        with_game(|game| {
            open_to_lan(game);
            game.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let host: &mut dyn ServerHost = game;
            let items = host
                .entity(20)
                .unwrap()
                .capability(host, &ENTITY_ITEMS, ())
                .unwrap();
            let helmet = ItemResource::of(
                &ItemStack::new("minecraft:iron_helmet", 1)
                    .try_into_api()
                    .unwrap(),
            )
            .with_component(
                host,
                &ComponentId::new("minecraft:equippable").unwrap(),
                Tag::Compound(
                    [
                        ("slot".into(), Tag::String("head".into())),
                        (
                            "allowed_entities".into(),
                            Tag::String("#minecraft:cannot_be_pushed_onto_boats".into()),
                        ),
                    ]
                    .into_iter()
                    .collect(),
                ),
            )
            .unwrap();
            assert_eq!(items.capacity(host, 39, &helmet), 1);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_kept_handler_reaches_nothing_once_its_world_closes() {
        use super::super::minecarts::{CartKind, Minecart};
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.carts.push(Minecart::new(
                500,
                CartKind::Chest,
                horizon_core::geometry::Vec3::ZERO,
            ));
            let host: &mut dyn ServerHost = game;
            let entity = host.entity(500).unwrap();
            let items = entity.capability(host, &ENTITY_ITEMS, ()).unwrap();
            game.native_lifecycle.begin_close();
            game.native_lifecycle.world_close_final();
            assert!(entity.capability(game, &ENTITY_ITEMS, ()).is_none());
            let host: &mut dyn ServerHost = game;
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let wrote =
                host.transaction(|host, tx| Ok::<_, ()>(items.insert(host, tx, 0, &sticks, 1)));
            assert_eq!(wrote, Ok(0));
        });
    }

    /// The open menu of guest 1 (UUID 123).
    fn menu_of(game: &super::Game<'_>) -> MenuRef {
        let player = game.player(123).unwrap();
        game.player_container_menu(player).unwrap()
    }

    /// One write at `place` in a transaction of its own: how many went in (`amount > 0`) or
    /// came out (`amount < 0`), committed or aborted.
    fn write(
        game: &mut super::Game<'_>,
        place: &ItemPlace,
        item: &str,
        amount: i32,
        commit: bool,
    ) -> u32 {
        let resource = ItemResource::of(&ItemStack::new(item, 1).try_into_api().unwrap());
        let host: &mut dyn ServerHost = game;
        let mut moved = 0;
        let _ = host.transaction(|host, tx| {
            moved = match amount > 0 {
                true => place.insert(host, tx, &resource, amount.unsigned_abs()),
                false => place.extract(host, tx, &resource, amount.unsigned_abs()),
            };
            if commit { Ok(()) } else { Err(()) }
        });
        moved
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_chest_menus_slots_write_the_chest_and_the_inventory_and_an_abort_puts_them_back() {
        use horizon_client::{chunk::Chunk, lan::menu::Slots};
        use mod_api::BlockUpdateFlags;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game.pack.assets.registry.default_state("minecraft:chest");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest.unwrap(), flags),
                Ok(true)
            );
            let mut player = guest(1);
            player.carried = Some(ItemStack::new("minecraft:emerald", 9));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let title = horizon_core::nbt::Tag::String("chest".into());
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x3",
                title,
                (0, 64, 0),
                Slots::Block((0, 64, 0)),
                vec![None; 27],
                None,
            );
            let menu = menu_of(game);
            // Slot 0 is the chest's first, 27 the inventory's 9 and 54 the hotbar's first.
            let (chest, inventory, hotbar) = (
                ItemPlace::MenuSlot { menu, slot: 0 },
                ItemPlace::MenuSlot { menu, slot: 27 },
                ItemPlace::MenuSlot { menu, slot: 54 },
            );
            let held = |game: &super::Game<'_>| {
                let Some(BlockEntity::Chest(items)) = game.world.block_entities.get(&(0, 64, 0))
                else {
                    panic!("no chest")
                };
                let g = &game.lan.as_ref().unwrap().hub.guests[0];
                (
                    items[0].clone(),
                    g.inventory[9].clone(),
                    g.inventory[0].clone(),
                    g.carried.clone(),
                )
            };
            let before = held(game);
            let host: &mut dyn ServerHost = game;
            let emerald = ItemResource::of(
                &ItemStack::new("minecraft:emerald", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let moved = |host: &mut dyn ServerHost, commit: bool| {
                host.transaction(|host, tx| {
                    let took = menu.cursor().extract(host, tx, &emerald, 9);
                    let went = [&chest, &inventory, &hotbar]
                        .map(|slot| slot.insert(host, tx, &emerald, 3));
                    if commit {
                        Ok((took, went))
                    } else {
                        Err((took, went))
                    }
                })
            };
            assert_eq!(moved(host, false), Err((9, [3, 3, 3])));
            assert_eq!(held(game), before);
            let host: &mut dyn ServerHost = game;
            assert_eq!(moved(host, true), Ok((9, [3, 3, 3])));
            let three = Some(ItemStack::new("minecraft:emerald", 3));
            assert_eq!(held(game), (three.clone(), three.clone(), three, None));
            // A slot holds what its item stacks to and no more, and nothing spills elsewhere.
            assert_eq!(write(game, &chest, "minecraft:emerald", 70, true), 61);
            assert_eq!(write(game, &chest, "minecraft:stick", 1, true), 0);
            assert_eq!(write(game, &chest, "minecraft:emerald", -64, true), 64);
            assert_eq!(held(game).0, None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_chests_menu_over_the_furnace_set_in_its_place_reaches_none_of_its_slots() {
        use horizon_client::{chunk::Chunk, lan::menu::Slots};
        use mod_api::BlockUpdateFlags;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let state = |game: &super::Game<'_>, name| {
                game.pack.assets.registry.default_state(name).unwrap()
            };
            let (chest, furnace) = (
                state(game, "minecraft:chest"),
                state(game, "minecraft:furnace"),
            );
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest, flags),
                Ok(true)
            );
            let mut player = guest(1);
            player.inventory[8] = Some(ItemStack::new("minecraft:stick", 4));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let title = horizon_core::nbt::Tag::String("chest".into());
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:generic_9x3",
                title,
                (0, 64, 0),
                Slots::Block((0, 64, 0)),
                vec![None; 27],
                None,
            );
            let menu = menu_of(game);
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], furnace, flags),
                Ok(true)
            );
            // Until the close pass, the menu still has the chest's 63 slots: 0 was the chest's
            // first, 62 is the last of the hotbar, past a furnace menu's 39.
            let (first, hotbar) = (
                ItemPlace::MenuSlot { menu, slot: 0 },
                ItemPlace::MenuSlot { menu, slot: 62 },
            );
            let stick =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            for place in [&first, &hotbar] {
                assert_eq!(write(game, place, "minecraft:stick", 1, true), 0);
                assert_eq!(write(game, place, "minecraft:stick", -1, true), 0);
                let host: &mut dyn ServerHost = game;
                let swapped =
                    host.transaction(|host, tx| Ok::<_, ()>(place.exchange(host, tx, &stick, 1)));
                assert_eq!(swapped, Ok(0));
            }
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.inventory[8], Some(ItemStack::new("minecraft:stick", 4)));
            let Some(BlockEntity::Furnace(f)) = game.world.block_entities.get(&(0, 64, 0)) else {
                panic!("no furnace")
            };
            assert!(f.items.iter().all(Option::is_none));
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn an_armour_slot_and_the_craft_result_take_only_what_the_inventory_menu_lets_them() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.craft_result = Some(ItemStack::new("minecraft:oak_planks", 4));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let menu = menu_of(game);
            // `InventoryMenu`: 0 the result, 5 the head, 8 the feet.
            let (result, head, feet) = (
                ItemPlace::MenuSlot { menu, slot: 0 },
                ItemPlace::MenuSlot { menu, slot: 5 },
                ItemPlace::MenuSlot { menu, slot: 8 },
            );
            assert_eq!(write(game, &head, "minecraft:stick", 1, true), 0);
            assert_eq!(write(game, &feet, "minecraft:iron_helmet", 1, true), 0);
            assert_eq!(write(game, &head, "minecraft:iron_helmet", 2, true), 1);
            assert_eq!(write(game, &result, "minecraft:oak_planks", 1, true), 0);
            assert_eq!(write(game, &result, "minecraft:oak_planks", -4, true), 0);
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.armor[0], Some(ItemStack::new("minecraft:iron_helmet", 1)));
            assert_eq!(
                g.craft_result,
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_crafting_grid_write_makes_its_result_at_the_commit() {
        with_game(|game| {
            open_to_lan(game);
            game.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let menu = menu_of(game);
            let grid = ItemPlace::MenuSlot { menu, slot: 1 };
            let log = ItemResource::of(
                &ItemStack::new("minecraft:oak_log", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let result = |game: &super::Game<'_>| {
                game.lan.as_ref().unwrap().hub.guests[0]
                    .craft_result
                    .clone()
            };
            let host: &mut dyn ServerHost = game;
            let aborted = host.transaction(|host, tx| {
                assert_eq!(grid.insert(host, tx, &log, 1), 1);
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!((g.craft_grid[0].clone(), result(game)), (None, None));
            let host: &mut dyn ServerHost = game;
            let committed = host.transaction(|host, tx| {
                assert_eq!(grid.insert(host, tx, &log, 1), 1);
                // Not yet: the result is made once, at the commit.
                Ok::<_, ()>(menu.slot(host, 0))
            });
            assert_eq!(committed, Ok(Some(None)));
            assert_eq!(
                result(game),
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
            assert_eq!(write(game, &grid, "minecraft:oak_log", -1, true), 1);
            assert_eq!(result(game), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_mods_chest_menu_is_written_by_slot_and_a_replaced_menu_writes_nothing() {
        with_game(|game| {
            open_to_lan(game);
            game.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let title = horizon_core::nbt::Tag::String("shop".into());
            let open = |game: &mut super::Game<'_>| {
                let hub = &mut game.lan.as_mut().unwrap().hub;
                hub.open_native_menu(1, 1, title.clone(), vec![]).unwrap();
                menu_of(game)
            };
            let shop = ItemPlace::MenuSlot {
                menu: open(game),
                slot: 2,
            };
            let shown = |game: &super::Game<'_>| {
                game.lan.as_ref().unwrap().hub.guests[0].native_menu[2].clone()
            };
            assert_eq!(write(game, &shop, "minecraft:diamond", 5, false), 5);
            assert_eq!(shown(game), None);
            assert_eq!(write(game, &shop, "minecraft:diamond", 5, true), 5);
            assert_eq!(shown(game), Some(ItemStack::new("minecraft:diamond", 5)));
            // The same chest opened again is a new menu: the old one reaches nothing.
            let again = open(game);
            assert_eq!(write(game, &shop, "minecraft:diamond", 1, true), 0);
            assert_eq!(write(game, &shop, "minecraft:diamond", -1, true), 0);
            let host: &mut dyn ServerHost = game;
            assert_eq!(shop.stack(host), None);
            let fresh = ItemPlace::MenuSlot {
                menu: again,
                slot: 2,
            };
            assert_eq!(write(game, &fresh, "minecraft:diamond", 1, true), 1);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_furnace_menus_fuel_slot_takes_one_bucket_as_its_slot_does() {
        use horizon_client::{chunk::Chunk, lan::menu::Slots};
        use mod_api::BlockUpdateFlags;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let furnace = game.pack.assets.registry.default_state("minecraft:furnace");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], furnace.unwrap(), flags),
                Ok(true)
            );
            game.lan.as_mut().unwrap().hub.guests.push(guest(1));
            game.lan.as_mut().unwrap().hub.open_container(
                1,
                "minecraft:furnace",
                horizon_core::nbt::Tag::String("furnace".into()),
                (0, 64, 0),
                Slots::Block((0, 64, 0)),
                vec![None; 3],
                None,
            );
            let menu = menu_of(game);
            let (fuel, result) = (
                ItemPlace::MenuSlot { menu, slot: 1 },
                ItemPlace::MenuSlot { menu, slot: 2 },
            );
            // `FurnaceFuelSlot.getMaxStackSize(bucket)` is 1, not a bucket's 16; coal stacks to 64.
            assert_eq!(write(game, &fuel, "minecraft:bucket", 16, true), 1);
            assert_eq!(write(game, &fuel, "minecraft:bucket", 1, true), 0);
            assert_eq!(write(game, &fuel, "minecraft:bucket", -1, true), 1);
            assert_eq!(write(game, &fuel, "minecraft:stick", 70, true), 64);
            // `FurnaceResultSlot.mayPlace` is false.
            assert_eq!(write(game, &result, "minecraft:iron_ingot", 1, true), 0);
            let host: &mut dyn ServerHost = game;
            let bucket = ItemStack::new("minecraft:bucket", 1)
                .try_into_api()
                .unwrap();
            assert_eq!(menu.slot_holds(host, 1, &bucket), Some(1));
            assert_eq!(menu.slot_holds(host, 2, &bucket), Some(0));
            // The hotbar's last slot is plain; past it there is no slot.
            assert_eq!(menu.slot_holds(host, 38, &bucket), Some(16));
            assert_eq!(menu.slot_holds(host, 39, &bucket), None);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_stale_menus_cursor_reaches_nothing_of_the_next_ones() {
        with_game(|game| {
            open_to_lan(game);
            game.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let stale = menu_of(game);
            let title = horizon_core::nbt::Tag::String("new".into());
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.open_native_menu(1, 1, title, vec![]).unwrap();
            {
                let value = Some(ItemStack::new("minecraft:stick", 1));
                hub.guests.update(0, |guest| guest.carried = value);
            }
            assert_eq!(stale.carried(game), None);
            assert_eq!(write(game, &stale.cursor(), "minecraft:stick", -1, true), 0);
            assert_eq!(write(game, &stale.cursor(), "minecraft:stick", 1, true), 0);
            let host: &mut dyn ServerHost = game;
            assert_eq!(stale.cursor().stack(host), None);
            let fresh = menu_of(game).cursor();
            assert_eq!(write(game, &fresh, "minecraft:stick", -1, true), 1);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_grid_written_in_and_around_an_aborted_child_makes_its_result_from_what_stays() {
        with_game(|game| {
            open_to_lan(game);
            game.lan.as_mut().unwrap().hub.guests.push(guest(1));
            let menu = menu_of(game);
            let (grid, other) = (
                ItemPlace::MenuSlot { menu, slot: 1 },
                ItemPlace::MenuSlot { menu, slot: 2 },
            );
            let log = ItemResource::of(
                &ItemStack::new("minecraft:oak_log", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let host: &mut dyn ServerHost = game;
            let committed = host.transaction(|host, tx| {
                assert_eq!(grid.insert(host, tx, &log, 1), 1);
                let child = host.transaction(|host, tx| {
                    assert_eq!(other.insert(host, tx, &log, 3), 3);
                    Err::<(), ()>(())
                });
                assert!(child.is_err());
                assert_eq!((grid.amount(host), other.amount(host)), (1, 0));
                assert_eq!(grid.insert(host, tx, &log, 1), 1);
                Ok::<_, ()>(menu.slot(host, 0))
            });
            assert_eq!(committed, Ok(Some(None)));
            // Two logs in one slot: one craft's planks; the child's second log never landed.
            let g = &game.lan.as_ref().unwrap().hub.guests[0];
            assert_eq!(g.craft_grid[1], None);
            assert_eq!(
                g.craft_result,
                Some(ItemStack::new("minecraft:oak_planks", 4))
            );
        });
    }

    /// The guests' inventories, by connection.
    fn inventories(game: &super::Game<'_>) -> Vec<Vec<Option<ItemStack>>> {
        let guests = &game.lan.as_ref().unwrap().hub.guests;
        guests.iter().map(|g| g.inventory.clone()).collect()
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_players_items_go_where_inventory_add_puts_them_and_an_abort_puts_them_back() {
        with_game(|game| {
            open_to_lan(game);
            let mut player = guest(1);
            player.selected = 4;
            player.inventory[4] = Some(ItemStack::new("minecraft:stick", 60));
            player.inventory[7] = Some(ItemStack::new("minecraft:stick", 10));
            player.inventory[2] = Some(ItemStack::new("minecraft:stone", 1));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let before = inventories(game);
            let host: &mut dyn ServerHost = game;
            let entity = host.entity(20).unwrap();
            let items = entity.capability(host, &ENTITY_ITEMS, ()).unwrap();
            assert_eq!(items.size(host), 41);
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let aborted = host.transaction(|host, tx| {
                assert_eq!(items.insert_any(host, tx, &sticks, 70), 70);
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert_eq!(inventories(game), before);

            // `Inventory.add`: onto the selected stack (4 more), then the same item in order
            // (54 more), then the first empty slot (the other 12); not onto the stone.
            let host: &mut dyn ServerHost = game;
            let went =
                host.transaction(|host, tx| Ok::<_, ()>(items.insert_any(host, tx, &sticks, 70)));
            assert_eq!(went, Ok(70));
            let after = &inventories(game)[0];
            assert_eq!(after[4], Some(ItemStack::new("minecraft:stick", 64)));
            assert_eq!(after[7], Some(ItemStack::new("minecraft:stick", 64)));
            assert_eq!(after[0], Some(ItemStack::new("minecraft:stick", 12)));
            assert_eq!(after[2], Some(ItemStack::new("minecraft:stone", 1)));
            // Index 39 is the head: a stick doesn't go there.
            let host: &mut dyn ServerHost = game;
            let head =
                host.transaction(|host, tx| Ok::<_, ()>(items.insert(host, tx, 39, &sticks, 1)));
            assert_eq!(head, Ok(0));
        });
    }

    /// One player's offer in a trade: inventory index `.0` must still hold exactly `.1`.
    type Offer = (usize, ItemStack);

    /// Two guests' trade on a menu, as the docs' recipe has it: each offer's index must still
    /// hold exactly what was offered, and goes whole to the other player, or nothing moves.
    fn trade(game: &mut super::Game<'_>, a: &[Offer], b: &[Offer]) -> Result<(), ()> {
        let host: &mut dyn ServerHost = game;
        let items = |host: &mut dyn ServerHost, id| {
            let entity = host.entity(id).unwrap();
            entity.capability(host, &ENTITY_ITEMS, ()).unwrap()
        };
        let (seller, buyer) = (items(host, 20), items(host, 21));
        host.transaction(|host, tx| {
            for (from, to, offer) in [(&seller, &buyer, a), (&buyer, &seller, b)] {
                for (index, stack) in offer {
                    let offered = ItemResource::of(&stack.try_into_api().unwrap());
                    let still = from.resource(host, *index).as_ref() == Some(&offered)
                        && from.amount(host, *index) == stack.count;
                    if !still
                        || from.extract(host, tx, *index, &offered, stack.count) != stack.count
                        || to.insert_any(host, tx, &offered, stack.count) != stack.count
                    {
                        return Err(());
                    }
                }
            }
            Ok(())
        })
    }

    /// A seller with 3 diamonds in slot 0 and a buyer with 5 emeralds in slot 0, beside
    /// `buyer_rest` in every other slot and the off hand.
    fn traders(game: &mut super::Game<'_>, buyer_rest: Option<ItemStack>) {
        open_to_lan(game);
        let mut seller = guest(1);
        seller.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3));
        let mut buyer = guest(2);
        buyer.inventory[0] = Some(ItemStack::new("minecraft:emerald", 5));
        for slot in &mut buyer.inventory[1..36] {
            *slot = buyer_rest.clone();
        }
        buyer.offhand = buyer_rest;
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .extend([seller, buyer]);
    }

    fn offers() -> ([Offer; 1], [Offer; 1]) {
        (
            [(0, ItemStack::new("minecraft:diamond", 3))],
            [(0, ItemStack::new("minecraft:emerald", 5))],
        )
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_trade_between_two_players_moves_both_offers() {
        with_game(|game| {
            traders(game, None);
            let (diamonds, emeralds) = offers();
            assert_eq!(trade(game, &diamonds, &emeralds), Ok(()));
            // The diamonds go first, into the buyer's first empty slot (1: the emeralds are
            // still in 0), then the emeralds into the seller's emptied slot 0.
            let after = inventories(game);
            assert_eq!(after[0][0], Some(ItemStack::new("minecraft:emerald", 5)));
            assert_eq!(after[1][0], None);
            assert_eq!(after[1][1], Some(ItemStack::new("minecraft:diamond", 3)));
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_trade_into_a_full_inventory_moves_nothing() {
        with_game(|game| {
            // The buyer's other slots and off hand hold stone, so only slot 0 is free once the
            // emeralds leave; the diamonds go before the emeralds and find no room.
            traders(game, Some(ItemStack::new("minecraft:stone", 64)));
            let before = inventories(game);
            let (diamonds, emeralds) = offers();
            assert_eq!(trade(game, &diamonds, &emeralds), Err(()));
            assert_eq!(inventories(game), before);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_stale_offer_moves_nothing() {
        with_game(|game| {
            traders(game, None);
            let (diamonds, emeralds) = offers();
            // The seller used a diamond after offering three, and the buyer moved the emeralds
            // to another slot: each offer no longer matches its index.
            let guests = &mut game.lan.as_mut().unwrap().hub.guests;
            guests.update(0, |g| {
                g.inventory[0] = Some(ItemStack::new("minecraft:diamond", 2))
            });
            let before = inventories(game);
            assert_eq!(trade(game, &diamonds, &emeralds), Err(()));
            assert_eq!(inventories(game), before);
            let guests = &mut game.lan.as_mut().unwrap().hub.guests;
            guests.update(0, |g| {
                g.inventory[0] = Some(ItemStack::new("minecraft:diamond", 3))
            });
            guests.update(1, |g| g.inventory[4] = g.inventory[0].take());
            let before = inventories(game);
            assert_eq!(trade(game, &diamonds, &emeralds), Err(()));
            assert_eq!(inventories(game), before);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn scoped_boat_slots_use_the_open_menu_and_refuse_a_removed_owner() {
        use super::super::boats::Boat;
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            let mut boat = Boat::new(600, "oak".into(), Vec3::new(1.0, 65.0, 2.0), 0.0);
            boat.chest = true;
            boat.items.resize(27, None);
            game.boats.push(boat);
            assert_eq!(
                game.edit_entity_slots(600, |slots, at| {
                    assert_eq!(at, [1.0, 65.0, 2.0]);
                    slots[0] = Some(ItemStack::new("minecraft:stone", 4));
                    slots.len()
                }),
                Some(27)
            );
            let key = (i32::MAX - 15, super::super::minecarts::CART_KEY_Y, 600);
            game.world
                .block_entities
                .insert(key, super::super::BlockEntity::Chest(vec![None; 27]));
            game.edit_entity_slots(600, |slots, _| {
                assert!(slots[0].is_none());
                slots[0] = Some(ItemStack::new("minecraft:sand", 7));
            });
            assert_eq!(
                game.boats[0].items[0],
                Some(ItemStack::new("minecraft:stone", 4))
            );
            assert_eq!(
                game.edit_entity_slots(600, |slots, _| slots[0].clone()),
                Some(Some(ItemStack::new("minecraft:sand", 7)))
            );
            game.boats.remove(0);
            assert_eq!(
                game.edit_entity_slots(600, |_, _| panic!("stale owner reached its menu")),
                None::<()>
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_chest_minecarts_items_are_its_slots() {
        use super::super::minecarts::{CartKind, Minecart};
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let position = horizon_core::geometry::Vec3::new(0.5, 65.0, 0.5);
            game.carts
                .push(Minecart::new(500, CartKind::Chest, position));
            let host: &mut dyn ServerHost = game;
            let cart = host.entity(500).unwrap();
            assert_eq!(
                cart.kind(host).map(|kind| kind.id(host)).as_deref(),
                Some("minecraft:chest_minecart")
            );
            let items = cart.capability(host, &ENTITY_ITEMS, ()).unwrap();
            assert_eq!(items.size(host), 27);
            let sticks =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let aborted = host.transaction(|host, tx| {
                assert_eq!(items.insert_any(host, tx, &sticks, 70), 70);
                Err::<(), ()>(())
            });
            assert!(aborted.is_err());
            assert!(game.carts[0].items.iter().all(Option::is_none));
            let host: &mut dyn ServerHost = game;
            let went =
                host.transaction(|host, tx| Ok::<_, ()>(items.insert_any(host, tx, &sticks, 70)));
            assert_eq!(went, Ok(70));
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:stick", 64))
            );
            assert_eq!(
                game.carts[0].items[1],
                Some(ItemStack::new("minecraft:stick", 6))
            );
            // A plain minecart has no items.
            game.carts
                .push(Minecart::new(501, CartKind::Rideable, position));
            let host: &mut dyn ServerHost = game;
            let plain = host.entity(501).unwrap();
            assert!(plain.capability(host, &ENTITY_ITEMS, ()).is_none());
        });
    }
    #[test]
    #[ignore = "needs prepared resources"]
    fn cart_slot_edits_prefer_the_menu_and_refuse_a_removed_owner() {
        use super::super::minecarts::{CART_KEY_Y, CartKind, Minecart};
        use horizon_core::geometry::Vec3;
        with_game(|game| {
            let mut cart = Minecart::new(600, CartKind::Chest, Vec3::new(1.0, 65.0, 2.0));
            cart.items[0] = Some(ItemStack::new("minecraft:stone", 4));
            game.carts.push(cart);
            let key = (i32::MAX - 15, CART_KEY_Y, 600);
            game.world
                .block_entities
                .insert(key, BlockEntity::Chest(vec![None; 27]));
            assert_eq!(
                game.edit_cart_slots(600, |slots| {
                    slots[0] = Some(ItemStack::new("minecraft:sand", 7));
                    slots.len()
                }),
                Some(27)
            );
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:stone", 4))
            );
            let Some(BlockEntity::Chest(open)) = game.world.block_entities.get(&key) else {
                panic!("missing live menu");
            };
            assert_eq!(open[0], Some(ItemStack::new("minecraft:sand", 7)));
            game.store_cart_items(key);
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:sand", 7))
            );
            game.edit_cart_slots(600, |slots| {
                slots[0] = Some(ItemStack::new("minecraft:gravel", 9))
            });
            game.close_cart_container(key);
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:gravel", 9))
            );
            assert!(!game.world.block_entities.contains_key(&key));
            game.carts.remove(0);
            assert_eq!(
                game.edit_cart_slots(600, |_| panic!("stale owner reached its menu")),
                None::<()>
            );
        });
    }

    /// A mod's `ENTITY_AUTOMATION` handler: one slot of up to 64 items, with the sides it was
    /// asked for.
    #[derive(Clone, Default)]
    struct Bin {
        held: Arc<std::sync::Mutex<Option<(String, u32)>>>,
        sides: Arc<std::sync::Mutex<Vec<Option<Direction>>>>,
    }

    impl mod_api::ResourceHandler<ItemResource> for Bin {
        fn size(&self, _: &mut dyn ServerHost) -> usize {
            1
        }
        fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
            let held = self.held.lock().unwrap();
            held.as_ref()
                .map(|(item, _)| ItemResource::of(&ItemStack::new(item, 1).try_into_api().unwrap()))
        }
        fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
            self.held.lock().unwrap().as_ref().map_or(0, |(_, n)| *n)
        }
        fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
            64
        }
        fn insert(
            &self,
            _: &mut dyn ServerHost,
            _: &mod_api::Transaction,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let item = mod_api::__private::item_key_id(resource.as_stack().item())
                .unwrap()
                .to_string();
            let mut held = self.held.lock().unwrap();
            match held.as_mut() {
                Some((have, n)) if *have == item => {
                    let went = amount.min(64 - *n);
                    *n += went;
                    went
                }
                Some(_) => 0,
                None => {
                    let went = amount.min(64);
                    *held = Some((item, went));
                    went
                }
            }
        }
        fn extract(
            &self,
            _: &mut dyn ServerHost,
            _: &mod_api::Transaction,
            _: usize,
            resource: &ItemResource,
            amount: u32,
        ) -> u32 {
            let item = mod_api::__private::item_key_id(resource.as_stack().item())
                .unwrap()
                .to_string();
            let mut held = self.held.lock().unwrap();
            let Some((_, n)) = held.as_mut().filter(|(have, _)| *have == item) else {
                return 0;
            };
            let came = amount.min(*n);
            *n -= came;
            if *n == 0 {
                *held = None;
            }
            came
        }
    }

    /// Runs `test` with a mod answering `ENTITY_AUTOMATION` for pigs with `bin`.
    fn with_bin_pigs(bin: &Bin, test: impl FnOnce(&mut super::Game<'_>)) {
        use mod_api::{self, Hooks, ModEntry, Mods, Registrar};
        struct Pigs(Bin);
        #[mod_api::__subscriptions]
        impl Hooks for Pigs {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let bin = self.0.clone();
                r.entity_capability(&ENTITY_AUTOMATION, "minecraft:pig", move |_, _, side| {
                    bin.sides.lock().unwrap().push(side);
                    Some(Box::new(bin.clone()))
                });
            }
        }
        let mods: Mods = vec![ModEntry::new("pigs", Box::new(Pigs(bin.clone())))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || with_game(test));
    }

    /// A hopper at (0, 64, 0) facing `facing` holding `stack`, in an otherwise empty world.
    fn hopper_with(game: &mut super::Game<'_>, facing: &str, stack: Option<ItemStack>) {
        use horizon_client::chunk::Chunk;
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let base = game
            .pack
            .assets
            .registry
            .default_state("minecraft:hopper")
            .unwrap();
        let state = game.with_props(base, &[("facing", facing)]).unwrap();
        game.world.set(0, 64, 0, state).unwrap();
        let mut slots = vec![None; 5];
        slots[0] = stack;
        game.world
            .block_entities
            .insert((0, 64, 0), BlockEntity::Chest(slots));
    }

    fn pig(game: &mut super::Game<'_>, id: u32, y: f64) {
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs.push(horizon_client::mobs::Mob::new(
            id,
            kind,
            horizon_core::geometry::Vec3::new(0.5, y, 0.5),
            0.0,
        ));
    }

    fn hopper_slot(game: &super::Game<'_>) -> Option<ItemStack> {
        match game.world.block_entities.get(&(0, 64, 0)) {
            Some(BlockEntity::Chest(slots)) => slots[0].clone(),
            _ => panic!("the hopper is gone"),
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_hopper_pushes_into_a_mod_entity_below_it_through_the_side_it_faces() {
        let bin = Bin::default();
        with_bin_pigs(&bin, |game| {
            hopper_with(game, "down", Some(ItemStack::new("minecraft:stick", 3)));
            pig(game, 700, 63.0);
            game.hopper_try_move_items((0, 64, 0));
            assert_eq!(
                *bin.held.lock().unwrap(),
                Some(("minecraft:stick".into(), 1))
            );
            assert_eq!(
                hopper_slot(game),
                Some(ItemStack::new("minecraft:stick", 2))
            );
            // The hopper reaches the pig from above: its top.
            let sides = bin.sides.lock().unwrap();
            assert!(!sides.is_empty());
            assert!(sides.iter().all(|side| *side == Some(Direction::Up)));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_hopper_pushes_into_a_block_with_no_block_entity_whose_provider_voids_the_items() {
        use mod_api::{self, Hooks, ModEntry, Mods, Registrar};
        /// A trash can's one slot: it takes anything and keeps nothing.
        struct Voids;
        impl mod_api::ResourceHandler<ItemResource> for Voids {
            fn size(&self, _: &mut dyn ServerHost) -> usize {
                1
            }
            fn resource(&self, _: &mut dyn ServerHost, _: usize) -> Option<ItemResource> {
                None
            }
            fn amount(&self, _: &mut dyn ServerHost, _: usize) -> u32 {
                0
            }
            fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
                64
            }
            fn insert(
                &self,
                _: &mut dyn ServerHost,
                _: &mod_api::Transaction,
                _: usize,
                _: &ItemResource,
                amount: u32,
            ) -> u32 {
                amount
            }
            fn extract(
                &self,
                _: &mut dyn ServerHost,
                _: &mod_api::Transaction,
                _: usize,
                _: &ItemResource,
                _: u32,
            ) -> u32 {
                0
            }
        }
        struct TrashCan;
        #[mod_api::__subscriptions]
        impl Hooks for TrashCan {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                let lodestone = r.block_key("minecraft:lodestone");
                r.block_capability(&mod_api::ITEMS, &lodestone, |_, _, _, entity| {
                    entity.is_none().then(|| Box::new(Voids) as _)
                });
            }
        }
        let mods: Mods = vec![ModEntry::new("trash_can", Box::new(TrashCan))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                hopper_with(game, "down", Some(ItemStack::new("minecraft:stick", 3)));
                let registry = &game.pack.assets.registry;
                let lodestone = registry.default_state("minecraft:lodestone").unwrap();
                let stone = registry.default_state("minecraft:stone").unwrap();
                game.world.set(0, 63, 0, lodestone).unwrap();
                game.world.set(1, 63, 0, stone).unwrap();
                // The hopper pushes one stick down into the lodestone, which keeps none.
                game.hopper_try_move_items((0, 64, 0));
                assert_eq!(
                    hopper_slot(game),
                    Some(ItemStack::new("minecraft:stick", 2))
                );
                let host: &mut dyn ServerHost = game;
                let level = host.level("minecraft:overworld").unwrap();
                let bin = level
                    .capability(
                        host,
                        &mod_api::ITEMS,
                        [0, 63, 0],
                        Some(mod_api::Direction::Up),
                    )
                    .expect("the lodestone's provider");
                assert_eq!(bin.owner(), "trash_can");
                assert_eq!(bin.amount(host, 0), 0);
                // Stone has no provider and no slots of the game's.
                assert!(
                    level
                        .capability(host, &mod_api::ITEMS, [1, 63, 0], None)
                        .is_none()
                );
            })
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_hopper_pulls_from_a_mod_entity_above_it_through_its_bottom() {
        let bin = Bin::default();
        *bin.held.lock().unwrap() = Some(("minecraft:stick".into(), 5));
        with_bin_pigs(&bin, |game| {
            hopper_with(game, "down", None);
            pig(game, 700, 65.0);
            game.hopper_try_move_items((0, 64, 0));
            assert_eq!(
                hopper_slot(game),
                Some(ItemStack::new("minecraft:stick", 1))
            );
            assert_eq!(
                *bin.held.lock().unwrap(),
                Some(("minecraft:stick".into(), 4))
            );
            let sides = bin.sides.lock().unwrap();
            assert!(!sides.is_empty());
            assert!(sides.iter().all(|side| *side == Some(Direction::Down)));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_hopper_still_fills_a_chest_minecart_and_a_player_is_not_reached() {
        use super::super::minecarts::{CartKind, Minecart};
        let bin = Bin::default();
        with_bin_pigs(&bin, |game| {
            open_to_lan(game);
            hopper_with(game, "down", Some(ItemStack::new("minecraft:stick", 3)));
            let position = horizon_core::geometry::Vec3::new(0.5, 63.0, 0.5);
            game.carts
                .push(Minecart::new(500, CartKind::Chest, position));
            // A guest standing in the same cell, and no pig anywhere.
            let mut player = guest(1);
            player.position = [0.5, 63.0, 0.5];
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let host: &mut dyn ServerHost = game;
            let player = host.entity(20).unwrap();
            assert!(
                player
                    .capability(host, &ENTITY_AUTOMATION, Some(Direction::Up))
                    .is_none()
            );
            game.hopper_try_move_items((0, 64, 0));
            assert_eq!(
                game.carts[0].items[0],
                Some(ItemStack::new("minecraft:stick", 1))
            );
            assert_eq!(
                hopper_slot(game),
                Some(ItemStack::new("minecraft:stick", 2))
            );
            assert!(bin.held.lock().unwrap().is_none());
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_dropper_puts_into_a_mod_entity_in_front_of_it() {
        use horizon_client::chunk::Chunk;
        let bin = Bin::default();
        with_bin_pigs(&bin, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let base = game
                .pack
                .assets
                .registry
                .default_state("minecraft:dropper")
                .unwrap();
            let dropper = game.with_props(base, &[("facing", "down")]).unwrap();
            game.world.set(0, 64, 0, dropper).unwrap();
            let mut slots = vec![None; 9];
            slots[4] = Some(ItemStack::new("minecraft:stick", 2));
            game.world
                .block_entities
                .insert((0, 64, 0), BlockEntity::Chest(slots));
            pig(game, 700, 63.0);
            let dropped = game.dropped.len();
            game.dispense_from(horizon_core::position::BlockPos::new(0, 64, 0), dropper)
                .unwrap();
            assert_eq!(
                *bin.held.lock().unwrap(),
                Some(("minecraft:stick".into(), 1))
            );
            assert_eq!(game.dropped.len(), dropped);
            match game.world.block_entities.get(&(0, 64, 0)) {
                Some(BlockEntity::Chest(slots)) => {
                    assert_eq!(slots[4], Some(ItemStack::new("minecraft:stick", 1)));
                }
                _ => panic!("the dropper is gone"),
            }
            let sides = bin.sides.lock().unwrap();
            assert!(!sides.is_empty());
            assert!(sides.iter().all(|side| *side == Some(Direction::Up)));
        });
    }
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_crafter_puts_into_a_mod_entity_and_dispenses_only_what_it_refuses() {
        use horizon_client::chunk::Chunk;
        use horizon_core::{position::BlockPos, random::RandomSource as _};
        use mod_api::BlockUpdateFlags;

        let bin = Bin::default();
        with_bin_pigs(&bin, |game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let base = game
                .pack
                .assets
                .registry
                .default_state("minecraft:crafter")
                .unwrap();
            let crafter = game
                .with_props(base, &[("orientation", "down_east")])
                .unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], crafter, BlockUpdateFlags::NONE),
                Ok(true)
            );
            pig(game, 700, 63.0);
            for (before, after, refused) in [(0, 4, 0), (62, 64, 2), (64, 64, 4)] {
                *bin.held.lock().unwrap() =
                    (before > 0).then(|| ("minecraft:stick".into(), before));
                let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get_mut(&(0, 64, 0))
                else {
                    panic!("the placed crafter is missing");
                };
                c.items[0] = Some(ItemStack::new("minecraft:oak_planks", 1));
                c.items[3] = Some(ItemStack::new("minecraft:oak_planks", 1));
                let dropped = game.dropped.len();
                let mut random = game.world.random.clone();
                game.dispense_from(BlockPos::new(0, 64, 0), crafter)
                    .unwrap();
                assert_eq!(
                    *bin.held.lock().unwrap(),
                    Some(("minecraft:stick".into(), after))
                );
                let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get(&(0, 64, 0))
                else {
                    panic!("the crafter is gone");
                };
                assert!(c.items.iter().all(Option::is_none));
                let output = &game.dropped[dropped..];
                if refused == 0 {
                    assert!(output.is_empty());
                    // One candidate-selection draw, including a single candidate, and no
                    // dispensing draws when the handler took the whole result.
                    random.next_int_bound(1).unwrap();
                    assert_eq!(game.world.random.clone().next_long(), random.next_long());
                } else {
                    assert_eq!(output.len(), 1);
                    assert_eq!(output[0].stack, ItemStack::new("minecraft:stick", refused));
                }
            }
            assert!(!game.world.block_entities.contains_key(&(0, 63, 0)));
            let sides = bin.sides.lock().unwrap();
            assert!(!sides.is_empty());
            assert!(sides.iter().all(|side| *side == Some(Direction::Up)));
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_mod_without_an_automation_provider_leaves_hoppers_on_the_vanilla_path() {
        use horizon_core::random::RandomSource as _;
        use mod_api::{
            self, ENTITY_ENERGY, Hooks, ModEntry, Mods, Registrar, has_entity_providers,
        };
        struct Energy;
        #[mod_api::__subscriptions]
        impl Hooks for Energy {
            fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
                r.entity_capability(&ENTITY_ENERGY, "minecraft:pig", |_, _, _| None);
            }
        }
        let mods: Mods = vec![ModEntry::new("energy", Box::new(Energy))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            assert!(!has_entity_providers(&ENTITY_AUTOMATION));
            with_game(|game| {
                hopper_with(game, "down", Some(ItemStack::new("minecraft:stick", 3)));
                pig(game, 700, 63.0);
                let mut before = game.world.random.clone();
                game.hopper_try_move_items((0, 64, 0));
                // Nothing to reach, and no draw made: no container entity was a candidate.
                assert_eq!(
                    hopper_slot(game),
                    Some(ItemStack::new("minecraft:stick", 3))
                );
                assert_eq!(
                    game.world.random.clone().next_int_bound(1 << 30),
                    before.next_int_bound(1 << 30)
                );
            });
        });
    }

    /// #932, review: an armor stand's equipment through the same handler —
    /// a hopper dressing a stand — journaled like a mob's, an abort putting
    /// #932, review: an armor stand's equipment through the same handler —
    /// a hopper dressing a stand — journaled like a mob's, an abort putting
    /// it back and a commit matching `set_equipment`.
    #[test]
    #[ignore = "needs prepared resources"]
    fn an_armor_stands_equipment_moves_through_the_handler() {
        use horizon_client::entities::ArmorStand;
        use horizon_core::geometry::Vec3;
        use mod_api::EquipmentSlot;
        with_game(|game| {
            game.armor_stands.push(ArmorStand::new(40, Vec3::ZERO, 0.0));
            game.armor_stands.push(ArmorStand::new(41, Vec3::ZERO, 0.0));
            let stand_slot =
                |game: &super::Game<'_>, id: usize| game.armor_stands[id].equipment.get(4).cloned();
            let chestplate = ItemResource::of(
                &ItemStack::new("minecraft:diamond_chestplate", 1)
                    .try_into_api()
                    .unwrap(),
            );
            let helmet = ItemResource::of(
                &ItemStack::new("minecraft:iron_helmet", 1)
                    .try_into_api()
                    .unwrap(),
            );

            // Seven slots; the stand wears what its `equippable` names for
            // it, one of it; a sword is not armour.
            {
                let host: &mut dyn ServerHost = game;
                let items = host
                    .entity(40)
                    .unwrap()
                    .capability(host, &ENTITY_ITEMS, ())
                    .unwrap();
                assert_eq!(items.size(host), 7);
                assert_eq!(items.capacity(host, 4, &chestplate), 1);
                assert_eq!(items.capacity(host, 5, &helmet), 1);
                assert_eq!(
                    items.capacity(
                        host,
                        4,
                        &ItemResource::of(
                            &ItemStack::new("minecraft:iron_sword", 1)
                                .try_into_api()
                                .unwrap()
                        )
                    ),
                    0
                );

                // An aborted insert leaves the stand as it was.
                let _ = host.transaction(|host, tx| {
                    assert_eq!(items.insert(host, tx, 4, &chestplate, 1), 1);
                    assert_eq!(items.amount(host, 4), 1);
                    Err::<(), ()>(())
                });
                assert_eq!(items.amount(host, 4), 0, "the abort put it back");
            }
            assert_eq!(
                stand_slot(game, 0),
                None,
                "the stand's own slot is empty again"
            );

            // A committed one matches `set_equipment`.
            {
                let host: &mut dyn ServerHost = game;
                let items = host
                    .entity(40)
                    .unwrap()
                    .capability(host, &ENTITY_ITEMS, ())
                    .unwrap();
                let _ = host
                    .transaction(|host, tx| Ok::<_, ()>(items.insert(host, tx, 4, &chestplate, 1)));
            }
            let through_setter = game.living(game.entity(41).unwrap()).unwrap();
            assert!(game.living_set_equipment(
                through_setter,
                EquipmentSlot::Chest,
                Some(ItemStack::new("minecraft:diamond_chestplate", 1)),
            ));
            assert_eq!(
                stand_slot(game, 0),
                stand_slot(game, 1),
                "the same worn stack as set_equipment's"
            );

            // And it hands it back out.
            {
                let host: &mut dyn ServerHost = game;
                let items = host
                    .entity(40)
                    .unwrap()
                    .capability(host, &ENTITY_ITEMS, ())
                    .unwrap();
                let came = host.transaction(|host, tx| {
                    Ok::<_, ()>(items.extract(host, tx, 4, &chestplate, 1))
                });
                assert_eq!(came, Ok(1));
                assert_eq!(items.amount(host, 4), 0);
            }
            assert_eq!(stand_slot(game, 0), None);
        });
    }

    /// A chunk at the origin, open to LAN, with a full water cauldron at (2, 64, 0).
    fn with_cauldron(game: &mut super::Game<'_>) -> mod_api::LevelRef {
        use horizon_client::chunk::Chunk;
        use mod_api::BlockUpdateFlags;
        open_to_lan(game);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let level = game.level("minecraft:overworld").unwrap();
        let state = horizon_world::chunk::BlockState {
            name: "minecraft:water_cauldron".to_owned(),
            properties: [("level".to_owned(), "3".to_owned())].into_iter().collect(),
        };
        let full = game.pack.assets.registry.resolve(&state).unwrap();
        let flags = BlockUpdateFlags::NEIGHBORS;
        assert_eq!(
            game.level_set_block(level, [2, 64, 0], full, flags),
            Ok(true)
        );
        level
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_scratch_stack_lives_in_its_transaction_and_keeps_its_item() {
        use mod_api::{FLUIDS, ITEM_FLUIDS, move_resources};
        with_game(|game| {
            let level = with_cauldron(game);
            let cauldron = game.world.block(2, 64, 0);
            let api = |name, count| ItemStack::new(name, count).try_into_api().unwrap();
            let bucket = api("minecraft:bucket", 1);
            let buckets = ItemResource::of(&bucket);
            let stick = ItemResource::of(&api("minecraft:stick", 1));
            let host: &mut dyn ServerHost = game;
            // Made in a transaction that aborts, it goes with it.
            let made = host.transaction(|host, tx| {
                Err::<(), _>(host.scratch_stack(tx, bucket.clone()).unwrap().unwrap())
            });
            let lost = made.unwrap_err();
            let gone = host.transaction(|host, tx| Ok::<_, ()>(host.scratch_take(tx, &lost)));
            assert_eq!(gone, Ok(None));
            let got = host.transaction(|host, tx| {
                let place = host.scratch_stack(tx, bucket.clone()).unwrap().unwrap();
                assert_eq!(place.insert(host, tx, &buckets, 2), 2);
                // Another item has nowhere to go: a scratch stack doesn't spill.
                assert_eq!(place.insert(host, tx, &stick, 1), 0);
                assert_eq!(place.amount(host), 3);
                // Filling it would make it a water bucket, another item: refused.
                let block = level.capability(host, &FLUIDS, [2, 64, 0], None).unwrap();
                let held = place.capability(host, &ITEM_FLUIDS).unwrap();
                let filled = move_resources(host, &block, &held, |_| true, 1000, tx);
                assert_eq!(filled, Ok(0));
                Ok::<_, ()>(host.scratch_take(tx, &place))
            });
            assert_eq!(got, Ok(Some(api("minecraft:bucket", 3))));
            assert_eq!(game.world.block(2, 64, 0), cauldron);
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_scratch_stack_merges_into_its_parent_and_ends_with_the_outermost() {
        with_game(|game| {
            open_to_lan(game);
            let bucket = ItemStack::new("minecraft:bucket", 1)
                .try_into_api()
                .unwrap();
            let buckets = ItemResource::of(&bucket);
            let host: &mut dyn ServerHost = game;
            let place = host.transaction(|host, _| {
                // Made and filled in a child that commits: the parent keeps it as it was left.
                let place = host
                    .transaction(|host, tx| {
                        let place = host.scratch_stack(tx, bucket.clone()).unwrap().unwrap();
                        assert_eq!(place.insert(host, tx, &buckets, 1), 1);
                        Ok::<_, ()>(place)
                    })
                    .unwrap();
                assert_eq!(place.amount(host), 2);
                // A child that writes it and aborts puts it back.
                let aborted = host.transaction(|host, tx| {
                    assert_eq!(place.insert(host, tx, &buckets, 3), 3);
                    assert_eq!(place.amount(host), 5);
                    Err::<(), _>(())
                });
                assert_eq!(aborted, Err(()));
                assert_eq!(place.amount(host), 2);
                Ok::<_, ()>(place)
            });
            // The outermost close ends it: nothing reads or takes it after.
            let place = place.unwrap();
            assert_eq!(place.amount(host), 0);
            let taken = host.transaction(|host, tx| Ok::<_, ()>(host.scratch_take(tx, &place)));
            assert_eq!(taken, Ok(None));
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn a_strict_slot_refuses_what_the_plain_slot_spreads() {
        use horizon_client::chunk::Chunk;
        use mod_api::BlockUpdateFlags;
        with_game(|game| {
            open_to_lan(game);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            let chest = game.pack.assets.registry.default_state("minecraft:chest");
            let flags = BlockUpdateFlags::NEIGHBORS;
            assert_eq!(
                game.level_set_block(level, [0, 64, 0], chest.unwrap(), flags),
                Ok(true)
            );
            let slots = |game: &mut super::Game<'_>| {
                let Some(BlockEntity::Chest(items)) =
                    game.world.block_entities.get_mut(&(0, 64, 0))
                else {
                    panic!("no chest")
                };
                items[..2].to_vec()
            };
            if let Some(BlockEntity::Chest(items)) = game.world.block_entities.get_mut(&(0, 64, 0))
            {
                items[0] = Some(ItemStack::new("minecraft:stick", 64));
            }
            let stick =
                ItemResource::of(&ItemStack::new("minecraft:stick", 1).try_into_api().unwrap());
            let place = ItemPlace::BlockEntity {
                level,
                pos: [0, 64, 0],
                side: None,
                slot: 0,
            };
            let strict = ItemPlace::Strict(Box::new(place.clone()));
            let host: &mut dyn ServerHost = game;
            let went = |host: &mut dyn ServerHost, at: &ItemPlace| {
                host.transaction(|host, tx| Ok::<_, ()>(at.insert(host, tx, &stick, 5)))
            };
            assert_eq!(went(&mut *host, &strict), Ok(0));
            assert_eq!(went(&mut *host, &place), Ok(5));
            assert_eq!(
                slots(game),
                [
                    Some(ItemStack::new("minecraft:stick", 64)),
                    Some(ItemStack::new("minecraft:stick", 5))
                ]
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources"]
    fn one_by_one_shows_one_bucket_of_sixteen_and_fills_one() {
        use mod_api::{FLUIDS, ITEM_FLUIDS, move_resources};
        with_game(|game| {
            let level = with_cauldron(game);
            let mut player = guest(1);
            player.inventory[0] = Some(ItemStack::new("minecraft:bucket", 16));
            game.lan.as_mut().unwrap().hub.guests.push(player);
            let one = ItemPlace::OneByOne(Box::new(ItemPlace::PlayerSlot {
                player: 123,
                slot: 0,
            }));
            let host: &mut dyn ServerHost = game;
            assert_eq!(one.amount(host), 1);
            let block = level.capability(host, &FLUIDS, [2, 64, 0], None).unwrap();
            let bucket = one.capability(host, &ITEM_FLUIDS).unwrap();
            let moved = host
                .transaction(|host, tx| move_resources(host, &block, &bucket, |_| true, 1000, tx));
            assert_eq!(moved, Ok(1000));
            let held = game.lan.as_ref().unwrap().hub.guests[0].inventory.clone();
            assert_eq!(held[0], Some(ItemStack::new("minecraft:bucket", 15)));
            let water = Some(ItemStack::new("minecraft:water_bucket", 1));
            assert!(held.contains(&water));
        });
    }
}
