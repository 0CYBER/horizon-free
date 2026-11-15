//! Regression coverage for the brain's actual StartAttacking behavior.
use super::*;
use crate::mobs::world::{test_player, with_test_world};
use crate::mobs::{MobHooks, PlayerView, TargetChange};
use std::sync::{Arc, Mutex};

struct Decide {
    outcome: TargetChange,
    calls: Mutex<Vec<(u32, bool, Option<Who>)>>,
}
impl MobHooks for Decide {
    fn enderman_anger_suppressed(&self, _: u32, _: &PlayerView) -> bool {
        false
    }
    fn can_grief(&self, _: u32, rule: bool) -> bool {
        rule
    }
    fn grief_hooked(&self) -> bool {
        false
    }
    fn living_visibility(&self, _: &PlayerView, _: u32, visibility: f64) -> f64 {
        visibility
    }
    fn living_change_target(
        &self,
        mob: &mut Mob,
        _: &MobWorld<'_>,
        brain: bool,
        proposed: Option<Who>,
    ) -> TargetChange {
        self.calls.lock().unwrap().push((mob.id, brain, proposed));
        self.outcome
    }
}

#[test]
#[ignore = "daily"]
fn start_attacking_brain_honors_cancel_and_redirect() {
    // First obtain a real mob view to which the hook can redirect.
    let mut redirected = None;
    assert!(
        with_test_world(&[], |_, types| {
            let kind = Arc::new(types.types["minecraft:iron_golem"].clone());
            redirected = Some(
                Mob::new(8, kind, Vec3::new(2.5, 64.0, 0.5), 0.0)
                    .seen(None, &crate::items::Items::empty()),
            );
        }),
        "prepared entity resources are required"
    );
    let others = [redirected.unwrap()];
    assert!(
        with_test_world(&others, |world, types| {
            let kind = Arc::new(types.types["minecraft:breeze"].clone());
            for (outcome, expected_start, expected_target) in [
                (TargetChange::Keep, false, None),
                (
                    TargetChange::Set(Some(Who::Mob(8))),
                    true,
                    Some(Who::Mob(8)),
                ),
                (TargetChange::Set(Some(Who::Mob(999))), false, None),
            ] {
                let hook = Decide {
                    outcome,
                    calls: Mutex::new(Vec::new()),
                };
                let mut world = world.clone();
                world.player = Some(test_player(Vec3::new(1.5, 64.0, 0.5)));
                world.mob_hooks = Some(&hook);
                let mut mob = Mob::new(7, kind.clone(), Vec3::new(0.5, 64.0, 0.5), 0.0);
                init(&mut mob, 7);
                mem(&mut mob).nearest_attackable.set(Who::Player);
                let mut behavior = B::StartAttacking(false, Finder::NearestAttackable);
                let mut level = crate::health::TestLevel::new(0);
                let mut events = Vec::new();
                let mut cx = Bx {
                    w: &world,
                    level: &mut level.level(),
                    events: &mut events,
                    now: 0,
                };
                assert_eq!(behavior.try_start(&mut mob, &mut cx), expected_start);
                assert_eq!(attack_target(&mob), expected_target);
                assert_eq!(mob.ai.target, expected_target);
                assert_eq!(*hook.calls.lock().unwrap(), [(7, true, Some(Who::Player))]);
            }
        }),
        "prepared entity resources are required"
    );
}
