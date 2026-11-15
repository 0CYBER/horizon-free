//! `ItemPart::empty_contents` on a dispenser and a player's pour: an item with the Part is
//! emptied as a bucket, the Part placing the contents itself through the host.
use crate::game::api_values::TryIntoApi;
use horizon_client::{chunk::Chunk, items::ItemStack};
use horizon_core::geometry::Direction;
use mod_api::*;
use std::sync::{Arc, Mutex};

/// The dispenser, and the block it faces, where the Part pours.
const DISPENSER: [i32; 3] = [5, 64, 5];
const TARGET: [i32; 3] = [6, 64, 5];

/// What the Part does when asked.
#[derive(Default)]
struct Plan {
    /// Its answer.
    answer: Option<bool>,
    /// Places a honey block at the target, as a jar of honey would, in this level.
    place: Option<LevelRef>,
    /// Also puts a stick in the dispenser's slot 1: a write the decision read.
    meddle: bool,
    /// Replaces the player's hand with a stone through the living equipment write.
    replace_hand: bool,
    /// Whether each ask named a player and a clicked block.
    asks: Vec<(bool, bool)>,
}

#[derive(Clone, Default)]
struct Jar(Arc<Mutex<Plan>>);

#[mod_api::__subscriptions]
impl Hooks for Jar {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:stick");
        r.item_part(key, self.clone());
    }
}
#[mod_api::horizon_part]
impl ItemPart for Jar {
    fn empty_contents(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EmptyContentsEvent<'_>,
    ) -> Option<bool> {
        let mut plan = self.0.lock().unwrap();
        plan.asks
            .push((event.player.is_some(), event.hit.is_some()));
        if let Some(level) = plan.place {
            let honey = host.block_state_id("minecraft:honey_block").unwrap();
            let placed = level.set_block(host, TARGET, honey, BlockUpdateFlags::NEIGHBORS);
            assert!(placed.unwrap());
            if plan.meddle {
                let slot = ItemPlace::BlockEntity {
                    level,
                    pos: DISPENSER,
                    side: None,
                    slot: 1,
                };
                let stick = ItemResource::of(event.stack);
                let went =
                    host.transaction(|host, tx| Ok::<_, ()>(slot.insert(host, tx, &stick, 1)));
                assert_eq!(went, Ok(1));
            }
        }
        if plan.replace_hand {
            let player = event.player.expect("a player's pour");
            let stone = ItemStack::new("minecraft:stone", 1).try_into_api().unwrap();
            assert_eq!(
                host.living(player.entity()).unwrap().set_hand_item(
                    host,
                    mod_api::client::first_person::Hand::Main,
                    Some(stone),
                ),
                Ok(true)
            );
        }
        plan.answer
    }
}

/// A dedicated server with a chunk at the origin and the Jar installed, running `test` with the
/// overworld.
fn with_jar(jar: &Jar, test: impl FnOnce(&mut super::Game<'_>, LevelRef)) {
    let mods: Mods = vec![ModEntry::new("jar", Box::new(jar.clone()))].into();
    crate::game::api_values::install_parts(&mods).unwrap();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let level = game.level("minecraft:overworld").unwrap();
            test(game, level);
        });
    });
}

fn dispense(game: &mut super::Game<'_>) -> Option<ItemStack> {
    game.dispense_item(
        (DISPENSER[0], DISPENSER[1], DISPENSER[2]),
        Direction::East,
        ItemStack::new("minecraft:stick", 1),
    )
    .unwrap()
}

fn honey_at_target(game: &super::Game<'_>) -> bool {
    let honey = game
        .pack
        .assets
        .registry
        .default_state("minecraft:honey_block");
    Some(game.world.block(TARGET[0], TARGET[1], TARGET[2])) == honey
}

/// A dispenser asks the Part of an item that is no bucket: `Some(true)` leaves an empty bucket in
/// the slot, `Some(false)` and `None` (no contents vanilla knows) dispense the item itself.
#[test]
#[ignore = "daily"]
fn a_part_empties_its_item_from_a_dispenser() {
    let jar = Jar::default();
    with_jar(&jar, |game, _| {
        jar.0.lock().unwrap().answer = Some(true);
        let left = dispense(game).map(|stack| stack.item.to_string());
        assert_eq!(left.as_deref(), Some("minecraft:bucket"));
        assert!(game.dropped.is_empty());

        for answer in [Some(false), None] {
            jar.0.lock().unwrap().answer = answer;
            let dropped = game.dropped.len();
            assert_eq!(dispense(game), None);
            assert_eq!(game.dropped.len(), dropped + 1);
        }
        assert_eq!(jar.0.lock().unwrap().asks, [(false, false); 3]);
    });
}

/// The Part places its contents through the host: the block stands after the dispense, and the
/// slot gets the empty bucket.
#[test]
#[ignore = "daily"]
fn a_dispensed_part_places_its_contents_through_the_host() {
    let jar = Jar::default();
    with_jar(&jar, |game, level| {
        *jar.0.lock().unwrap() = Plan {
            answer: Some(true),
            place: Some(level),
            ..Plan::default()
        };
        let left = dispense(game).map(|stack| stack.item.to_string());
        assert_eq!(left.as_deref(), Some("minecraft:bucket"));
        assert!(honey_at_target(game));
    });
}

/// A Part that writes the dispenser's slots, which the decision read, is refused: its writes are
/// undone (slot 1 empty) and the dispenser keeps the stick; direct block writes remain.
#[test]
#[ignore = "daily"]
fn a_part_that_changes_the_dispensers_slots_is_refused_and_undone() {
    let jar = Jar::default();
    with_jar(&jar, |game, level| {
        let dispenser = game
            .pack
            .assets
            .registry
            .default_state("minecraft:dispenser");
        let placed =
            game.level_set_block(level, DISPENSER, dispenser.unwrap(), BlockUpdateFlags::NONE);
        assert_eq!(placed, Ok(true));
        let at = (DISPENSER[0], DISPENSER[1], DISPENSER[2]);
        let slots = |game: &super::Game<'_>| game.world.block_entities[&at].slots().to_vec();
        game.world.block_entities.get_mut(&at).unwrap().slots_mut()[0] =
            Some(ItemStack::new("minecraft:stick", 1));
        let before = slots(game);
        *jar.0.lock().unwrap() = Plan {
            answer: Some(true),
            place: Some(level),
            meddle: true,
            ..Plan::default()
        };
        let left = dispense(game);
        assert_eq!(left, Some(ItemStack::new("minecraft:stick", 1)));
        // A direct block write is not journaled (see the transactions page); only journaled writes are undone.
        assert_eq!(slots(game), before);
    });
}

/// A stale-hand pour is refused without consuming the listener's final replacement.
#[test]
#[ignore = "daily"]
fn a_player_pour_with_a_changed_hand_is_refused() {
    let jar = Jar::default();
    with_jar(&jar, |game, level| {
        *jar.0.lock().unwrap() = Plan {
            answer: Some(true),
            place: Some(level),
            replace_hand: true,
            ..Plan::default()
        };
        crate::game::native_item_places::tests::open_to_lan(game);
        game.lan
            .as_mut()
            .unwrap()
            .hub
            .guests
            .push(crate::game::native_item_places::tests::guest(1));
        game.as_guest(1, |game| {
            game.creative = false;
            let stick = ItemStack::new("minecraft:stick", 1);
            let slot = game.held_slot();
            *game.held_mut() = Some(stick.clone());
            let hit = crate::game::Hit {
                pos: [6, 63, 5],
                face: [0, 1, 0],
                point: [6.5, 64.0, 5.5],
                inside: false,
            };
            let used = game.player_empty_contents(&stick, TARGET, &hit);
            assert!(matches!(
                used,
                Some(horizon_client::first_person::UseResult::Fail)
            ));
            assert_eq!(game.inventory[slot], Some(stick));
            // The Part's equipment write is journaled and rolls back with the refused pour.
        });
    });
}

/// A player's pour asks the Part with the clicked block; the Part places the contents and the hand
/// becomes an empty bucket.
#[test]
#[ignore = "daily"]
fn a_player_pours_through_the_part_and_keeps_an_empty_bucket() {
    let jar = Jar::default();
    with_jar(&jar, |game, level| {
        *jar.0.lock().unwrap() = Plan {
            answer: Some(true),
            place: Some(level),
            ..Plan::default()
        };
        game.creative = false;
        let stick = ItemStack::new("minecraft:stick", 1);
        let slot = game.held_slot();
        *game.held_mut() = Some(stick.clone());
        let hit = crate::game::Hit {
            pos: [6, 63, 5],
            face: [0, 1, 0],
            point: [6.5, 64.0, 5.5],
            inside: false,
        };
        let used = game.player_empty_contents(&stick, TARGET, &hit);
        assert!(matches!(
            used,
            Some(horizon_client::first_person::UseResult::Success { swing: true })
        ));
        assert!(honey_at_target(game));
        let held = game.inventory[slot]
            .as_ref()
            .map(|stack| stack.item.to_string());
        assert_eq!(held.as_deref(), Some("minecraft:bucket"));
        let asks = &jar.0.lock().unwrap().asks;
        assert_eq!(asks.len(), 1);
        assert!(asks[0].1, "a player's pour names the clicked block");
    });
}
