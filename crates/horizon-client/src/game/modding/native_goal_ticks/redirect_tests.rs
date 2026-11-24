//! A target listener can choose a different real guest within a mob tick.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_client::mobs::{
    MobEvent, TargetChange,
    player_selection::{PlayerSelection, TickPlayer},
};
use horizon_core::geometry::Vec3;
use std::sync::{Arc, Mutex};

struct Redirect(Arc<Mutex<Option<LivingRef>>>);

#[mod_api::__subscriptions]
impl mod_api::Hooks for Redirect {
    fn living_change_target(
        &self,
        _host: &dyn mod_api::Host,
        event: &mut mod_api::LivingChangeTargetEvent<'_>,
    ) -> mod_api::Flow {
        *event.new_target = *self.0.lock().unwrap();
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn redirect_to_another_guest_switches_view_and_tags_subsequent_events() {
    let redirect = Arc::new(Mutex::new(None));
    let mods = vec![mod_api::ModEntry::new(
        "redirect",
        Box::new(Redirect(redirect.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            use crate::game::native_item_places::tests::{guest, open_to_lan};

            open_to_lan(game);
            for conn in [12, 13] {
                let mut placed = guest(conn);
                placed.loaded = true;
                game.lan.as_mut().unwrap().hub.guests.push(placed);
            }
            let level = game.level(game.dim.id()).unwrap();
            let fake = game.fake_player(level, 999, "fake").unwrap();
            let fake_target = LivingRef::new(fake.entity());
            let epoch = game.native_lifecycle.epoch().unwrap();
            let first_id = 31;
            let second_id = 32;
            let second = LivingRef::new(game.entity(second_id).unwrap());
            let views = game.lan_guest_views();
            let players = views
                .into_iter()
                .filter_map(|(conn, mut view)| {
                    if conn != 12 && conn != 13 {
                        return None;
                    }
                    let (_, entity_id) = game.guest_identity(conn)?;
                    view.position = Vec3::new(f64::from(conn), 64.0, 0.0);
                    Some(TickPlayer {
                        entity_id,
                        connection: Some(conn),
                        view,
                    })
                })
                .collect();
            let selection = PlayerSelection::new(players, Some(first_id)).unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:zombie")
                .unwrap()
                .clone();
            let mut actor = Mob::new(9001, kind, Vec3::ZERO, 0.0);
            let mut other = Mob::new(9002, actor.kind.clone(), Vec3::ZERO, 0.0);
            other.ai.target = Some(Who::Player);
            game.lan.as_mut().unwrap().mob_players.insert(other.id, 12);
            let mut world = game.mob_world_base(&[]);
            world.goal_host = Some(game);
            world.player_selection = Some(&selection);

            let before = world.route_player_event(MobEvent::AttackPlayer(1.0, [0.0; 3]));
            assert!(matches!(before, MobEvent::AttackPlayer(..)));
            *redirect.lock().unwrap() = Some(second);
            assert_eq!(
                game.native_mob_change_target_in_tick(&mut actor, &world, false, Some(Who::Player)),
                TargetChange::Set(Some(Who::Player))
            );
            assert_eq!(world.selected_player_identity().unwrap().1, second_id);
            assert_eq!(world.selected_player().unwrap().position.x, 13.0);
            assert_eq!(
                mobs::part_goals::GoalHost::mob_target(game, &other, &actor, &world),
                Some(LivingRef::new(game.entity(first_id).unwrap()))
            );
            let after = world.route_player_event(MobEvent::AttackPlayer(1.0, [0.0; 3]));
            assert!(
                matches!(after, MobEvent::ForPlayer { conn: Some(13), event }
                if matches!(*event, MobEvent::AttackPlayer(..)))
            );
            // An alert (`HurtByTargetGoal.alertOthers`) hands over the redirected player too.
            let alert = world.route_player_event(MobEvent::SetTarget {
                mob: other.id,
                target: Who::Player,
                player: None,
            });
            assert!(
                matches!(alert, MobEvent::ForPlayer { conn: Some(13), event }
                if matches!(*event, MobEvent::SetTarget { target: Who::Player, .. }))
            );

            for invalid in [fake_target, LivingRef::new(EntityRef::new(epoch, 99999))] {
                *redirect.lock().unwrap() = Some(invalid);
                assert_eq!(
                    game.native_mob_change_target_in_tick(
                        &mut actor,
                        &world,
                        false,
                        Some(Who::Player)
                    ),
                    TargetChange::Keep
                );
                assert_eq!(world.selected_player_identity().unwrap().1, second_id);
            }
        });
    });
}
