//! Installed native listeners intercept the AI setter through the real server host.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::geometry::Vec3;
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Default)]
enum Decision {
    #[default]
    Cancel,
    Replace(Option<LivingRef>),
}

type ObservedTarget = (u32, bool, Option<u32>);

#[derive(Default)]
struct Intercept {
    decision: Arc<Mutex<Decision>>,
    calls: Arc<Mutex<Vec<ObservedTarget>>>,
}

#[mod_api::horizon_mod]
impl mod_api::Hooks for Intercept {
    fn living_change_target(
        &self,
        host: &dyn mod_api::Host,
        event: &mut mod_api::LivingChangeTargetEvent<'_>,
    ) -> mod_api::Flow {
        // The actor is outside the game's mob collection during an AI tick. These reads must
        // use the tick's scoped backend, rather than accidentally consult Game.
        assert!(event.mob.living().health(host).is_some());
        if let Some(target) = event.original_target {
            assert!(target.health(host).is_some());
        }
        self.calls.lock().unwrap().push((
            event.mob.entity().id(),
            event.is_brain,
            event.original_target.map(|target| target.entity().id()),
        ));
        match *self.decision.lock().unwrap() {
            Decision::Cancel => return mod_api::Flow::Cancel,
            Decision::Replace(target) => *event.new_target = target,
        }
        mod_api::Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn installed_native_listener_intercepts_ai_setter_and_preserves_species_effects() {
    let decision = Arc::new(Mutex::new(Decision::Cancel));
    let calls = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "target-interception",
        Box::new(Intercept {
            decision: decision.clone(),
            calls: calls.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let kind = game
                .res
                .mob_types()
                .get("minecraft:iron_golem")
                .unwrap()
                .clone();
            let target = Mob::new(9010, kind, Vec3::new(2.0, 64.0, 0.0), 0.0);
            let seen = [target.seen(None, game.items())];
            let epoch = game.native_lifecycle.epoch().unwrap();
            let replacement = LivingRef::new(EntityRef::new(epoch, target.id));
            let world = game.mob_world_base(&seen);
            assert!(world.mob_hooks.is_some());
            assert!(world.goal_host.is_some());

            // Both paths use the installed dispatcher, with the brain flag
            // derived from the actor's species rather than supplied by a test.
            for (id, species, brain) in [
                (9001, "minecraft:enderman", false),
                (9002, "minecraft:breeze", true),
            ] {
                let kind = game.res.mob_types().get(species).unwrap().clone();
                let mut actor = Mob::new(id, kind, Vec3::ZERO, 0.0);
                actor.age = 321;
                actor.set_target(Some(Who::Mob(target.id)));
                let flag_before = actor.extra.flag;
                let counter_before = actor.extra.counter;
                *decision.lock().unwrap() = Decision::Cancel;
                assert_eq!(actor.set_ai_target(&world, None), mobs::TargetChange::Keep);
                assert_eq!(actor.ai.target, Some(Who::Mob(target.id)));
                assert_eq!(
                    (actor.extra.flag, actor.extra.counter),
                    (flag_before, counter_before)
                );

                *decision.lock().unwrap() =
                    Decision::Replace(Some(LivingRef::new(EntityRef::new(epoch, 99999))));
                assert_eq!(actor.set_ai_target(&world, None), mobs::TargetChange::Keep);
                assert_eq!(actor.ai.target, Some(Who::Mob(target.id)));
                assert_eq!(
                    (actor.extra.flag, actor.extra.counter),
                    (flag_before, counter_before)
                );

                *decision.lock().unwrap() = Decision::Replace(None);
                assert_eq!(
                    actor.set_ai_target(&world, Some(Who::Mob(target.id))),
                    mobs::TargetChange::Set(None)
                );
                assert_eq!(actor.ai.target, None);
                if !brain {
                    assert!(!actor.extra.flag);
                    assert_eq!(actor.extra.counter, 0);
                }

                *decision.lock().unwrap() = Decision::Replace(Some(replacement));
                assert_eq!(
                    actor.set_ai_target(&world, None),
                    mobs::TargetChange::Set(Some(Who::Mob(target.id)))
                );
                assert_eq!(actor.ai.target, Some(Who::Mob(target.id)));
                if !brain {
                    assert!(actor.extra.flag);
                    assert_eq!(actor.extra.counter, 321);
                }
            }
            assert_eq!(
                *calls.lock().unwrap(),
                [
                    (9001, false, None),
                    (9001, false, None),
                    (9001, false, Some(9010)),
                    (9001, false, None),
                    (9002, true, None),
                    (9002, true, None),
                    (9002, true, Some(9010)),
                    (9002, true, None),
                ]
            );
        });
    });
}
