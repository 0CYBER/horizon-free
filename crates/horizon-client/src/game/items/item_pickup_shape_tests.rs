#![cfg(test)]
use super::*;
use horizon_client::join_seq::{
    Joined as _,
    residency::{Journal, Kind, Owner, table::EntityTable},
};
use horizon_client::motion::PlayerMotion;
use horizon_core::geometry::Vec3;
use mod_api::{Flow, Hooks, ItemPickedUpEvent, ModEntry, ServerHost};
use mod_platform::testing::with_mods;
use std::sync::{Arc, Mutex};

/// What `item_picked_up_post` heard: the original count and what remained.
type Posts = Arc<Mutex<Vec<(u32, Option<u32>)>>>;
#[derive(Clone)]
struct Pickups(
    Arc<Mutex<Vec<u32>>>,
    Arc<Mutex<Vec<&'static str>>>,
    Posts,
    /// The item entity `item_picked_up` refuses.
    Arc<Mutex<u32>>,
);
#[mod_api::__subscriptions]
impl Hooks for Pickups {
    fn player_stat_award(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::PlayerStatAwardEvent<'_>,
    ) -> Flow {
        self.1.lock().unwrap().push("stat");
        Flow::Continue
    }
    fn player_advancement_progress(
        &self,
        _: &mut dyn ServerHost,
        _: &mut mod_api::PlayerAdvancementProgressEvent<'_>,
    ) {
        self.1.lock().unwrap().push("criterion");
    }
    fn item_picked_up(&self, _: &mut dyn ServerHost, event: &mut ItemPickedUpEvent<'_>) {
        let id = event.item.entity().id();
        self.0.lock().unwrap().push(id);
        if id == *self.3.lock().unwrap() {
            *event.can_pickup = mod_api::TriState::False;
        }
    }
    fn item_picked_up_post(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::ItemPickedUpPostEvent<'_>,
    ) {
        self.2.lock().unwrap().push((
            event.original.count(),
            event.remaining.as_ref().map(|rest| rest.count()),
        ));
    }
}

/// Runs the production lifted pass, not a substitute pickup loop. Binding is
/// strictly test-only; no Game residency producer is activated here.
#[test]
#[ignore = "daily"]
fn indexed_pickup_keeps_bound_survivors_and_processes_shifted_tail() {
    let heard = Arc::new(Mutex::new(Vec::new()));
    let notifications = Arc::new(Mutex::new(Vec::new()));
    let posts = Posts::default();
    let refused = Arc::new(Mutex::new(0));
    let mods: mod_api::Mods = vec![ModEntry::new(
        "pickup_shape",
        Box::new(Pickups(
            heard.clone(),
            notifications.clone(),
            posts.clone(),
            refused.clone(),
        )),
    )]
    .into();
    with_mods(mods, || {
        let mut loaded = 0;
        for dimension in [crate::game::Dim::Overworld, crate::game::Dim::Nether] {
            let scratch = std::path::PathBuf::from(
                std::env::var_os("HORIZON_HOME")
                    .expect("pickup fixture requires an isolated HORIZON_HOME"),
            )
            .join(format!("pickup-shape-{}-{dimension:?}", std::process::id()));
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.world_root = scratch.clone();
                game.enter_level(dimension).unwrap();
                loaded += 1;
                game.player =
                    Some(PlayerMotion::new(game.motion, Vec3::new(1.5, 64.0, 1.5)).unwrap());
                game.inventory
                    .fill(Some(ItemStack::new("minecraft:stone", 64)));
                game.offhand = Some(ItemStack::new("minecraft:stone", 64));
                game.inventory[0] = Some(ItemStack::new("minecraft:apple", 62));
                let uuid = horizon_client::lan::playerdata::uuid_ints(7);
                game.level.player_uuid = Some(uuid);
                game.lan_acting_uuid = Some(7);
                game.lan_acting_name = Some("pickup_test".into());
                game.lan_acting_entity = Some(42);
                let data = scratch.join("data/pickup_test/advancement");
                std::fs::create_dir_all(&data).unwrap();
                std::fs::write(data.join("pickup.json"), br#"{"criteria":{"take":{"trigger":"minecraft:thrown_item_picked_up_by_player"}}}"#).unwrap();
                game.adv.tree = Arc::new(horizon_client::advancements::Tree::load(
                    &scratch.join("data"),
                ));
                game.adv.loaded = true;
                let level = game.level(game.dim.id()).unwrap();
                let mut ids = Vec::new();
                for count in [1, 3, 2] {
                    let item = game
                        .level_spawn_item(
                            level,
                            1.5,
                            64.0,
                            1.5,
                            ItemStack::new("minecraft:apple", count),
                            0,
                        )
                        .unwrap();
                    game.dropped
                        .iter_mut()
                        .find(|row| row.id == item.id())
                        .unwrap()
                        .thrower = Some(uuid);
                    ids.push(item.id());
                }
                let joins: Vec<_> = game.dropped.iter().map(|item| item.join_seq()).collect();
                let mut registrations =
                    crate::game::entity::taken_pass_tests::SlotIdentities::default();
                for item in &mut game.dropped {
                    let id = item.id;
                    registrations.observe_one(item, level.epoch(), id);
                }
                let mut table = EntityTable::new(Journal::default());
                game.dropped.bind_residency(
                    &table.producer(),
                    Owner {
                        dimension: game.dim.key(),
                        kind: Kind::Item,
                    },
                    |item| (item.id, item.join),
                );
                table.drain();
                heard.lock().unwrap().clear();
                *refused.lock().unwrap() = ids[2];
                notifications.lock().unwrap().clear();
                posts.lock().unwrap().clear();
                game.world.random = horizon_core::random::LevelRandom::for_level(
                    horizon_core::random::LegacyRandom::new(12345),
                );
                let mut round = game.native_tick_round();
                game.tick_dropped_items(&mut round);
                assert_eq!(
                    *heard.lock().unwrap(),
                    ids,
                    "the shifted row must not be skipped"
                );
                assert_eq!(
                    *notifications.lock().unwrap(),
                    ["stat", "criterion", "stat"]
                );
                assert_eq!(
                    *posts.lock().unwrap(),
                    [(1, None)],
                    "only the whole pickup is heard, not the partial or the refused one"
                );
                assert_eq!(game.inventory[0].as_ref().unwrap().count, 64);
                assert_eq!(
                    game.stats.get("picked_up", "minecraft:apple"),
                    2
                );
                assert_eq!(
                    game.world.random.next_int(),
                    1553932502,
                    "client pickup pitches do not consume the server level random"
                );
                assert_eq!(
                    game.dropped
                        .iter()
                        .map(|item| (item.id, item.stack.count, item.pickup_delay))
                        .collect::<Vec<_>>(),
                    [(ids[1], 2, 0), (ids[2], 2, 10)]
                );
                assert_eq!(
                    game.dropped
                        .iter()
                        .map(|item| item.join_seq())
                        .collect::<Vec<_>>(),
                    joins[1..]
                );
                for item in &mut game.dropped {
                    let id = item.id;
                    registrations.observe_one(item, level.epoch(), id);
                }
                for (index, item) in game.dropped.iter().enumerate() {
                    assert_eq!(game.dropped.locate(item.join_seq()), Some(index));
                }
                table
                    .end_tick()
                    .expect("actual pickup pass closes its original binding");
                for &id in &ids {
                    use horizon_client::join_seq::residency::table::{EntityLookup, SourceEntity};
                    let result = table.entity_lookup(&game.dim.key(), id, |location| {
                        let index = game.dropped.locate(location.join)?;
                        Some(SourceEntity {
                            id: game.dropped[index].id,
                            value: index,
                        })
                    });
                    if id == ids[0] {
                        assert_eq!(result, EntityLookup::Missing);
                    } else {
                        assert!(matches!(result, EntityLookup::Found { .. }));
                    }
                }
                assert_eq!(
                    game.pickups.made().get("item").copied().unwrap_or(0),
                    2,
                    "full and partial pickup animations"
                );

                // A pickup the full inventory refuses is not heard.
                for item in &mut game.dropped {
                    if item.id == ids[1] {
                        item.stack.count = 1;
                    }
                }
                heard.lock().unwrap().clear();
                posts.lock().unwrap().clear();
                let mut round = game.native_tick_round();
                game.tick_dropped_items(&mut round);
                assert_eq!(
                    heard.lock().unwrap()[..1],
                    [ids[1]],
                    "the pre hook let it through"
                );
                assert!(posts.lock().unwrap().is_empty());
                assert_eq!(
                    game.dropped
                        .iter()
                        .find(|item| item.id == ids[1])
                        .unwrap()
                        .stack
                        .count,
                    1
                );
                table.end_tick().expect("full inventory pass closes");

                // The same production pass must restore its binding on all early returns.
                for mode in 0..3 {
                    game.player = match mode {
                        0 => None,
                        _ => Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap()),
                    };
                    if mode == 1 {
                        game.player
                            .as_mut()
                            .unwrap()
                            .set_health(0.0, false)
                            .unwrap();
                    }
                    if mode == 2 {
                        game.mode = horizon_client::command::args::GameMode::Spectator;
                    }
                    let before: Vec<_> = game
                        .dropped
                        .iter()
                        .map(|item| (item.id, item.join_seq(), item.stack.count))
                        .collect();
                    let mut round = game.native_tick_round();
                    game.tick_dropped_items(&mut round);
                    assert_eq!(
                        game.dropped
                            .iter()
                            .map(|item| (item.id, item.join_seq(), item.stack.count))
                            .collect::<Vec<_>>(),
                        before
                    );
                    table.end_tick().expect("early return closes pass");
                    game.mode = horizon_client::command::args::GameMode::Survival;
                }
            });
            let _ = std::fs::remove_dir_all(scratch);
        }
        assert_eq!(loaded, 2, "both resource fixtures executed");
    });
}
