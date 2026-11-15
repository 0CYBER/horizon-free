//! `ItemDrop`'s item-entity state (arismoko/horizon#3423): `thrower`, `target`, `no_gravity` and `age` set by a
//! drop hook reach the item entity that joins the level and behave as vanilla's do; a drop left
//! as it was joins as before.
use std::sync::{
    Arc,
    atomic::{AtomicU8, Ordering},
};

use horizon_client::{entities::ItemEntity, items::ItemStack, lan::playerdata::uuid_ints};
use horizon_core::geometry::Vec3;
use mod_api::{self, Flow, Hooks, ItemDrop, LivingDropsEvent, ServerHost};

use super::spill::{BlockItemDrop, DropState, prepare_item_random};

const KILLER: u128 = 0x1234;
const THROWER: u128 = 0x5678;

fn drop_with(state: DropState, random: bool) -> BlockItemDrop {
    BlockItemDrop {
        random: random.then(|| prepare_item_random(None)),
        stack: ItemStack::new("minecraft:rotten_flesh", 1),
        position: [0.5, 64.0, 0.5],
        velocity: [0.1, 0.2, 0.0],
        pickup_delay: 10,
        state,
    }
}

fn set() -> DropState {
    DropState {
        thrower: Some(THROWER),
        target: Some(KILLER),
        no_gravity: true,
        age: ItemDrop::UNLIMITED_LIFETIME,
    }
}

#[test]
fn an_unedited_drop_keeps_its_random_as_it_was() {
    let drawn = drop_with(DropState::default(), true);
    assert_eq!(drawn.random_with_state(), drawn.random);
    assert_eq!(
        drop_with(DropState::default(), false).random_with_state(),
        None
    );
}

#[test]
fn a_drop_with_state_carries_it_into_its_random() {
    let drawn = drop_with(set(), true);
    let random = drawn.random_with_state().unwrap();
    assert_eq!(random.state, set());
    assert_eq!(
        random.uuid,
        drawn.random.unwrap().uuid,
        "the drawn random is kept"
    );
    // An edited drop has no random yet: one is drawn with the drop's own motion.
    let random = drop_with(set(), false).random_with_state().unwrap();
    assert_eq!((random.state, random.velocity), (set(), [0.1, 0.2, 0.0]));
}

#[test]
fn the_state_reaches_the_item_entity() {
    let fresh = || {
        ItemEntity::new(
            1,
            ItemStack::new("minecraft:stone", 1),
            Vec3::ZERO,
            Vec3::ZERO,
            0.0,
        )
    };
    let item = set().apply(fresh());
    assert_eq!(item.thrower, Some(uuid_ints(THROWER)));
    assert_eq!(item.target, Some(uuid_ints(KILLER)));
    assert!(item.no_gravity);
    assert_eq!(item.age, -32768);
    // The default state is a fresh item entity's.
    let plain = DropState::default().apply(fresh());
    let new = fresh();
    assert_eq!(
        (plain.thrower, plain.target, plain.no_gravity, plain.age),
        (new.thrower, new.target, new.no_gravity, new.age)
    );
}

/// What the `living_drops` listener does to each drop: nothing.
const LEAVE: u8 = 0;
/// The state of [`set`].
const MARK: u8 = 1;
/// Only guest 12 (UUID 12) may pick it up, at once.
const TARGET_GUEST: u8 = 2;
/// No gravity, ordinary lifetime: the control for [`FLOAT_FOREVER`].
const FLOAT: u8 = 3;
/// No gravity and an unlimited lifetime.
const FLOAT_FOREVER: u8 = 4;
/// No gravity and the largest `age` a mod can give.
const FLOAT_OLDEST: u8 = 5;

struct Marker(Arc<AtomicU8>);
#[mod_api::__subscriptions]
impl Hooks for Marker {
    fn living_drops(&self, _: &mut dyn ServerHost, event: &mut LivingDropsEvent<'_>) -> Flow {
        let mode = self.0.load(Ordering::SeqCst);
        for drop in event.drops.iter_mut() {
            let mut edited = drop.clone();
            edited = match mode {
                MARK => edited
                    .with_thrower(Some(THROWER))
                    .with_target(Some(KILLER))
                    .with_no_gravity(true)
                    .with_age(ItemDrop::UNLIMITED_LIFETIME),
                TARGET_GUEST => {
                    edited.pickup_delay = 0;
                    edited.with_target(Some(12))
                }
                FLOAT => edited.with_no_gravity(true),
                FLOAT_FOREVER => edited
                    .with_no_gravity(true)
                    .with_age(ItemDrop::UNLIMITED_LIFETIME),
                FLOAT_OLDEST => edited.with_no_gravity(true).with_age(i32::MAX),
                _ => edited,
            };
            *drop = edited;
        }
        Flow::Continue
    }
}

fn with_marker(test: impl FnOnce(&Arc<AtomicU8>)) {
    let mode = Arc::new(AtomicU8::new(LEAVE));
    let marker = Box::new(Marker(mode.clone()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("drop-marker", marker)].into();
    mod_platform::testing::with_mods(mods, || test(&mode));
}

/// A copper golem holding a diamond dies at `at`: the item entities its death added.
fn golem_dies(game: &mut super::Game<'_>, at: [f64; 3]) -> Vec<ItemEntity> {
    let kind = game
        .res
        .mob_types()
        .get("minecraft:copper_golem")
        .unwrap()
        .clone();
    game.next_entity += 1;
    let position = Vec3::new(at[0], at[1], at[2]);
    let mut golem = horizon_client::mobs::Mob::new(game.next_entity, kind, position, 0.0);
    horizon_client::mobs::transport_items::set_held(
        &mut golem,
        Some(ItemStack::new("minecraft:diamond", 1)),
    );
    golem.health = 0.0;
    game.mobs.push(golem.clone());
    let before = game.dropped.len();
    game.mob_died(&golem);
    game.dropped[before..].to_vec()
}

fn ids(items: &[ItemEntity]) -> Vec<u32> {
    items.iter().map(|d| d.id).collect()
}

fn on_ground(game: &super::Game<'_>, ids: &[u32]) -> usize {
    game.dropped.iter().filter(|d| ids.contains(&d.id)).count()
}

/// A copper golem's death drops (its preserved diamond included) join with the state the
/// listener set, through the API drop and back; with the listener leaving them alone they join
/// as fresh item entities.
#[test]
#[ignore = "daily"]
fn living_drops_state_reaches_the_dropped_item_entities() {
    assert_eq!(ItemDrop::UNLIMITED_LIFETIME, -32768);
    with_marker(|mode| {
        super::native_entity_refs::tests::with_game(|game| {
            game.load_server_loot();
            mode.store(MARK, Ordering::SeqCst);
            let marked = golem_dies(game, [0.5, 64.0, 0.5]);
            assert!(!marked.is_empty());
            for item in &marked {
                assert_eq!(item.thrower, Some(uuid_ints(0x5678)));
                assert_eq!(item.target, Some(uuid_ints(0x1234)));
                assert!(item.no_gravity);
                assert_eq!(item.age, -32768);
            }
            mode.store(LEAVE, Ordering::SeqCst);
            let plain = golem_dies(game, [0.5, 64.0, 0.5]);
            assert!(!plain.is_empty());
            for item in &plain {
                assert_eq!((item.thrower, item.target), (None, None));
                assert!(!item.no_gravity);
                assert_eq!(item.age, 0);
            }
        });
    });
}

/// A drop the hook targets at guest 12 is not picked up by guest 13 standing on it, and is by
/// guest 12.
#[test]
#[ignore = "daily"]
fn a_targeted_drop_is_picked_up_only_by_its_target() {
    with_marker(|mode| {
        super::native_guest_turn_tests::with_guests(|game| {
            game.load_server_loot();
            mode.store(TARGET_GUEST, Ordering::SeqCst);
            // Both guests stand at (3, 64, 5).
            let drops = ids(&golem_dies(game, [3.0, 64.0, 5.0]));
            assert!(!drops.is_empty());
            let other = game.guest_index(13).unwrap();
            game.guest_pickups(other);
            assert_eq!(on_ground(game, &drops), drops.len(), "guest 13 takes none");
            let target = game.guest_index(12).unwrap();
            game.guest_pickups(target);
            assert_eq!(on_ground(game, &drops), 0, "guest 12 takes them all");
        });
    });
}

/// A world of one empty column (0, 0) for item ticks.
fn item_world(game: &mut super::Game<'_>) {
    game.load_server_loot();
    game.native_lifecycle
        .begin_open(mod_api::Authority::IntegratedServer);
    game.world = horizon_client::world::World::new(game.res.facts().air, 1);
    let column = Arc::new(horizon_client::chunk::Chunk::empty(0, 0));
    game.world.chunks.insert((0, 0), column);
}

const CENTRE: [f64; 3] = [8.5, 64.0, 8.5];

/// After 6000 ticks a floating drop with an ordinary lifetime has despawned, and one the hook
/// gave an unlimited lifetime (which never merges) is still there.
#[test]
#[ignore = "daily"]
fn an_unlimited_lifetime_drop_outlives_an_ordinary_one() {
    with_marker(|mode| {
        super::native_entity_refs::tests::with_game(|game| {
            item_world(game);
            mode.store(FLOAT, Ordering::SeqCst);
            let ordinary = ids(&golem_dies(game, CENTRE));
            mode.store(FLOAT_FOREVER, Ordering::SeqCst);
            let forever = ids(&golem_dies(game, CENTRE));
            assert!(!ordinary.is_empty() && !forever.is_empty());
            game.ticked_entities_through = game.next_entity;
            for _ in 0..5999 {
                game.tick_items();
            }
            // Ordinary drops of one kind may merge (`mergeWithNeighbours`), so count what is left.
            assert!(on_ground(game, &ordinary) > 0, "still there at 5999");
            game.tick_items();
            assert_eq!(on_ground(game, &ordinary), 0, "despawned at 6000");
            assert_eq!(
                on_ground(game, &forever),
                forever.len(),
                "unlimited: still there"
            );
            for item in game.dropped.iter().filter(|d| forever.contains(&d.id)) {
                assert_eq!(item.age, -32768);
            }
        });
    });
}

/// A drop given `age` `i32::MAX` ticks and wraps to `i32::MIN`, as Java's int does, with no
/// overflow panic.
#[test]
#[ignore = "daily"]
fn the_largest_age_wraps_on_its_first_tick() {
    with_marker(|mode| {
        super::native_entity_refs::tests::with_game(|game| {
            item_world(game);
            mode.store(FLOAT_OLDEST, Ordering::SeqCst);
            let oldest = ids(&golem_dies(game, CENTRE));
            assert!(!oldest.is_empty());
            game.ticked_entities_through = game.next_entity;
            game.tick_items();
            let ages: Vec<i32> = game
                .dropped
                .iter()
                .filter(|d| oldest.contains(&d.id))
                .map(|d| d.age)
                .collect();
            assert!(!ages.is_empty());
            assert!(ages.iter().all(|&age| age == i32::MIN), "{ages:?}");
        });
    });
}
