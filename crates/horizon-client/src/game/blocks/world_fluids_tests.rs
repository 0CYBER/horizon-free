//! A mod's world pickup and placement of fluid in a real game: decided at the call, its block
//! kept by the transaction, its updates, drops and effects at the outermost commit, nothing of
//! it after an abort.
use crate::game::{Game, dispense_fixture, native_entity_refs::tests::with_game};
use horizon_client::{chunk::Chunk, containers::BlockEntity};
use horizon_world::registry::BlockStateId;
use mod_api::{FluidStack, HostTransaction, LevelRef, ServerHost};
use std::sync::Arc;

const AT: [i32; 3] = [2, 65, 2];

/// A game with one empty chunk, a dirt floor at y 64 and the level's handle.
fn with_level(test: impl FnOnce(&mut Game<'_>, LevelRef)) {
    with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        let dirt = state(game, "minecraft:dirt");
        for x in 0..5 {
            for z in 0..5 {
                game.world.set(x, 64, z, dirt).unwrap();
            }
        }
        let level = game.level("minecraft:overworld").unwrap();
        test(game, level);
    });
}

fn state(game: &Game<'_>, name: &str) -> BlockStateId {
    game.pack.assets.registry.default_state(name).unwrap()
}

fn key(id: &str) -> mod_api::FluidKey {
    mod_api::__private::fluid_key(id).unwrap()
}

fn block(game: &Game<'_>) -> BlockStateId {
    game.world.block(AT[0], AT[1], AT[2])
}

#[test]
#[ignore = "daily"]
fn a_second_pickup_of_one_source_in_a_transaction_gets_nothing() {
    with_level(|game, level| {
        let water = state(game, "minecraft:water");
        game.world.set(AT[0], AT[1], AT[2], water).unwrap();
        let host: &mut dyn ServerHost = game;
        let picked: Result<_, ()> = host.transaction(|host, tx| {
            Ok((
                level.pickup_fluid(host, AT, None, tx),
                level.pickup_fluid(host, AT, None, tx),
            ))
        });
        let one = FluidStack::new(key("minecraft:water"), 1000);
        assert_eq!(picked, Ok((Some(one), None)));
        assert_eq!(block(game), state(game, "minecraft:air"));
    });
}

#[test]
#[ignore = "daily"]
fn an_aborted_placement_over_a_skull_leaves_it_and_drops_nothing() {
    with_level(|game, level| {
        let skull = state(game, "minecraft:skeleton_skull");
        game.world.set(AT[0], AT[1], AT[2], skull).unwrap();
        let pos = (AT[0], AT[1], AT[2]);
        game.world
            .block_entities
            .insert(pos, BlockEntity::Skull(Default::default()));
        let kept = format!("{:?}", game.world.block_entities.get(&pos));
        assert!(
            game.world.block_entities.get(&pos).is_some(),
            "the skull has its block entity"
        );
        let items = game.dropped.len();
        dispense_fixture::start_recording();
        let root = game.transaction_open();
        let water = key("minecraft:water");
        assert!(game.level_place_fluid(level, AT, water, None, true, root.transaction()));
        // Inside the transaction the skull is gone and the water is there.
        assert_eq!(block(game), state(game, "minecraft:water"));
        assert!(game.world.block_entities.get(&pos).is_none());
        game.transaction_close(&root, false);
        assert_eq!(block(game), skull);
        assert_eq!(format!("{:?}", game.world.block_entities.get(&pos)), kept);
        assert_eq!(game.dropped.len(), items);
        assert_eq!(dispense_fixture::stop_recording(), Vec::<String>::new());
    });
}

#[test]
#[ignore = "daily"]
fn a_committed_pour_breaks_what_it_replaces_then_sends_its_events_in_order() {
    with_level(|game, level| {
        crate::game::native_item_places::tests::open_to_lan(game);
        let poppy = state(game, "minecraft:poppy");
        game.world.set(AT[0], AT[1], AT[2], poppy).unwrap();
        let items = game.dropped.len();
        game.world.block_log = Some(Vec::new());
        game.creatures.lan_sounds.clear();
        dispense_fixture::start_recording();
        let host: &mut dyn ServerHost = game;
        let placed: Result<bool, ()> = host.transaction(|host, tx| {
            Ok(level.place_fluid(host, AT, key("minecraft:lava"), None, false, tx))
        });
        assert_eq!(placed, Ok(true));
        let lines = dispense_fixture::stop_recording();
        // One write tells the clients of the block, once.
        let told = game.world.block_log.as_ref().unwrap();
        assert_eq!(told.iter().filter(|p| **p == (2, 65, 2)).count(), 1);
        // The bucket's empty sound, queued once at the commit.
        let sounds: Vec<_> = game
            .creatures
            .lan_sounds
            .iter()
            .map(|s| (s.sound.clone(), s.at))
            .collect();
        assert_eq!(
            sounds,
            [(
                "minecraft:item.bucket.empty_lava".to_owned(),
                [2.5, 65.5, 2.5]
            )]
        );
        assert_eq!(block(game), state(game, "minecraft:lava"));
        // The poppy's drop, made at the call, spawned at the commit.
        assert_eq!(game.dropped.len(), items + 1);
        assert_eq!(&*game.dropped[items].stack.item, "minecraft:poppy");
        assert_eq!(
            lines,
            [
                format!("L 2001 2,65,2 {}", poppy.0),
                "G 2,65,2 minecraft:block_destroy".to_owned(),
                "G 2,65,2 minecraft:fluid_place".to_owned(),
            ]
        );
    });
}

#[test]
#[ignore = "daily"]
fn the_call_time_answer_stands_when_the_block_changes_before_the_commit() {
    with_level(|game, level| {
        let poppy = state(game, "minecraft:poppy");
        game.world.set(AT[0], AT[1], AT[2], poppy).unwrap();
        let stone = state(game, "minecraft:stone");
        let items = game.dropped.len();
        dispense_fixture::start_recording();
        let root = game.transaction_open();
        let water = key("minecraft:water");
        let placed = game.level_place_fluid(level, AT, water, None, false, root.transaction());
        // A later write in the same transaction: the commit doesn't decide again.
        game.journal_set_block(AT, stone);
        game.transaction_close(&root, true);
        assert!(placed);
        let lines = dispense_fixture::stop_recording();
        assert_eq!(block(game), stone);
        assert_eq!(game.dropped.len(), items + 1);
        assert_eq!(
            lines,
            [
                format!("L 2001 2,65,2 {}", poppy.0),
                "G 2,65,2 minecraft:block_destroy".to_owned(),
                "G 2,65,2 minecraft:fluid_place".to_owned(),
            ]
        );
    });
}

/// Listens to `block_drops`, keeps the block it reads where the drops are made and cancels
/// the drops when asked to.
struct SeenBlocks(Arc<std::sync::Mutex<Vec<Option<String>>>>, bool);
#[mod_api::__subscriptions]
impl mod_api::Hooks for SeenBlocks {
    fn block_drops(
        &self,
        host: &dyn mod_api::LevelHost,
        _: &mut mod_api::BlockDropsEvent<'_>,
    ) -> mod_api::Flow {
        let block = host
            .block_state(AT)
            .and_then(|state| host.state_block(state).map(str::to_owned));
        self.0.lock().unwrap().push(block);
        if self.1 {
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

#[test]
#[ignore = "daily"]
fn a_drop_listener_sees_the_old_block_and_an_abort_undoes_the_loot_it_unpacked() {
    use horizon_client::chunk::PendingContainerLoot;
    use horizon_core::nbt::Tag;
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "seen_blocks",
        Box::new(SeenBlocks(seen.clone(), false)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_level(|game, level| {
            game.options.dedicated = Some(Vec::new());
            // A skull with a container's block entity, its loot not yet rolled: breaking it
            // unpacks the loot into the entity.
            let skull = state(game, "minecraft:skeleton_skull");
            game.world.set(AT[0], AT[1], AT[2], skull).unwrap();
            let pos = (AT[0], AT[1], AT[2]);
            game.world
                .block_entities
                .insert(pos, BlockEntity::Chest(vec![None; 27]));
            let loot = PendingContainerLoot {
                table: Tag::String("minecraft:chests/simple_dungeon".into()),
                seed: Some(Tag::Long(42)),
            };
            game.world.set_block_entity_extras(
                pos,
                horizon_client::world::BlockEntityExtras {
                    source: None,
                    pending_loot: Some(loot),
                },
            );
            let root = game.transaction_open();
            let water = key("minecraft:water");
            assert!(game.level_place_fluid(level, AT, water, None, false, root.transaction()));
            game.transaction_close(&root, false);
            // The listener ran while the broken block was still there.
            assert_eq!(
                *seen.lock().unwrap(),
                [Some("minecraft:skeleton_skull".to_owned())]
            );
            assert_eq!(block(game), skull);
            let Some(BlockEntity::Chest(slots)) = game.world.block_entities.get(&pos) else {
                panic!("the container's block entity is back");
            };
            assert!(
                slots.iter().all(Option::is_none),
                "its loot is unrolled again"
            );
            assert!(game.world.block_entity_extras(pos).pending_loot.is_some());
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_cancelled_drop_keeps_the_replacement() {
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "cancelled_drops",
        Box::new(SeenBlocks(seen.clone(), true)),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_level(|game, level| {
            game.options.dedicated = Some(Vec::new());
            let poppy = state(game, "minecraft:poppy");
            game.world.set(AT[0], AT[1], AT[2], poppy).unwrap();
            let items = game.dropped.len();
            let root = game.transaction_open();
            let water = key("minecraft:water");
            assert!(game.level_place_fluid(level, AT, water, None, false, root.transaction()));
            game.transaction_close(&root, true);
            // The listener saw the poppy and cancelled: no drop, but the water is there.
            assert_eq!(*seen.lock().unwrap(), [Some("minecraft:poppy".to_owned())]);
            assert_eq!(game.dropped.len(), items);
            assert_eq!(block(game), state(game, "minecraft:water"));
        });
    });
}

#[test]
#[ignore = "daily"]
fn a_later_write_that_puts_the_block_back_keeps_its_block_entity_at_the_commit() {
    with_level(|game, level| {
        let skull = state(game, "minecraft:skeleton_skull");
        game.world.set(AT[0], AT[1], AT[2], skull).unwrap();
        let pos = (AT[0], AT[1], AT[2]);
        game.world
            .block_entities
            .insert(pos, BlockEntity::Skull(Default::default()));
        let root = game.transaction_open();
        let water = key("minecraft:water");
        assert!(game.level_place_fluid(level, AT, water, None, false, root.transaction()));
        assert!(game.world.block_entities.get(&pos).is_none());
        game.journal_set_block(AT, skull);
        game.transaction_close(&root, true);
        assert_eq!(block(game), skull);
        assert!(matches!(
            game.world.block_entities.get(&pos),
            Some(BlockEntity::Skull(_))
        ));
    });
}
