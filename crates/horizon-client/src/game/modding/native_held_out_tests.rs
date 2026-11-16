use std::sync::{Arc, Mutex};

use horizon_client::{
    chunk::Chunk,
    items::ItemStack,
    mobs::{Mob, leash::Holder, transport_items},
};
use horizon_core::geometry::Vec3;
use mod_api::{self, EntityMountEvent, EntityRef, Flow, Hooks, ItemSpawnEvent, ServerHost};

use super::{
    boats::Boat,
    minecarts::{CartKind, Minecart},
    native_entity_refs::tests::with_game,
};

#[derive(Default)]
struct Observation {
    handles: Vec<EntityRef>,
    reads: Vec<Vec<Option<[f64; 3]>>>,
    results: Vec<Vec<bool>>,
}

struct Observer(Arc<Mutex<Observation>>);
impl Observer {
    fn observe(&self, host: &mut dyn ServerHost) {
        let mut state = self.0.lock().unwrap();
        let reads = state.handles.iter().map(|e| e.velocity(host)).collect();
        let results = state
            .handles
            .iter()
            .map(|e| e.set_velocity(host, [2.0, 0.0, 0.0]))
            .collect();
        state.reads.push(reads);
        state.results.push(results);
    }
}
#[mod_api::__subscriptions]
impl Hooks for Observer {
    fn item_spawn(&self, host: &mut dyn ServerHost, _: &mut ItemSpawnEvent<'_>) {
        self.observe(host);
    }
    fn entity_mount(&self, host: &mut dyn ServerHost, _: &mut EntityMountEvent<'_>) -> Flow {
        self.observe(host);
        Flow::Continue
    }
}

fn with_observer(test: impl FnOnce(&mut super::Game<'_>, &Arc<Mutex<Observation>>)) {
    let state = Arc::new(Mutex::new(Observation::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "observer",
        Box::new(Observer(state.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            test(game, &state);
        })
    });
}

#[test]
#[ignore = "daily"]
fn boat_tick_refuses_a_write_from_a_leash_drop_hook() {
    with_observer(|game, state| {
        let mut boat = Boat::new(100, "oak".into(), Vec3::new(0.5, 64.0, 0.5), 0.0);
        boat.leash = Some(Holder::Knot([0, 64, 0]));
        game.boats.push(boat);
        let handle = game.entity(100).unwrap();
        state.lock().unwrap().handles.push(handle);
        game.tick_boats();
        assert_eq!(state.lock().unwrap().reads.as_slice(), &[vec![None]]);
        assert_eq!(state.lock().unwrap().results.as_slice(), &[vec![false]]);
        assert!(handle.set_velocity(game, [2.0, 0.0, 0.0]));
        assert_eq!(handle.velocity(game), Some([2.0, 0.0, 0.0]));
    });
}

#[test]
#[ignore = "daily"]
fn statue_conversion_refuses_a_write_from_an_item_spawn_hook() {
    with_observer(|game, state| {
        let kind = game
            .res
            .mob_types()
            .get("minecraft:copper_golem")
            .unwrap()
            .clone();
        let mut golem = Mob::new(101, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        transport_items::set_held(&mut golem, Some(ItemStack::new("minecraft:diamond", 1)));
        game.mobs.push(golem);
        let handle = game.entity(101).unwrap();
        state.lock().unwrap().handles.push(handle);
        assert!(handle.set_velocity(game, [2.0, 0.0, 0.0]));
        assert_eq!(handle.velocity(game), Some([2.0, 0.0, 0.0]));
        game.golem_become_statue(101);
        assert_eq!(state.lock().unwrap().reads.as_slice(), &[vec![None]]);
        assert_eq!(state.lock().unwrap().results.as_slice(), &[vec![false]]);
    });
}

#[test]
#[ignore = "daily"]
fn cart_tick_refuses_a_write_from_a_mount_hook() {
    with_observer(|game, state| {
        game.options.dedicated = Some(Vec::new());
        let mut cart = Minecart::new(102, CartKind::Rideable, Vec3::new(0.5, 64.0, 0.5));
        cart.velocity = Vec3::new(0.5, 0.0, 0.0);
        game.carts.push(cart);
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        game.mobs
            .push(Mob::new(103, kind, Vec3::new(0.7, 64.0, 0.5), 0.0));
        let handle = game.entity(102).unwrap();
        state.lock().unwrap().handles.push(handle);
        game.tick_carts();
        assert_eq!(state.lock().unwrap().reads.as_slice(), &[vec![None]]);
        assert_eq!(state.lock().unwrap().results.as_slice(), &[vec![false]]);
        assert!(handle.set_velocity(game, [2.0, 0.0, 0.0]));
        assert_eq!(handle.velocity(game), Some([2.0, 0.0, 0.0]));
    });
}

#[test]
#[ignore = "daily"]
fn leash_interaction_keeps_the_stored_boat_writable() {
    with_observer(|game, state| {
        let mut boat = Boat::new(106, "oak".into(), Vec3::new(0.5, 64.0, 0.5), 0.0);
        boat.leash = Some(Holder::Knot([0, 64, 0]));
        game.boats.push(boat);
        let handle = game.entity(106).unwrap();
        state.lock().unwrap().handles.push(handle);
        game.drop_boat_leashes(&[0]);
        assert_eq!(state.lock().unwrap().results.as_slice(), &[vec![true]]);
        assert_eq!(handle.velocity(game), Some([2.0, 0.0, 0.0]));
    });
}

#[test]
#[ignore = "daily"]
fn nested_held_out_entities_both_refuse() {
    with_observer(|game, state| {
        game.carts.push(Minecart::new(
            104,
            CartKind::Rideable,
            Vec3::new(0.5, 64.0, 0.5),
        ));
        let mut boat = Boat::new(105, "oak".into(), Vec3::new(0.5, 64.0, 0.5), 0.0);
        boat.leash = Some(Holder::Knot([0, 64, 0]));
        game.boats.push(boat);
        let handles = [game.entity(104).unwrap(), game.entity(105).unwrap()];
        state.lock().unwrap().handles.extend(handles);
        game.with_entity_held_out(104, |game| {
            game.tick_boats();
            assert!(!handles[0].set_velocity(game, [2.0, 0.0, 0.0]));
            assert!(handles[1].set_velocity(game, [2.0, 0.0, 0.0]));
        });
        assert_eq!(state.lock().unwrap().reads.as_slice(), &[vec![None, None]]);
        assert_eq!(
            state.lock().unwrap().results.as_slice(),
            &[vec![false, false]]
        );
        for handle in handles {
            assert!(handle.set_velocity(game, [2.0, 0.0, 0.0]));
        }
    });
}
