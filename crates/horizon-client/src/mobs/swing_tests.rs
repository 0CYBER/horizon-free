//! `Mob.swingForAttack(hand)`: an AI melee swing plays the held stack's
//! `attack_animation` (`ItemStack.getAttackAnimation`) for as long as
//! `getModifiedSwingDuration` says (haste, conduit power, mining fatigue), and
//! `SwingState.startIfAble` refuses a swing asked for inside the first half of
//! the current one, `/swing` included.
use crate::first_person::{Hand, SwingAnimation, SwingState};
use crate::items::ItemStack;
use crate::mobs::ai::tick_swing;
use crate::mobs::{Mob, Species};
use horizon_core::geometry::Vec3;
use std::sync::Arc;

/// The repro's zombie: right-handed, holding nothing yet.
fn zombie() -> Mob {
    let mut kind = crate::mobs::sensor_scan_tests::kind();
    let kind_mut = Arc::make_mut(&mut kind);
    kind_mut.species = Species::Zombie;
    kind_mut.id = "minecraft:zombie".into();
    Mob::new(1, kind, Vec3::ZERO, 0.0)
}

/// The repro: a zombie holding an iron spear thrusts for the spear's 19 ticks
/// (`(int)(0.95F * 20.0F)`) as a STAB of the main arm; a bare-handed zombie
/// whacks for `SwingAnimation.DEFAULT`'s 6.
#[test]
fn a_melee_swing_plays_the_held_weapons_animation() {
    let mut m = zombie();
    m.extra
        .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
    m.swing_for_attack(false);
    assert!(m.ai.swinging, "the attack swings");
    assert_eq!(m.swing_duration(), 19, "the iron spear's attack animation");
    assert!(m.extra.swing_stab, "a STAB, not the default whack");
    let main_arm = !m.main_arm_left();
    assert_eq!(
        m.extra.swing_right_arm,
        Some(main_arm),
        "the main hand's arm"
    );

    let mut bare = zombie();
    bare.swing_for_attack(false);
    assert_eq!(bare.swing_duration(), 6, "SwingAnimation.DEFAULT");
    assert!(!bare.extra.swing_stab);
}

/// `getModifiedSwingDuration`: haste (and conduit power) shorten the swing,
/// mining fatigue lengthens it.
#[test]
fn haste_and_mining_fatigue_change_the_swing_length() {
    let spear = || {
        let mut m = zombie();
        m.extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
        m
    };
    let mut fast = spear();
    fast.extra
        .effects
        .push(crate::mobs::StoredEffect::new("minecraft:haste", 200, 1));
    fast.swing_for_attack(false);
    assert_eq!(fast.swing_duration(), 17, "19 - (1 + haste's amplifier)");

    let mut slow = spear();
    slow.extra.effects.push(crate::mobs::StoredEffect::new(
        "minecraft:mining_fatigue",
        200,
        1,
    ));
    slow.swing_for_attack(false);
    assert_eq!(
        slow.swing_duration(),
        23,
        "19 + (1 + mining fatigue's amplifier) * 2"
    );
}

/// A zombie mid-thrust of its iron spear, at vanilla `SwingState.ticks` =
/// `ticks` of the spear's 19 (`ai.swing_time` counts one behind it).
fn thrust_at(ticks: i32) -> Mob {
    let mut m = zombie();
    m.extra
        .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
    m.swing_for_attack(false);
    for _ in 0..ticks {
        tick_swing(&mut m);
    }
    assert_eq!(m.ai.swing_time, ticks - 1, "the swing's tick");
    m
}

/// `SwingState.startIfAble`: `/swing` (`LivingEntity.swing(hand, animation,
/// true)`) inside the first half of a spear's thrust leaves it alone, and past
/// the half restarts it with the command's own animation. Of a 19-tick swing
/// the pinned 26.3 oracle refuses `ticks` 1 through 9 and accepts 0 and 10.
#[test]
fn a_swing_inside_the_first_half_is_refused() {
    for ticks in 1..=9 {
        let mut m = thrust_at(ticks);
        m.swing_hand_modified(false, SwingAnimation::DEFAULT);
        assert_eq!(
            m.ai.swing_time,
            ticks - 1,
            "tick {ticks}: the refused swing leaves the animation"
        );
        assert_eq!(m.swing_duration(), 19, "and the spear's length");
        assert!(m.extra.swing_stab);
    }
    for ticks in [0, 10] {
        let mut m = thrust_at(ticks);
        m.swing_hand_modified(false, SwingAnimation::DEFAULT);
        assert_eq!(
            m.ai.swing_time, -1,
            "tick {ticks}: past the first half it restarts"
        );
        assert_eq!(m.swing_duration(), 6, "with the command's animation");
        assert!(!m.extra.swing_stab);
    }
}

/// The mob's guard and the first-person `SwingState`'s are the same vanilla
/// rule: of a 19-tick swing both refuse `ticks` 1 through 9 and accept 0 and
/// 10, which the mob reads one lower because `ai.swing_time` counts one behind
/// `SwingState.ticks`.
#[test]
fn the_mob_and_the_first_person_guard_refuse_the_same_ticks() {
    for ticks in [0, 1, 8, 9, 10] {
        let mut m = thrust_at(ticks);
        let mut state = SwingState::default();
        assert!(state.start_if_able(Hand::Main, SwingAnimation::DEFAULT, 19));
        for _ in 0..ticks {
            state.tick();
        }
        assert_eq!(
            m.swing_hand_modified(false, SwingAnimation::DEFAULT),
            state.start_if_able(Hand::Main, SwingAnimation::DEFAULT, 6),
            "tick {ticks}"
        );
    }
}

/// The wiring: `MeleeAttackGoal.checkAndPerformAttack` swings through
/// `swingForAttack(MAIN_HAND)`, so the zombie of the repro thrusts its spear
/// when it hits, and the hit still lands.
#[test]
fn a_zombies_melee_goal_swings_the_spear_it_holds() {
    use crate::mobs::goals::{Cx, GoalSelector, MeleeAttack};
    use crate::mobs::world::{test_player, with_test_world};
    use crate::mobs::{MobEvent, Who};

    let mut out = None;
    let ran = with_test_world(&[], |w, types| {
        let kind = Arc::new(types.types["minecraft:zombie"].clone());
        let mut m = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
        m.extra
            .set_main_hand_stack(Some(ItemStack::new("minecraft:iron_spear", 1)));
        m.ai.target = Some(Who::Player);
        let mut world = w.clone();
        // A spear's reach is 1.0 to 2.25 blocks for a mob (its `mob_factor`
        // halves `attack_range`), so the target stands 2.5 blocks away.
        world.player = Some(test_player(Vec3::new(0.5, 64.0, 2.5)));
        let mut goals = GoalSelector::default();
        goals.add(1, MeleeAttack::new(1.0, true));
        let mut fixture = crate::health::TestLevel::new(0);
        let mut events = Vec::new();
        goals.tick(
            &mut m,
            &mut Cx {
                w: &world,
                level: &mut fixture.level(),
                events: &mut events,
            },
        );
        let hit = events
            .iter()
            .any(|e| matches!(e, MobEvent::AttackPlayer(..)));
        out = Some((m.ai.swinging, m.swing_duration(), m.extra.swing_stab, hit));
    });
    if !ran {
        return;
    }
    assert_eq!(
        out,
        Some((true, 19, true, true)),
        "the goal's attack swings the spear for its 19 ticks and still hits"
    );
}
