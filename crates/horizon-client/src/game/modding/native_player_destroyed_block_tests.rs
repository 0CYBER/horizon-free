//! `player_destroyed_block` heard through the game's own break path: a creative and a survival
//! break are heard once with the removed state; a break `break_block` cancels and a piston's
//! are not heard.
use super::native_entity_refs::tests::with_game;
use horizon_client::{command::args::GameMode, items::ItemStack, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, position::BlockPos};

use mod_api::{BreakBlockEvent, Flow, Hooks, PlayerDestroyedBlockEvent, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

/// One heard break: the position, the state's block name and the block entity's saved data.
type Heard = ([i32; 3], String, Option<mod_api::Compound>);

struct Probe {
    heard: Arc<Mutex<Vec<Heard>>>,
    /// The hook and the `minecraft:mined` awards, in the order they came.
    order: Arc<Mutex<Vec<String>>>,
    cancel_break: Arc<AtomicBool>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn player_stat_award(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::PlayerStatAwardEvent<'_>,
    ) -> Flow {
        if event.stat_type.as_str() == "minecraft:mined" {
            let award = format!("{} {}", event.stat_type, event.stat);
            self.order.lock().unwrap().push(award);
        }
        Flow::Continue
    }
    fn break_block(&self, _: &mut dyn ServerHost, _: &mut BreakBlockEvent<'_>) -> Flow {
        if self.cancel_break.load(Ordering::Relaxed) {
            Flow::Cancel
        } else {
            Flow::Continue
        }
    }
    fn player_destroyed_block(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerDestroyedBlockEvent,
    ) {
        // The state is the block's default state, which the test placed.
        let name = ["minecraft:stone", "minecraft:chest"]
            .into_iter()
            .find(|name| host.block_state_id(name) == Some(event.state))
            .expect("the broken state is a default state the test placed")
            .to_owned();
        // The level is already past the removal: the block is gone from it.
        let level = host.level("minecraft:overworld").expect("the overworld");
        assert_eq!(
            level.block_state(host, event.pos),
            host.block_state_id("minecraft:air"),
        );
        assert!(
            host.players().acting_player().is_some(),
            "the breaker is the acting player"
        );
        self.order.lock().unwrap().push("destroyed".into());
        self.heard
            .lock()
            .unwrap()
            .push((event.pos, name, event.block_entity.clone()));
    }
}

fn place(game: &mut super::Game<'_>, pos: [i32; 3], block: &str) {
    let state = game.pack.assets.registry.default_state(block).unwrap();
    game.world.set(pos[0], pos[1], pos[2], state).unwrap();
}

fn block(game: &super::Game<'_>, pos: [i32; 3]) -> String {
    let state = game.world.block(pos[0], pos[1], pos[2]);
    game.res.facts().get(state).name.to_string()
}

/// A server's game with its own player standing in the origin chunk, as the native player-break
/// tests set one up: their handle.
fn with_player(
    game: &mut super::Game<'_>,
    mode: GameMode,
    held: Option<ItemStack>,
) -> mod_api::PlayerRef {
    game.options.dedicated = Some(Vec::new());
    let _ = game
        .block_ticks
        .add_empty_chunk(horizon_core::position::ChunkPos::new(0, 0));
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
    game.lan_acting_uuid = Some(123);
    game.lan_acting_entity = Some(20);
    game.lan_acting_name = Some("Breaker".into());
    game.lan_acting_permission = Some(0);
    game.set_mode(mode);
    game.inventory[game.selected] = held;
    // The acting player as a hook would name them: this server's entity 20.
    let epoch = game.native_lifecycle.epoch().unwrap();
    mod_api::__private::game::player_ref(mod_api::EntityRef::new(epoch, 20), 123)
}

#[test]
#[ignore = "daily"]
fn a_players_completed_break_is_heard_once_with_the_removed_state() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let order = Arc::new(Mutex::new(Vec::new()));
    let cancel_break = Arc::new(AtomicBool::new(false));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "destroyed",
        Box::new(Probe {
            heard: heard.clone(),
            order: order.clone(),
            cancel_break: cancel_break.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let take = || std::mem::take(&mut *heard.lock().unwrap());

            // Survival, with the right tool: stone drops, and the break is heard.
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_pickaxe", 1)),
            );
            let stone = [6, 64, 4];
            place(game, stone, "minecraft:stone");
            assert!(game.player_destroy_block(player, stone, false));
            assert_eq!(block(game, stone), "minecraft:air");
            assert_eq!(take(), [(stone, "minecraft:stone".to_owned(), None)]);
            // Heard before `playerDestroy` counts the block mined; the count comes after.
            assert_eq!(
                std::mem::take(&mut *order.lock().unwrap()),
                ["destroyed", "minecraft:mined minecraft:stone"]
            );

            // Creative: no drops and an early return in vanilla, still heard once. A chest
            // carries its saved contents.
            let player = with_player(game, GameMode::Creative, None);
            let chest = [7, 64, 4];
            place(game, chest, "minecraft:chest");
            let mut contents = vec![None; 27];
            contents[0] = Some(ItemStack::new("minecraft:diamond", 3));
            game.world.block_entities.insert(
                (chest[0], chest[1], chest[2]),
                horizon_client::containers::BlockEntity::Chest(contents),
            );
            assert!(game.player_destroy_block(player, chest, false));
            assert_eq!(block(game, chest), "minecraft:air");
            let heard_now = take();
            assert_eq!(heard_now.len(), 1);
            assert_eq!(
                (heard_now[0].0, heard_now[0].1.as_str()),
                (chest, "minecraft:chest")
            );
            let data = heard_now[0]
                .2
                .as_ref()
                .expect("the removed chest's saved data");
            let Some(mod_api::Tag::List(items)) = data.get(&"Items".into()) else {
                panic!("a chest saves its Items");
            };
            assert_eq!(items.len(), 1, "the one stack the chest held");
            let mod_api::Tag::Compound(stack) = &items[0] else {
                panic!("a saved stack is a compound");
            };
            assert_eq!(stack.get(&"count".into()), Some(&mod_api::Tag::Int(3)));
            assert_eq!(
                stack.get(&"id".into()),
                Some(&mod_api::Tag::String("minecraft:diamond".into()))
            );

            // A break `break_block` cancels keeps the block and is not heard.
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_pickaxe", 1)),
            );
            cancel_break.store(true, Ordering::Relaxed);
            let kept = [8, 64, 4];
            place(game, kept, "minecraft:stone");
            assert!(!game.player_destroy_block(player, kept, false));
            assert_eq!(block(game, kept), "minecraft:stone");
            assert!(take().is_empty());
            cancel_break.store(false, Ordering::Relaxed);

            // A piston pushing a torch breaks it from the level's side: not heard.
            let registry = game.pack.assets.registry;
            let piston = registry.default_state("minecraft:piston").unwrap();
            let piston = game.with_props(piston, &[("facing", "east")]).unwrap();
            let torch = registry.default_state("minecraft:torch").unwrap();
            let power = registry.default_state("minecraft:redstone_block").unwrap();
            game.world.set(10, 70, 4, piston).unwrap();
            game.world.set(11, 70, 4, torch).unwrap();
            game.with_level(|b, level, ticks| {
                b.set_block(level, ticks, BlockPos::new(9, 70, 4), power, 3);
                b.run_block_events(level, ticks);
            });
            assert_ne!(
                block(game, [11, 70, 4]),
                "minecraft:torch",
                "the torch was pushed away"
            );
            assert!(take().is_empty(), "a level-side break is not a player's");
        });
    });
}

/// What one `player_destroyed_block` saw of the level and its player while it ran.
#[derive(Debug, PartialEq)]
struct Seen {
    pos: [i32; 3],
    /// `BlockStateParser.serialize` of the state the hook was told.
    state: String,
    /// Whether the acting player was the fake that broke the block.
    by_breaker: bool,
    /// Item entities in the level at that moment.
    items: usize,
    /// Experience points lying in orbs around the block at that moment.
    orb_points: i64,
    /// Whether the event carried block entity data.
    block_entity: bool,
}

/// Item entities in the overworld.
fn items(host: &dyn mod_api::Host) -> usize {
    let level = host.level("minecraft:overworld").expect("the overworld");
    level
        .all_entities(host)
        .unwrap_or_default()
        .into_iter()
        .filter(|entity| {
            entity.kind(host).map(|kind| kind.id(host)).as_deref() == Some("minecraft:item")
        })
        .count()
}

/// Experience points in the orbs within 3 blocks of `pos`.
fn orb_points(host: &dyn ServerHost, pos: [i32; 3]) -> i64 {
    let at = pos.map(f64::from);
    host.experience_orbs(
        "minecraft:overworld",
        at.map(|v| v - 3.0),
        at.map(|v| v + 4.0),
    )
    .iter()
    .map(|orb| i64::from(orb.value) * i64::from(orb.count))
    .sum()
}

/// Gives every break 7 experience points, and records and optionally rewrites what
/// `player_destroyed_block` sees.
struct OrderProbe {
    breaker: mod_api::PlayerRef,
    seen: Arc<Mutex<Vec<Seen>>>,
    /// A block `break_block` puts in place of the target, without cancelling the break.
    replace: Option<&'static str>,
    /// A block `player_destroyed_block` writes where the broken one stood.
    write: Option<&'static str>,
}

/// Sets the overworld's block at `pos` to `block`'s default state from a listener.
fn set(host: &mut dyn ServerHost, pos: [i32; 3], block: &str) {
    let state = host.block_state_id(block).expect("a known block");
    let level = host.level("minecraft:overworld").expect("the overworld");
    assert!(
        matches!(
            level.set_block(host, pos, state, mod_api::BlockUpdateFlags::CLIENTS),
            Ok(true)
        ),
        "the level is writable from the hook"
    );
}

#[mod_api::__subscriptions]
impl Hooks for OrderProbe {
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        *event.exp_to_drop = Some(7);
        if let Some(block) = self.replace {
            set(host, event.pos, block);
        }
        Flow::Continue
    }
    fn player_destroyed_block(
        &self,
        host: &mut dyn ServerHost,
        event: &mut PlayerDestroyedBlockEvent,
    ) {
        let acting = host.players().acting_player();
        let seen = Seen {
            pos: event.pos,
            state: host.block_state_string(event.state).unwrap_or_default(),
            by_breaker: acting == Some(self.breaker),
            items: items(&*host),
            orb_points: orb_points(&*host, event.pos),
            block_entity: event.block_entity.is_some(),
        };
        self.seen.lock().unwrap().push(seen);
        if let Some(block) = self.write {
            set(host, event.pos, block);
        }
    }
}

/// Runs `test` on a server with a fake player (`fake_action_tests::fake`) holding `item` in
/// survival, with an `OrderProbe` that puts `replace` and writes `write`: what the probe saw.
fn with_order_probe(
    item: &str,
    replace: Option<&'static str>,
    write: Option<&'static str>,
    test: impl FnOnce(&mut super::Game<'_>, mod_api::PlayerRef),
) -> Vec<Seen> {
    let seen = Arc::new(Mutex::new(Vec::new()));
    with_game(|game| {
        let (player, ..) = super::native_server::fake_action_tests::fake(game, Some(item), 0);
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
            "order",
            Box::new(OrderProbe {
                breaker: player,
                seen: seen.clone(),
                replace,
                write,
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || test(game, player));
    });
    std::mem::take(&mut *seen.lock().unwrap())
}

#[test]
#[ignore = "daily"]
fn a_fakes_break_action_is_heard_before_the_drops_and_the_experience() {
    use mod_api::PlayerAction;
    let stone = [1, 63, 0];
    let mut after = None;
    let seen = with_order_probe("minecraft:diamond_pickaxe", None, None, |game, player| {
        assert_eq!(
            game.player_perform_action(player, PlayerAction::BreakBlock { pos: stone }),
            Ok(mod_api::ActionOutcome::Consumed)
        );
        assert_eq!(block(game, stone), "minecraft:air");
        after = Some((items(game), orb_points(game, stone)));
    });
    // Heard once, as the fake's own break, before `playerDestroy`: no dropped item and no
    // experience yet. (A fake keeps no statistics; the mined count's order is checked with
    // a real player in `a_players_completed_break_is_heard_once_with_the_removed_state`.)
    assert_eq!(
        seen,
        [Seen {
            pos: stone,
            state: "minecraft:stone".into(),
            by_breaker: true,
            items: 0,
            orb_points: 0,
            block_entity: false,
        }]
    );
    // Then the break finishes: the cobblestone dropped and the 7 points `break_block` asked
    // for.
    assert_eq!(after, Some((1, 7)));
}

#[test]
#[ignore = "daily"]
fn a_listener_writes_the_level_where_the_block_stood() {
    use mod_api::PlayerAction;
    let stone = [1, 63, 0];
    let mut after = None;
    let seen = with_order_probe(
        "minecraft:diamond_pickaxe",
        None,
        Some("minecraft:glass"),
        |game, player| {
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: stone }),
                Ok(mod_api::ActionOutcome::Consumed)
            );
            after = Some((items(game), orb_points(game, stone)));
            assert_eq!(block(game, stone), "minecraft:glass");
        },
    );
    assert_eq!(seen.len(), 1);
    // The write does not undo the break: the stone's drops and experience still come.
    assert_eq!(after, Some((1, 7)));
}

#[test]
#[ignore = "daily"]
fn shears_disarm_a_tripwire_and_the_hook_is_told_the_disarmed_string() {
    use mod_api::PlayerAction;
    let wire = [1, 64, 0];
    let seen = with_order_probe("minecraft:shears", None, None, |game, player| {
        place(game, wire, "minecraft:tripwire");
        assert_eq!(
            game.player_perform_action(player, PlayerAction::BreakBlock { pos: wire }),
            Ok(mod_api::ActionOutcome::Consumed)
        );
        assert_eq!(block(game, wire), "minecraft:air");
    });
    // TripWireBlock.playerWillDestroy sets the disarmed string in the level (flags 260)
    // before the removal, and the hook is told what the removal took out. (Vanilla's
    // Block.destroy is passed the armed one: playerWillDestroy returns the state it was given.)
    assert_eq!(
        seen.iter().map(|s| s.state.as_str()).collect::<Vec<_>>(),
        [
            "minecraft:tripwire[attached=false,disarmed=true,east=false,north=false,powered=false,south=false,west=false]"
        ]
    );
}

#[test]
#[ignore = "daily"]
fn a_target_break_block_replaced_is_told_with_its_own_block_entity() {
    use mod_api::PlayerAction;
    let target = [1, 63, 0];
    let seen = with_order_probe(
        "minecraft:diamond_pickaxe",
        Some("minecraft:chest"),
        None,
        |game, player| {
            assert_eq!(
                game.player_perform_action(player, PlayerAction::BreakBlock { pos: target }),
                Ok(mod_api::ActionOutcome::Consumed)
            );
            assert_eq!(block(game, target), "minecraft:air");
        },
    );
    // `break_block` put a chest where the stone was and let the break go on: the removal
    // takes the chest out, and the hook hears the chest with the chest's block entity, not
    // the stone the break started on.
    assert_eq!(seen.len(), 1);
    assert_eq!(
        (seen[0].state.as_str(), seen[0].block_entity),
        (
            "minecraft:chest[facing=north,type=single,waterlogged=false]",
            true
        )
    );
}

/// What `block_drops` named of the breaker and the block entity, and the chest's saved
/// `Items` count read through the block entity handle while the event ran.
type DropsSeen = (
    Option<u128>,
    Option<mod_api::EntityRef>,
    Option<usize>,
    bool,
);

/// Listens to `block_drops` and keeps the block entity handle it was given.
struct DropsProbe {
    seen: Arc<Mutex<Vec<DropsSeen>>>,
    handle: Arc<Mutex<Option<mod_api::BlockEntityRef>>>,
    /// Panics after reading, so the break unwinds through the event.
    panic: bool,
}

#[mod_api::__subscriptions]
impl Hooks for DropsProbe {
    fn block_drops(
        &self,
        level: &dyn mod_api::LevelHost,
        event: &mut mod_api::BlockDropsEvent<'_>,
    ) -> Flow {
        let items = event
            .block_entity
            .and_then(|entity| level.block_entity_data(entity))
            .and_then(|data| match data.get(&"Items".into()) {
                Some(mod_api::Tag::List(items)) => Some(items.len()),
                _ => None,
            });
        *self.handle.lock().unwrap() = event.block_entity;
        // The named-chest use: copy the chest's custom name onto the chest item it drops.
        let name = event
            .block_entity
            .and_then(|entity| level.block_entity_data(entity))
            .and_then(|data| data.get(&"CustomName".into()).cloned());
        if let Some(name) = name {
            for drop in event.drops.iter_mut().filter(|drop| {
                mod_api::__private::item_key_id(drop.stack.item()).unwrap() == "minecraft:chest"
            }) {
                let mut patch = mod_api::Compound::default();
                patch.insert("minecraft:custom_name".into(), name.clone());
                drop.stack.set_components(patch).unwrap();
            }
        }
        self.seen.lock().unwrap().push((
            event.breaker,
            event.breaker_entity,
            items,
            event.block_entity.is_some(),
        ));
        if self.panic {
            panic!("a listener fails after reading the broken block entity");
        }
        Flow::Continue
    }
}

fn put_chest(game: &mut super::Game<'_>, pos: [i32; 3]) {
    place(game, pos, "minecraft:chest");
    let mut contents = vec![None; 27];
    contents[0] = Some(ItemStack::new("minecraft:diamond", 3));
    game.world.block_entities.insert(
        (pos[0], pos[1], pos[2]),
        horizon_client::containers::BlockEntity::Chest(contents),
    );
}

/// A player's survival break names the player as `breaker` and `breaker_entity`, and its
/// `block_entity` reads the chest the removal took out of the world; the handle is stale once
/// the break completes.
#[test]
#[ignore = "daily"]
fn block_drops_reads_the_chest_a_players_break_removed_and_the_handle_goes_stale() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "drops-probe",
        Box::new(DropsProbe {
            seen: seen.clone(),
            handle: handle.clone(),
            panic: false,
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_axe", 1)),
            );
            let chest = [7, 64, 4];
            put_chest(game, chest);
            game.world.block_names.insert(
                (chest[0], chest[1], chest[2]),
                mod_api::Tag::String("Loot Chest".into()),
            );
            game.dropped.clear();
            assert!(game.player_destroy_block(player, chest, false));
            assert_eq!(block(game, chest), "minecraft:air");
            // The listener read the name the removal clears from the level and put it on the
            // chest item the break dropped.
            let named = game
                .dropped
                .iter()
                .filter(|item| &*item.stack.item == "minecraft:chest")
                .map(|item| {
                    item.stack
                        .extra
                        .as_ref()
                        .and_then(|extra| extra.custom_name_plain())
                })
                .collect::<Vec<_>>();
            assert_eq!(named, [Some("Loot Chest".to_owned())]);
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 1);
            let (breaker, breaker_entity, items, has_block_entity) = &heard[0];
            assert_eq!(*breaker, Some(123));
            assert_eq!(
                breaker_entity.map(|entity| entity.id()),
                Some(20),
                "the player's own entity"
            );
            assert!(*has_block_entity);
            assert_eq!(
                *items,
                Some(1),
                "the chest's one stack, read during the event"
            );
            // After the break completes the handle is stale and nothing stays held.
            let stale = handle.lock().unwrap().expect("the event's handle");
            game.with_level(|_, level, _| {
                assert!(mod_api::LevelHost::block_entity_data(level, stale).is_none());
            });
            assert!(
                game.world
                    .block_entities
                    .broken_generation(&(chest[0], chest[1], chest[2]))
                    .is_none()
            );

            // A block with no block entity gives none.
            let dirt = [8, 64, 4];
            place(game, dirt, "minecraft:dirt");
            assert!(game.player_destroy_block(player, dirt, false));
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 1);
            assert!(!heard[0].3);
        });
    });
}

/// A ravager's break (a mob's, not a player's) names the ravager as `breaker_entity` and no
/// `breaker` UUID; a break the level makes (no entity) names neither.
#[test]
#[ignore = "daily"]
fn block_drops_names_the_mob_that_broke_the_block() {
    use horizon_client::mobs::Mob;
    let seen = Arc::new(Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "drops-probe",
        Box::new(DropsProbe {
            seen: seen.clone(),
            handle: Arc::default(),
            panic: false,
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            with_player(game, GameMode::Survival, None);
            let kind = game
                .res
                .mob_types()
                .get("minecraft:ravager")
                .unwrap()
                .clone();
            game.mobs
                .push(Mob::new(51, kind, Vec3::new(5.5, 64.0, 5.5), 0.0));
            let ravager = game.entity(51).expect("the ravager's handle");
            let leaves = game
                .pack
                .assets
                .registry
                .default_state("minecraft:oak_leaves")
                .unwrap();
            game.world.set(9, 64, 4, leaves).unwrap();
            game.entity_block_dropped([9, 64, 4], leaves, Some(51), Vec::new());
            game.world.set(10, 64, 4, leaves).unwrap();
            game.entity_block_dropped([10, 64, 4], leaves, None, Vec::new());
            let heard = std::mem::take(&mut *seen.lock().unwrap());
            assert_eq!(heard.len(), 2);
            assert_eq!((heard[0].0, heard[0].1), (None, Some(ravager)));
            assert_eq!((heard[1].0, heard[1].1), (None, None));
        });
    });
}

/// A listener that panics inside the event leaves nothing held: the handle it was given is stale
/// after the break unwinds.
#[test]
#[ignore = "daily"]
fn block_drops_holds_nothing_after_a_listener_panics() {
    let seen = Arc::new(Mutex::new(Vec::new()));
    let handle = Arc::new(Mutex::new(None));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "drops-probe",
        Box::new(DropsProbe {
            seen: seen.clone(),
            handle: handle.clone(),
            panic: true,
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            let player = with_player(
                game,
                GameMode::Survival,
                Some(ItemStack::new("minecraft:diamond_axe", 1)),
            );
            let chest = [7, 64, 4];
            put_chest(game, chest);
            // The platform may contain the panic or let it unwind through the break: either way.
            let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                game.player_destroy_block(player, chest, false)
            }));
            assert_eq!(
                seen.lock().unwrap().len(),
                1,
                "the listener ran and read the chest"
            );
            assert_eq!(seen.lock().unwrap()[0].2, Some(1));
            let stale = handle.lock().unwrap().expect("the event's handle");
            game.with_level(|_, level, _| {
                assert!(mod_api::LevelHost::block_entity_data(level, stale).is_none());
            });
            assert!(
                game.world
                    .block_entities
                    .broken_generation(&(chest[0], chest[1], chest[2]))
                    .is_none()
            );
        });
    });
}
