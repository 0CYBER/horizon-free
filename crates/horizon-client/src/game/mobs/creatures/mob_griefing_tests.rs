// Test module extracted without changing its logical path.
use super::super::native_entity_refs::tests::with_game;
use super::super::native_item_places::tests::{guest, open_to_lan};
use std::sync::{Arc, Mutex};

/// Denies griefing to the entities in `self.0`, and logs who was asked.
struct Deny(Vec<u32>, Arc<Mutex<Vec<(u32, bool)>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Deny {
    fn entity_mob_griefing(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::EntityMobGriefingEvent,
    ) {
        self.1
            .lock()
            .unwrap()
            .push((event.entity.id(), event.can_grief));
        if self.0.contains(&event.entity.id()) {
            event.can_grief = false;
        }
    }
}

/// `EntityMobGriefingEvent`: the mod's answer for one entity reaches the
/// game's checks and the mob AI's; another entity keeps the rule, and a
/// check with no entity reads the rule without asking.
#[test]
#[ignore = "daily"]
fn a_mod_denies_one_entity_griefing() {
    let heard = Arc::default();
    let (denied, other) = (900, 901);
    // Guest 5's entity id (`guest` gives `conn + 19`): a player the mod denies.
    let denied_player = 24;
    let mods = vec![mod_api::ModEntry::new(
        "deny",
        Box::new(Deny(vec![denied, denied_player], Arc::clone(&heard))),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            open_to_lan(game);
            assert!(
                game.ui
                    .rules
                    .bool_at(horizon_client::gamerules::vanilla::MOB_GRIEFING)
            );
            assert!(!game.can_entity_grief(Some(denied)));
            assert!(game.can_entity_grief(Some(other)));
            assert!(game.can_entity_grief(None));
            let world = game.mob_world_base(&[]);
            assert!(!world.can_grief(denied));
            assert!(world.can_grief(other));
            let asked = heard.lock().unwrap().clone();
            assert_eq!(
                asked,
                [(denied, true), (other, true), (denied, true), (other, true)]
            );
            // A large fireball asks for its owner: the ghast the mod denies
            // lights no fire and another ghast's does; a fireball sent back
            // by the guest the mod denies lights none and another guest's
            // does. A gone owner, or none, reads the rule without asking.
            use super::Who;
            let hub = &mut game.lan.as_mut().unwrap().hub;
            hub.guests.push(guest(5));
            hub.guests.push(guest(6));
            heard.lock().unwrap().clear();
            assert!(!game.large_fireball_fire(Some(Who::Mob(denied)), None));
            assert!(game.large_fireball_fire(Some(Who::Mob(other)), None));
            assert!(!game.large_fireball_fire(Some(Who::Player), Some(5)));
            assert!(game.large_fireball_fire(Some(Who::Player), Some(6)));
            assert!(game.large_fireball_fire(Some(Who::Player), Some(7)));
            assert!(game.large_fireball_fire(None, None));
            assert_eq!(
                *heard.lock().unwrap(),
                [
                    (denied, true),
                    (other, true),
                    (denied_player, true),
                    (25, true)
                ]
            );
            // `shouldAffectBlocklikeEntities` asks for the exploding entity: a
            // blast that keeps blocks spares decorations when the mod denies
            // it, and reaches them for another exploding entity.
            use super::super::explosion::BlockInteraction::{Destroy, Keep};
            game.combat.explosion_source = Some(denied);
            assert!(game.blast_spares_decorations(Keep));
            assert!(!game.blast_spares_decorations(Destroy));
            game.combat.explosion_source = Some(other);
            assert!(!game.blast_spares_decorations(Keep));
            game.combat.explosion_source = None;
        });
    });
}
