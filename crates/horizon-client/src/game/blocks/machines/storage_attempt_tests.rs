//! #3720: real Game ticks, Parts storage and transaction close/restore traces.
use super::super::{Game, Pos};
use crate::game::native_entity_refs::tests::{run_fresh_cases, with_game};
use horizon_client::{
    chunk::Chunk,
    containers::{BlockEntity, ModBlockEntity, ModContainer},
    items::ItemStack,
};
use horizon_core::{
    geometry::Vec3,
    position::{BlockPos, ChunkPos},
};
use mod_api::*;
use std::sync::{Arc, Mutex};

const HOPPER: Pos = (8, 64, 8);
const ABOVE: Pos = (8, 65, 8);
const BELOW: Pos = (8, 63, 8);
const SINK: [i32; 3] = [10, 63, 8];
const OTHER: [i32; 3] = [11, 63, 8];
type Trace = Arc<Mutex<Vec<String>>>;

#[derive(Default)]
struct Rules {
    slots: Option<usize>,
    accept: Option<&'static str>,
    notify: bool,
    release_first: bool,
    nontransactional: bool,
    refuse_extract: bool,
    write_on_refuse: bool,
    rename_source_on_notice: bool,
    forward_to_vanilla: bool,
    external: Vec<Option<mod_api::ItemStack>>,
    source_seen: Vec<(u32, u32)>,
}

#[derive(Clone)]
struct Probe {
    trace: Trace,
    rules: Arc<Mutex<Rules>>,
}

struct Storage {
    probe: Probe,
    at: BlockSide,
}

impl Storage {
    fn entity(&self, host: &dyn Host) -> BlockEntityRef {
        host.block_entity(self.at.level, self.at.pos).unwrap()
    }
    fn stack(&self, host: &dyn Host, slot: usize) -> Option<mod_api::ItemStack> {
        let rules = self.probe.rules.lock().unwrap();
        if rules.nontransactional {
            return rules.external.get(slot).cloned().flatten();
        }
        drop(rules);
        self.entity(host).item_slot(host, slot as u32).unwrap()
    }
}

impl ResourceHandler<ItemResource> for Storage {
    fn size(&self, _: &mut dyn ServerHost) -> usize {
        self.probe.rules.lock().unwrap().slots.unwrap_or(2)
    }
    fn resource(&self, host: &mut dyn ServerHost, slot: usize) -> Option<ItemResource> {
        self.stack(host, slot).as_ref().map(ItemResource::of)
    }
    fn amount(&self, host: &mut dyn ServerHost, slot: usize) -> u32 {
        self.stack(host, slot).map_or(0, |stack| stack.count())
    }
    fn capacity(&self, _: &mut dyn ServerHost, _: usize, _: &ItemResource) -> u32 {
        64
    }
    fn insert(
        &self,
        _: &mut dyn ServerHost,
        _: &Transaction,
        _: usize,
        _: &ItemResource,
        _: u32,
    ) -> u32 {
        panic!("automation must use the storage's indexless insertion")
    }
    fn insert_any(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        assert_eq!(amount, 1);
        assert_eq!(host.transaction_depth(tx), Some(0));
        let item = mod_api::__private::item_key_id(resource.as_stack().item()).unwrap();
        self.probe
            .trace
            .lock()
            .unwrap()
            .push(format!("insert:{item}"));
        let hopper = self
            .at
            .level
            .capability(host, &ITEMS, [8, 64, 8], None)
            .unwrap();
        let source = (hopper.amount(host, 0), hopper.amount(host, 1));
        let (accept, notify, write_on_refuse, forward) = {
            let mut rules = self.probe.rules.lock().unwrap();
            rules.source_seen.push(source);
            (
                rules.accept == Some(item),
                rules.notify,
                rules.write_on_refuse,
                rules.forward_to_vanilla,
            )
        };
        if accept && forward {
            let sink = self.at.level.capability(host, &ITEMS, SINK, None).unwrap();
            return sink.insert_any(host, tx, resource, amount);
        }
        let entity = self.entity(host);
        if accept {
            entity
                .set_item_slot(host, tx, 0, Some(resource.as_stack().clone()))
                .unwrap();
        } else if write_on_refuse {
            entity.set_data_slot(host, 0, 1).unwrap();
        }
        if notify {
            entity.set_changed(host).unwrap();
        }
        u32::from(accept)
    }
    fn extract(
        &self,
        host: &mut dyn ServerHost,
        tx: &Transaction,
        slot: usize,
        resource: &ItemResource,
        amount: u32,
    ) -> u32 {
        assert_eq!(amount, 1);
        assert_eq!(host.transaction_depth(tx), Some(0));
        self.probe
            .trace
            .lock()
            .unwrap()
            .push(format!("extract:{slot}"));
        let (notify, release_first, nontransactional, refuse) = {
            let rules = self.probe.rules.lock().unwrap();
            (
                rules.notify,
                rules.release_first,
                rules.nontransactional,
                rules.refuse_extract,
            )
        };
        if refuse {
            return 0;
        }
        let Some(stack) = self.stack(host, slot) else {
            return 0;
        };
        assert_eq!(ItemResource::of(&stack), *resource);
        let remaining = stack.shrink(1);
        let entity = self.entity(host);
        if nontransactional {
            self.probe.rules.lock().unwrap().external[slot] = remaining;
        } else {
            entity
                .set_item_slot(host, tx, slot as u32, remaining)
                .unwrap();
        }
        if release_first {
            let hopper = self
                .at
                .level
                .capability(host, &ITEMS, [8, 64, 8], None)
                .unwrap();
            let held = hopper.resource(host, 0).unwrap();
            assert_eq!(hopper.extract(host, tx, 0, &held, 63), 63);
        }
        if notify {
            entity.set_changed(host).unwrap();
        }
        1
    }
}

#[mod_api::horizon_part]
impl BlockPart for Probe {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        let pos = (event.pos.x, event.pos.y, event.pos.z);
        if pos == HOPPER {
            self.trace
                .lock()
                .unwrap()
                .push(format!("mark:hopper:{:?}", host.transaction_lifecycle()));
        } else if pos == ABOVE || pos == BELOW {
            let entity = host
                .block_entity(event.level, [pos.0, pos.1, pos.2])
                .unwrap();
            let amount: u32 = (0..2)
                .map(|slot| {
                    entity
                        .item_slot(host, slot)
                        .ok()
                        .flatten()
                        .map_or(0, |s| s.count())
                })
                .sum();
            self.trace.lock().unwrap().push(format!(
                "notice:{amount}:{:?}",
                host.transaction_lifecycle()
            ));
            if pos == BELOW && self.rules.lock().unwrap().rename_source_on_notice {
                let source = host.block_entity(event.level, [8, 64, 8]).unwrap();
                assert!(
                    source
                        .set_name(host, Some("inside insertion notice"))
                        .unwrap()
                );
            }
        }
    }
    fn tick_block(&self, _: &mut dyn LevelWriteHost, _: &mut TickBlockEvent) -> Flow {
        self.trace.lock().unwrap().push("comparator".into());
        Flow::Continue
    }
}

#[derive(Clone)]
struct CommitMover(Probe);

#[mod_api::horizon_part]
impl BlockPart for CommitMover {
    fn contents_changed(&self, host: &mut dyn ServerHost, event: &mut ContentsChangedEvent) {
        if [event.pos.x, event.pos.y, event.pos.z] != SINK {
            return;
        }
        let source = event
            .level
            .capability(host, &ITEMS, [8, 64, 8], None)
            .unwrap();
        let amount = source.amount(host, 0);
        self.0.trace.lock().unwrap().push(format!(
            "sink-source:{amount}:{:?}",
            host.transaction_lifecycle()
        ));
        if let Some(resource) = source.resource(host, 0) {
            let other = event.level.capability(host, &ITEMS, OTHER, None).unwrap();
            host.transaction(|host, tx| {
                assert_eq!(source.extract(host, tx, 0, &resource, 1), 1);
                assert_eq!(other.insert_any(host, tx, &resource, 1), 1);
                Ok::<_, ()>(())
            })
            .unwrap();
        }
    }
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        for block in [
            "minecraft:barrel",
            "minecraft:chest",
            "minecraft:composter",
            "minecraft:decorated_pot",
        ] {
            let key = r.block_key(block);
            let probe = self.clone();
            r.block_capability_before_game(&ITEMS, &key, move |_, at, _, _| {
                if at.pos == SINK || at.pos == OTHER {
                    return None;
                }
                probe
                    .trace
                    .lock()
                    .unwrap()
                    .push(format!("face:{:?}", at.side.unwrap()));
                Some(Box::new(Storage {
                    probe: probe.clone(),
                    at,
                }))
            });
        }
        for block in ["minecraft:barrel", "minecraft:hopper"] {
            let key = r.block_key(block);
            r.block_part(key, self.clone());
        }
        let chest = r.block_key("minecraft:chest");
        r.block_part(chest, CommitMover(self.clone()));
        let comparator = r.block_key("minecraft:comparator");
        r.block_part(comparator, self.clone());
        r.entity_capability_before_game(
            &ENTITY_AUTOMATION,
            "minecraft:chest_minecart",
            |_, _, _| panic!("vanilla container entities must not ask their storage providers"),
        );
    }
}

fn fixture(name: &str, test: impl FnOnce(&mut Game<'_>, LevelRef, Probe)) {
    if run_fresh_cases(name, "HORIZON_STORAGE_ATTEMPTS_CHILD", &["game"]) {
        return;
    }
    let probe = Probe {
        trace: Arc::default(),
        rules: Arc::default(),
    };
    let mods: Mods = vec![ModEntry::new("storage_attempts", Box::new(probe.clone()))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            let level = game.level("minecraft:overworld").unwrap();
            place(game, HOPPER, "minecraft:hopper");
            game.world
                .block_entities
                .insert(HOPPER, BlockEntity::Chest(vec![None; 5]));
            let barrel = game
                .pack
                .assets
                .registry
                .default_state("minecraft:barrel")
                .unwrap();
            game.world.block_entities.set_mod_containers(
                [(
                    barrel.0,
                    Arc::new(ModContainer {
                        slots: 2,
                        inputs: vec![0, 1],
                        outputs: vec![0, 1],
                        faces: std::array::from_fn(|_| vec![0, 1]),
                        menu: None,
                    }),
                )]
                .into_iter()
                .collect(),
            );
            game.transactions.trace = Some(probe.trace.clone());
            probe.trace.lock().unwrap().clear();
            test(game, level, probe);
        })
    });
}

fn place(game: &mut Game<'_>, pos: Pos, block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(pos.0, pos.1, pos.2, state).unwrap();
}
fn storage(game: &mut Game<'_>, pos: Pos, items: Vec<Option<ItemStack>>) {
    place(game, pos, "minecraft:barrel");
    let mut entity = ModBlockEntity::new("storage_attempts:machine".into(), Default::default());
    entity.items = items;
    entity.data = vec![0];
    game.world
        .block_entities
        .insert(pos, BlockEntity::Mod(Box::new(entity)));
}
fn hopper(game: &mut Game<'_>, items: Vec<Option<ItemStack>>) {
    game.world
        .block_entities
        .insert(HOPPER, BlockEntity::Chest(items));
}
fn blocked_hopper(game: &mut Game<'_>) {
    hopper(
        game,
        vec![
            Some(ItemStack::new("minecraft:stone", 63)),
            Some(ItemStack::new("minecraft:stone", 64)),
            Some(ItemStack::new("minecraft:stone", 64)),
            Some(ItemStack::new("minecraft:stone", 64)),
            Some(ItemStack::new("minecraft:stone", 64)),
        ],
    );
}
fn trace(probe: &Probe) -> Vec<String> {
    std::mem::take(&mut *probe.trace.lock().unwrap())
}
fn count(game: &Game<'_>, pos: Pos, slot: usize) -> u32 {
    game.world.block_entities.get(&pos).unwrap().slots()[slot]
        .as_ref()
        .map_or(0, |s| s.count)
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn blocked_pull_ticks_extract_abort_restore_without_hopper_mark_or_cooldown() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::blocked_pull_ticks_extract_abort_restore_without_hopper_mark_or_cooldown",
        |game, _, probe| {
            blocked_hopper(game);
            place(game, BELOW, "minecraft:bedrock");
            storage(
                game,
                ABOVE,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    Some(ItemStack::new("minecraft:gold_ingot", 1)),
                ],
            );
            probe.rules.lock().unwrap().notify = true;
            for time in [1, 2] {
                game.world.game_time = time;
                game.tick_hopper(HOPPER);
                assert_eq!(
                    trace(&probe),
                    [
                        "face:Down",
                        "open",
                        "extract:0",
                        "notice:1:Open",
                        "abort",
                        "restore:8,65,8:0",
                        "open",
                        "extract:1",
                        "notice:1:Open",
                        "abort",
                        "restore:8,65,8:1"
                    ]
                );
                assert_eq!(game.hopper_cooldown(HOPPER), 0);
                assert_eq!((count(game, ABOVE, 0), count(game, ABOVE, 1)), (1, 1));
                assert_eq!(count(game, HOPPER, 0), 63);
            }
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn blocked_push_ticks_share_one_abort_and_restore_refused_source_slots() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::blocked_push_ticks_share_one_abort_and_restore_refused_source_slots",
        |game, _, probe| {
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    Some(ItemStack::new("minecraft:gold_ingot", 1)),
                    None,
                    None,
                    None,
                ],
            );
            storage(game, BELOW, vec![None; 2]);
            place(game, ABOVE, "minecraft:bedrock");
            probe.rules.lock().unwrap().notify = true;
            for time in [1, 2] {
                game.world.game_time = time;
                game.tick_hopper(HOPPER);
                let mut seen = trace(&probe);
                let mut restored = seen.split_off(7);
                restored.sort();
                assert_eq!(
                    seen,
                    [
                        "face:Up",
                        "open",
                        "insert:minecraft:diamond",
                        "notice:0:Open",
                        "insert:minecraft:gold_ingot",
                        "notice:0:Open",
                        "abort"
                    ]
                );
                assert_eq!(restored, ["restore:8,64,8:0", "restore:8,64,8:1"]);
                assert_eq!(game.hopper_cooldown(HOPPER), 0);
                assert_eq!((count(game, HOPPER, 0), count(game, HOPPER, 1)), (1, 1));
            }
            assert_eq!(
                probe.rules.lock().unwrap().source_seen,
                [(0, 1), (1, 0), (0, 1), (1, 0)]
            );
            probe.rules.lock().unwrap().accept = Some("minecraft:gold_ingot");
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "notice:0:Open",
                    "insert:minecraft:gold_ingot",
                    "notice:1:Open",
                    "commit",
                    "mark:hopper:None"
                ]
            );
            assert_eq!((count(game, HOPPER, 0), count(game, HOPPER, 1)), (1, 0));
            assert_eq!(count(game, BELOW, 0), 1);
            assert_eq!(game.hopper_cooldown(HOPPER), 8);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn push_then_pull_commit_in_one_tick_with_only_the_ordered_hopper_marks() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::push_then_pull_commit_in_one_tick_with_only_the_ordered_hopper_marks",
        |game, _, probe| {
            hopper(game, vec![Some(ItemStack::new("minecraft:stone", 64)); 5]);
            storage(game, BELOW, vec![None; 2]);
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:stone", 1)), None],
            );
            probe.rules.lock().unwrap().accept = Some("minecraft:stone");
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:stone",
                    "commit",
                    "face:Down",
                    "open",
                    "extract:0",
                    "mark:hopper:Open",
                    "commit",
                    "mark:hopper:None"
                ]
            );
            assert_eq!(game.hopper_cooldown(HOPPER), 8);
            assert_eq!(
                (
                    count(game, HOPPER, 0),
                    count(game, BELOW, 0),
                    count(game, ABOVE, 0)
                ),
                (64, 1, 0)
            );
            for time in 1..8 {
                game.world.game_time = time;
                game.tick_hopper(HOPPER);
            }
            assert!(trace(&probe).is_empty());
            assert_eq!(game.hopper_cooldown(HOPPER), 1);
            game.tick_hopper(HOPPER);
            assert_eq!(game.hopper_cooldown(HOPPER), 8);
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:stone",
                    "commit",
                    "face:Down",
                    "mark:hopper:None"
                ]
            );
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn pull_chooses_the_first_slot_after_extract_and_not_an_existing_stack() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::pull_chooses_the_first_slot_after_extract_and_not_an_existing_stack",
        |game, _, probe| {
            blocked_hopper(game);
            place(game, BELOW, "minecraft:bedrock");
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:diamond", 1)), None],
            );
            probe.rules.lock().unwrap().release_first = true;
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Down",
                    "open",
                    "extract:0",
                    "mark:hopper:Open",
                    "commit",
                    "mark:hopper:RootClosing",
                    "mark:hopper:None"
                ]
            );
            assert_eq!(count(game, HOPPER, 0), 1);
            assert_eq!(
                game.world.block_entities.get(&HOPPER).unwrap().slots()[0]
                    .as_ref()
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:diamond"
            );
            // A lower empty index wins over a compatible stack at a later index.
            hopper(
                game,
                vec![
                    None,
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    None,
                    None,
                    None,
                ],
            );
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:diamond", 1)), None],
            );
            probe.rules.lock().unwrap().release_first = false;
            game.hoppers.get_mut(&HOPPER).unwrap().cooldown = 0;
            game.tick_hopper(HOPPER);
            assert_eq!((count(game, HOPPER, 0), count(game, HOPPER, 1)), (1, 1));
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn full_storage_and_fully_stacked_hopper_are_the_only_capacity_shortcuts() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::full_storage_and_fully_stacked_hopper_are_the_only_capacity_shortcuts",
        |game, _, probe| {
            hopper(game, vec![Some(ItemStack::new("minecraft:stone", 64)); 5]);
            storage(
                game,
                BELOW,
                vec![Some(ItemStack::new("minecraft:diamond", 64)); 2],
            );
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:diamond", 1)), None],
            );
            game.tick_hopper(HOPPER);
            assert_eq!(trace(&probe), ["face:Up"]);
            assert_eq!(game.hopper_cooldown(HOPPER), 0);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn dropper_commits_refused_insertion_and_its_storage_notifications() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::dropper_commits_refused_insertion_and_its_storage_notifications",
        |game, _, probe| {
            place(game, HOPPER, "minecraft:dropper");
            let state = game.world.block(8, 64, 8);
            let down = game.with_props(state, &[("facing", "down")]).unwrap();
            game.world.set(8, 64, 8, down).unwrap();
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 2)),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            );
            storage(game, BELOW, vec![None; 2]);
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.notify = true;
                rules.write_on_refuse = true;
            }
            let state = game.world.block(8, 64, 8);
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "notice:0:Open",
                    "commit"
                ]
            );
            assert_eq!((count(game, HOPPER, 0), count(game, BELOW, 0)), (2, 0));
            let level = game.level("minecraft:overworld").unwrap();
            let entity = game.block_entity(level, [8, 63, 8]).unwrap();
            assert_eq!(entity.data_slots(game), Some(vec![1]));
            probe.rules.lock().unwrap().accept = Some("minecraft:diamond");
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "notice:1:Open",
                    "commit"
                ]
            );
            assert_eq!((count(game, HOPPER, 0), count(game, BELOW, 0)), (1, 1));
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn vanilla_container_provider_and_entity_never_ask_their_storage() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::vanilla_container_provider_and_entity_never_ask_their_storage",
        |game, _, probe| {
            use crate::game::minecarts::{CartKind, Minecart};
            for target in ["minecraft:chest", "minecraft:composter", "minecraft:air"] {
                hopper(game, vec![None; 5]);
                game.hoppers.clear();
                place(game, ABOVE, target);
                game.world.block_entities.remove(&ABOVE);
                game.carts.clear();
                match target {
                    "minecraft:chest" => {
                        game.world.block_entities.insert(
                            ABOVE,
                            BlockEntity::Chest(vec![
                                Some(ItemStack::new("minecraft:stone", 1));
                                27
                            ]),
                        );
                    }
                    "minecraft:composter" => {
                        let state = game.world.block(8, 65, 8);
                        let full = game.with_props(state, &[("level", "8")]).unwrap();
                        game.world.set(8, 65, 8, full).unwrap();
                    }
                    _ => {
                        let mut cart =
                            Minecart::new(700, CartKind::Chest, Vec3::new(8.5, 65.0, 8.5));
                        cart.items[0] = Some(ItemStack::new("minecraft:stone", 1));
                        game.carts.push(cart);
                    }
                }
                game.tick_hopper(HOPPER);
                assert_eq!(count(game, HOPPER, 0), 1, "{target}");
                assert_eq!(
                    trace(&probe),
                    ["mark:hopper:None", "mark:hopper:None"],
                    "{target}"
                );
            }
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn failed_block_pull_stops_but_minecart_falls_back_to_item_entities() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::failed_block_pull_stops_but_minecart_falls_back_to_item_entities",
        |game, _, probe| {
            use crate::game::minecarts::{CartKind, Minecart};
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:diamond", 1)), None],
            );
            probe.rules.lock().unwrap().refuse_extract = true;
            game.spawn_item(
                ItemStack::new("minecraft:stone", 1),
                [8.5, 64.5, 8.5],
                [0.0; 3],
                0,
            );
            game.tick_hopper(HOPPER);
            assert_eq!(game.dropped.len(), 1);
            assert_eq!(count(game, HOPPER, 0), 0);
            assert_eq!(trace(&probe), ["face:Down", "open", "extract:0", "abort"]);
            let mut cart = Minecart::new(701, CartKind::Hopper, Vec3::new(8.5, 64.0, 8.5));
            game.hopper_cart_suck(&mut cart);
            assert!(game.dropped.is_empty());
            assert_eq!(cart.items[0], Some(ItemStack::new("minecraft:stone", 1)));
            assert_eq!(trace(&probe), ["face:Down", "open", "extract:0", "abort"]);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn nontransactional_storage_loses_a_blocked_extraction_on_abort() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::nontransactional_storage_loses_a_blocked_extraction_on_abort",
        |game, _, probe| {
            blocked_hopper(game);
            place(game, BELOW, "minecraft:bedrock");
            storage(game, ABOVE, vec![None; 2]);
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.nontransactional = true;
                rules.external = vec![
                    Some(
                        mod_api::ItemStack::new(
                            mod_api::__private::item_key("minecraft:diamond").unwrap(),
                            1,
                        )
                        .unwrap(),
                    ),
                    None,
                ];
            }
            game.tick_hopper(HOPPER);
            assert_eq!(trace(&probe), ["face:Down", "open", "extract:0", "abort"]);
            assert!(probe.rules.lock().unwrap().external[0].is_none());
            assert_eq!(count(game, HOPPER, 0), 63);
            assert_eq!(game.hopper_cooldown(HOPPER), 0);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn extract_notifications_cross_signal_each_tick_and_comparator_ticks_every_second_tick() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::extract_notifications_cross_signal_each_tick_and_comparator_ticks_every_second_tick",
        |game, _, probe| {
            use horizon_core::ticks::TickTypeId;
            use horizon_world::level::LevelAccess as _;
            // Real scheduled ticks require a lit, surrounded, entity-ready block-ticking chunk.
            Arc::make_mut(game.world.chunks.get_mut(&(0, 0)).unwrap()).lit = true;
            for x in -1..=1 {
                for z in -1..=1 {
                    if (x, z) != (0, 0) {
                        game.world
                            .chunks
                            .insert((x, z), Arc::new(Chunk::empty(x, z)));
                    }
                }
            }
            game.tickets
                .update(Some(horizon_client::ticket_levels::Sources {
                    forced: vec![(0, 0)],
                    ..Default::default()
                }));
            assert!(game.with_level(|_, level, _| level.is_ticking_chunk(ChunkPos::new(0, 0))));
            blocked_hopper(game);
            // A different item keeps the non-full hopper unable to merge the source stone.
            if let Some(stack) = game
                .world
                .block_entities
                .get_mut(&HOPPER)
                .unwrap()
                .slots_mut()[0]
                .as_mut()
            {
                stack.item = "minecraft:dirt".into();
            }
            place(game, BELOW, "minecraft:bedrock");
            storage(
                game,
                ABOVE,
                vec![Some(ItemStack::new("minecraft:stone", 64)), None],
            );
            probe.rules.lock().unwrap().slots = Some(1);
            game.world.block_entities.set_mod_containers(
                [(
                    game.world.block(8, 65, 8).0,
                    Arc::new(ModContainer {
                        slots: 1,
                        inputs: vec![0],
                        outputs: vec![0],
                        faces: std::array::from_fn(|_| vec![0]),
                        menu: None,
                    }),
                )]
                .into_iter()
                .collect(),
            );
            let comparator = BlockPos::new(9, 65, 8);
            place(game, (9, 64, 8), "minecraft:stone");
            place(game, (9, 65, 8), "minecraft:comparator");
            let state = game.world.block(9, 65, 8);
            let state = game.with_props(state, &[("facing", "west")]).unwrap();
            game.world.set(9, 65, 8, state).unwrap();
            game.with_level(|b, level, ticks| b.scheduled_block_tick(level, ticks, comparator));
            assert_eq!(game.block_ticks.comparator_outputs[&comparator], 15);
            let kind = TickTypeId(u32::from(game.res.behaviours().block(state).0));
            probe.rules.lock().unwrap().notify = true;
            trace(&probe);
            let mut comparator_ticks = Vec::new();
            for time in 1..=6 {
                game.world.game_time = time;
                game.with_level(|b, level, ticks| b.tick_scheduled(level, ticks));
                let before = trace(&probe);
                if before.iter().any(|event| event == "comparator") {
                    comparator_ticks.push(time);
                }
                game.tick_hopper(HOPPER);
                assert_eq!(
                    trace(&probe),
                    [
                        "face:Down",
                        "open",
                        "extract:0",
                        "notice:63:Open",
                        "abort",
                        "restore:8,65,8:0"
                    ]
                );
                assert_eq!(game.hopper_cooldown(HOPPER), 0);
                assert_eq!(count(game, ABOVE, 0), 64);
                assert_eq!(
                    game.res
                        .facts()
                        .get(game.world.block(9, 65, 8))
                        .name
                        .as_ref(),
                    "minecraft:comparator"
                );
                assert!(
                    game.block_ticks.blocks.has_scheduled(comparator, kind),
                    "extract notice keeps real comparator work queued"
                );
            }
            assert_eq!(comparator_ticks, [3, 5]);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn a_blocked_pull_slot_aborts_before_the_next_slot_commits() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::a_blocked_pull_slot_aborts_before_the_next_slot_commits",
        |game, _, probe| {
            blocked_hopper(game);
            place(game, BELOW, "minecraft:bedrock");
            storage(
                game,
                ABOVE,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    Some(ItemStack::new("minecraft:stone", 1)),
                ],
            );
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Down",
                    "open",
                    "extract:0",
                    "abort",
                    "restore:8,65,8:0",
                    "open",
                    "extract:1",
                    "mark:hopper:Open",
                    "commit",
                    "mark:hopper:None"
                ]
            );
            assert_eq!(
                (
                    count(game, ABOVE, 0),
                    count(game, ABOVE, 1),
                    count(game, HOPPER, 0)
                ),
                (1, 0, 64)
            );
            assert_eq!(game.hopper_cooldown(HOPPER), 8);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn pushes_use_only_the_opposite_face_for_each_hopper_facing() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::pushes_use_only_the_opposite_face_for_each_hopper_facing",
        |game, _, probe| {
            place(game, ABOVE, "minecraft:bedrock");
            let base = game.world.block(8, 64, 8);
            for (facing, target, expected) in [
                ("down", BELOW, "face:Up"),
                ("north", (8, 64, 7), "face:South"),
                ("south", (8, 64, 9), "face:North"),
                ("west", (7, 64, 8), "face:East"),
                ("east", (9, 64, 8), "face:West"),
            ] {
                let state = game.with_props(base, &[("facing", facing)]).unwrap();
                game.world.set(8, 64, 8, state).unwrap();
                hopper(
                    game,
                    vec![
                        Some(ItemStack::new("minecraft:stone", 1)),
                        None,
                        None,
                        None,
                        None,
                    ],
                );
                game.hoppers.clear();
                storage(game, target, vec![None; 2]);
                probe.rules.lock().unwrap().accept = Some("minecraft:stone");
                game.tick_hopper(HOPPER);
                assert_eq!(
                    trace(&probe),
                    [
                        expected,
                        "open",
                        "insert:minecraft:stone",
                        "commit",
                        "mark:hopper:None"
                    ]
                );
                assert_eq!(count(game, target, 0), 1);
            }
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn refused_push_callback_journals_source_without_losing_the_restored_item() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::refused_push_callback_journals_source_without_losing_the_restored_item",
        |game, level, probe| {
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    None,
                    None,
                    None,
                    None,
                ],
            );
            storage(game, BELOW, vec![None; 2]);
            place(game, ABOVE, "minecraft:bedrock");
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.notify = true;
                rules.rename_source_on_notice = true;
            }
            game.tick_hopper(HOPPER);
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "notice:0:Open",
                    "abort",
                    "restore:8,64,8:0"
                ]
            );
            assert_eq!(count(game, HOPPER, 0), 1);
            assert_eq!(count(game, BELOW, 0), 0);
            assert_eq!(game.hopper_cooldown(HOPPER), 0);
            assert_eq!(
                game.block_entity(level, [8, 64, 8]).unwrap().name(game),
                None
            );
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn dropper_removes_accepted_source_before_a_real_root_commit_callback() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::dropper_removes_accepted_source_before_a_real_root_commit_callback",
        |game, _, probe| {
            place(game, HOPPER, "minecraft:dropper");
            let state = game.world.block(8, 64, 8);
            let state = game.with_props(state, &[("facing", "down")]).unwrap();
            game.world.set(8, 64, 8, state).unwrap();
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:diamond", 1)),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            );
            storage(game, BELOW, vec![None; 2]);
            for pos in [SINK, OTHER] {
                let key = (pos[0], pos[1], pos[2]);
                place(game, key, "minecraft:chest");
                game.world
                    .block_entities
                    .insert(key, BlockEntity::Chest(vec![None; 27]));
            }
            {
                let mut rules = probe.rules.lock().unwrap();
                rules.accept = Some("minecraft:diamond");
                rules.forward_to_vanilla = true;
            }
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            assert_eq!(
                trace(&probe),
                [
                    "face:Up",
                    "open",
                    "insert:minecraft:diamond",
                    "commit",
                    "sink-source:0:RootClosing"
                ]
            );
            assert_eq!(count(game, HOPPER, 0), 0);
            assert_eq!(count(game, (10, 63, 8), 0), 1);
            assert_eq!(count(game, (11, 63, 8), 0), 0);
        },
    );
}

#[test]
#[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES; isolated scratch process"]
fn dropper_routes_saved_pot_directly_even_with_a_storage_provider() {
    fixture(
        "game::machines::native_items::storage_attempt_tests::dropper_routes_saved_pot_directly_even_with_a_storage_provider",
        |game, _, probe| {
            use crate::game::spill;
            use horizon_core::nbt::Tag;
            let pot = [9, 64, 8];
            place(game, HOPPER, "minecraft:dropper");
            let state = game.world.block(8, 64, 8);
            let state = game.with_props(state, &[("facing", "east")]).unwrap();
            game.world.set(8, 64, 8, state).unwrap();
            hopper(
                game,
                vec![
                    Some(ItemStack::new("minecraft:stone", 2)),
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                    None,
                ],
            );
            place(game, (9, 64, 8), "minecraft:decorated_pot");
            spill::put_saved_entity(
                &mut game.world,
                pot,
                Tag::compound([
                    ("id", Tag::String("minecraft:decorated_pot".into())),
                    ("x", Tag::Int(9)),
                    ("y", Tag::Int(64)),
                    ("z", Tag::Int(8)),
                    (
                        "item",
                        horizon_client::save::item_to_nbt(&ItemStack::new("minecraft:stone", 63)),
                    ),
                ]),
            );
            let dropped = game.dropped.len();
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            let saved = spill::saved_entity(&game.world, pot).unwrap();
            assert_eq!(
                horizon_client::save::item_from_nbt(saved.get("item").unwrap())
                    .unwrap()
                    .count,
                64
            );
            assert_eq!(count(game, HOPPER, 0), 1);
            assert_eq!(game.dropped.len(), dropped);
            assert!(
                trace(&probe).is_empty(),
                "the pot's provider was never asked"
            );
            game.dispense_from(BlockPos::new(8, 64, 8), state).unwrap();
            assert_eq!(
                count(game, HOPPER, 0),
                1,
                "a full pot refuses on its direct path"
            );
            assert_eq!(game.dropped.len(), dropped);
            assert!(trace(&probe).is_empty());
        },
    );
}
