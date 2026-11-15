#[cfg(test)]
thread_local! { static STACK_CONVERSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

use super::*;
use crate::game::native_ticks;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    pub(in crate::game) fn item_update_round(&mut self) -> native_ticks::TickRound {
        // EntityTickEvent.Pre for the items about to tick, and the Post after them.
        let mut round = self.native_tick_round();
        let merging = self.remote.is_none();
        // One read says whether any item part listens; with none, no stack is looked at.
        let update_hook =
            merging && mod_api::__private::has_part_hook(mod_api::HookId::OnEntityItemUpdate);
        if round.listening() || update_hook {
            let ticking = self.entity_ticking_area();
            let mut i = 0;
            while let Some(item) = self.dropped.get(i) {
                let (id, x, z) = (
                    item.id,
                    item.position.x.floor() as i32,
                    item.position.z.floor() as i32,
                );
                if !(merging && self.joined_during_entity_pass(id)) && ticking(x, z) {
                    let kind = native_ticks::TickKind::Fixed("minecraft:item");
                    if self.native_tick_pre_batch(&mut round, id, kind)
                        && update_hook
                        && self.item_update_halts(i)
                    {
                        round.halt(id);
                    }
                }
                i += 1;
            }
        }
        round
    }

    /// `IItemExtension.onEntityItemUpdate`: a dropped item entity holding
    /// `stack` is about to tick (`ItemEntity.tick`). Set `canceled` and its
    /// movement and merging are skipped for this tick. Server-side.
    pub(in crate::game) fn native_on_entity_item_update(
        &mut self,
        call: mod_api::__private::OnEntityItemUpdateCall<'_>,
    ) {
        self.fire_on_entity_item_update(call);
    }

    /// Asks the parts of the item entity at `index` of `dropped` whether its tick
    /// ends here. An item no part is attached to is told nothing: its stack is not
    /// converted and no handle is made.
    pub(in crate::game) fn item_update_halts(&mut self, index: usize) -> bool {
        let item = &self.dropped[index];
        if !mod_api::__private::item_listens(mod_api::HookId::OnEntityItemUpdate, &item.stack.item)
        {
            return false;
        }
        let id = item.id;
        #[cfg(test)]
        STACK_CONVERSIONS.with(|count| count.set(count.get() + 1));
        let Ok(stack) = item.stack.try_into_api() else {
            return false;
        };
        let Some(handle) = self.entity(id).and_then(|entity| self.item_entity(entity)) else {
            return false;
        };
        let mut canceled = false;
        self.native_on_entity_item_update(mod_api::__private::OnEntityItemUpdateCall {
            stack: &stack,
            item: handle,
            canceled: &mut canceled,
        });
        canceled
    }
}

#[cfg(test)]
mod on_entity_item_update_tests {
    use super::*;
    use horizon_client::chunk::Chunk;
    use mod_api::{
        Flow, Hooks, ItemPart, OnEntityItemUpdateEvent, Registrar, RegistrationContext, ServerHost,
    };
    use std::sync::{Arc, Mutex};

    /// An item part that writes the entity's pickup delay to 7 on every update and
    /// records the entities it was told about; `cancels` ends the tick.
    #[derive(Clone)]
    struct Update {
        item: &'static str,
        cancels: bool,
        remove: bool,
        mixed: bool,
        heard: Arc<Mutex<Vec<u32>>>,
    }

    #[mod_api::horizon_part]
    impl ItemPart for Update {
        fn on_entity_item_update(
            &self,
            host: &mut dyn ServerHost,
            event: &mut OnEntityItemUpdateEvent<'_>,
        ) -> Flow {
            ORDER.with(|order| {
                order
                    .borrow_mut()
                    .push((event.item.entity().id(), "update"))
            });
            self.heard.lock().unwrap().push(event.item.entity().id());
            if self.remove {
                assert!(
                    event
                        .item
                        .entity()
                        .remove(host, mod_api::RemovalReason::Discarded)
                );
                assert_eq!(event.item.entity().position(host), None);
                assert!(!event.item.set_pickup_delay(host, 7));
                return Flow::Continue;
            }
            assert!(event.item.set_pickup_delay(host, 7));
            if self.cancels && (!self.mixed || event.item.entity().id().is_multiple_of(2)) {
                Flow::Cancel
            } else {
                Flow::Continue
            }
        }
    }

    struct Mod(Vec<Update>);

    thread_local! { static ORDER: std::cell::RefCell<Vec<(u32, &'static str)>> = const { std::cell::RefCell::new(Vec::new()) }; }

    #[mod_api::__subscriptions]
    impl Hooks for Mod {
        fn entity_tick_listened(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::EntityTickListenedEvent,
        ) {
            event.listened = true;
        }
        fn entity_tick_pre(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::EntityTickPreEvent<'_>,
        ) -> Flow {
            ORDER.with(|order| order.borrow_mut().push((event.entity, "pre")));
            Flow::Continue
        }
        fn entity_tick_post(
            &self,
            _: &mut dyn ServerHost,
            event: &mut mod_api::EntityTickPostEvent<'_>,
        ) {
            ORDER.with(|order| order.borrow_mut().push((event.entity, "post")));
        }
        fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
            for part in &self.0 {
                let key = r.item_key(part.item);
                r.item_part(key, part.clone());
            }
        }
    }

    /// A serving level with a stone floor at y 63 and nothing dropped.
    fn floor(game: &mut Game<'_>) {
        game.options.dedicated = Some(Vec::new());
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        game.world = horizon_client::world::World::new(game.res.facts().air, 1);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        for x in 0..16 {
            for z in 0..16 {
                game.world.set(x, 63, z, stone).unwrap();
            }
        }
        game.dropped.clear();
    }

    /// One stack of `item` at rest on the floor, a pickup delay of 20 ticks.
    fn rest(game: &mut Game<'_>, item: &str, x: f64, z: f64) -> u32 {
        game.spawn_item(ItemStack::new(item, 1), [x, 64.0, z], [0.0; 3], 20);
        game.ticked_entities_through = game.next_entity;
        game.dropped.last().unwrap().id
    }

    fn counts(game: &Game<'_>, ids: &[u32]) -> Vec<u32> {
        game.dropped
            .iter()
            .filter(|item| ids.contains(&item.id))
            .map(|item| item.stack.count)
            .collect()
    }

    /// An item the part of its own item cancels does not age, merge or lose pickup
    /// delay for the tick, and keeps the part's write; an item whose part does not
    /// cancel ticks as vanilla does after the write; an item without a part ticks
    /// and merges as before and is never told.
    #[test]
    #[ignore = "daily"]
    fn a_canceling_part_holds_its_own_item_and_nothing_else() {
        crate::game::in_own_process(
            "game::native_hooks::on_entity_item_update::on_entity_item_update_tests::a_canceling_part_holds_its_own_item_and_nothing_else",
            a_canceling_part_holds_its_own_item_and_nothing_else_isolated,
        );
    }

    fn a_canceling_part_holds_its_own_item_and_nothing_else_isolated() {
        use crate::game::native_entity_refs::tests::with_game;
        let iron = Update {
            item: "minecraft:iron_ingot",
            cancels: true,
            remove: false,
            mixed: false,
            heard: Default::default(),
        };
        let gold = Update {
            item: "minecraft:gold_ingot",
            cancels: false,
            remove: false,
            mixed: false,
            heard: Default::default(),
        };
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "item_update",
            Box::new(Mod(vec![
                iron.clone(),
                gold.clone(),
                Update {
                    item: "minecraft:emerald",
                    cancels: false,
                    remove: true,
                    mixed: false,
                    heard: Default::default(),
                },
                Update {
                    item: "minecraft:copper_ingot",
                    cancels: true,
                    remove: false,
                    mixed: true,
                    heard: Default::default(),
                },
            ])),
        )]
        .into();
        mod_platform::testing::with_mods(mods.clone(), || {
            crate::game::api_values::install_parts(&mods).unwrap();
            with_game(|game| {
                floor(game);
                // Two of each of iron and diamond side by side, a gold one apart.
                let irons = [
                    rest(game, "minecraft:iron_ingot", 2.5, 2.5),
                    rest(game, "minecraft:iron_ingot", 2.7, 2.5),
                ];
                let diamonds = [
                    rest(game, "minecraft:diamond", 10.5, 2.5),
                    rest(game, "minecraft:diamond", 10.7, 2.5),
                ];
                let gold_id = rest(game, "minecraft:gold_ingot", 6.5, 10.5);

                ORDER.with(|order| order.borrow_mut().clear());
                STACK_CONVERSIONS.with(|count| count.set(0));
                game.tick_items();
                assert_eq!(STACK_CONVERSIONS.with(|count| count.get()), 3);
                for id in irons.into_iter().chain([gold_id]) {
                    assert_eq!(
                        ORDER.with(|order| order
                            .borrow()
                            .iter()
                            .filter(|(entity, _)| *entity == id)
                            .map(|(_, phase)| *phase)
                            .collect::<Vec<_>>()),
                        ["pre", "update", "post"]
                    );
                }
                for id in irons {
                    let held = game.dropped.iter().find(|item| item.id == id).unwrap();
                    // Nothing of the entity's own tick ran: not its age, nor its delay's
                    // countdown; only the part's write is there.
                    assert_eq!((held.age, held.ticks, held.pickup_delay), (0, 0, 7));
                }
                for id in diamonds {
                    let ticked = game.dropped.iter().find(|item| item.id == id).unwrap();
                    assert_eq!((ticked.age, ticked.pickup_delay), (1, 19));
                }
                let gold_item = game.dropped.iter().find(|item| item.id == gold_id).unwrap();
                // The part wrote 7, and then the item's own tick counted down.
                assert_eq!((gold_item.age, gold_item.pickup_delay), (1, 6));

                // A resting item merges within 40 ticks (`ItemEntity.tick`): the part-less
                // pair does, the pair held by its part never does.
                for _ in 0..40 {
                    game.tick_items();
                }
                assert_eq!(counts(game, &irons), [1, 1]);
                assert_eq!(counts(game, &diamonds), [2]);
                assert!(
                    game.dropped
                        .iter()
                        .filter(|item| irons.contains(&item.id))
                        .all(|item| item.age == 0)
                );

                // Told once per tick of each item of its own: 41 ticks, two iron items; 41, one gold item.
                let mut heard = iron.heard.lock().unwrap().clone();
                heard.sort_unstable();
                let mut expected: Vec<u32> = irons.iter().flat_map(|id| [*id; 41]).collect();
                expected.sort_unstable();
                assert_eq!(heard, expected);
                assert_eq!(*gold.heard.lock().unwrap(), vec![gold_id; 41]);
                let removed = rest(game, "minecraft:emerald", 12.5, 12.5);
                game.tick_items();
                assert_eq!(
                    ORDER.with(|order| order
                        .borrow()
                        .iter()
                        .filter(|(id, _)| *id == removed)
                        .map(|(_, phase)| *phase)
                        .collect::<Vec<_>>()),
                    ["pre", "update", "post"]
                );
                assert!(game.entity(removed).is_none());
                game.native_removals_flush();
                assert!(!game.dropped.iter().any(|item| item.id == removed));
                let pair = [
                    rest(game, "minecraft:copper_ingot", 8.5, 8.5),
                    rest(game, "minecraft:copper_ingot", 8.7, 8.5),
                ];
                for _ in 0..80 {
                    game.tick_items();
                }
                assert_eq!(counts(game, &pair), [2]);
            });
        });
    }

    /// With no part attached to any item, every item ticks and merges as it did before the hook.
    #[test]
    #[ignore = "daily"]
    fn without_parts_items_tick_and_merge() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            floor(game);
            let ids = [
                rest(game, "minecraft:iron_ingot", 2.5, 2.5),
                rest(game, "minecraft:iron_ingot", 2.7, 2.5),
            ];
            STACK_CONVERSIONS.with(|count| count.set(0));
            game.tick_items();
            assert_eq!(STACK_CONVERSIONS.with(|count| count.get()), 0);
            assert!(
                game.dropped
                    .iter()
                    .all(|item| (item.age, item.pickup_delay) == (1, 19))
            );
            for _ in 0..40 {
                game.tick_items();
            }
            assert_eq!(counts(game, &ids), [2]);
        });
    }
}
