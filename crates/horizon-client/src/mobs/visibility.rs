//! How visible a mob is to the mob that looks for it
//! (`LivingEntity.getVisibilityPercent`).

use super::{MobWorld, Seen, Species};

/// What decides how visible a mob is to another: its armour cover and the worn
/// head that lowers its `mob_visibility`.
#[derive(Clone, Copy, Debug, Default)]
pub struct Cover {
    /// The share of armour slots filled (`getArmorCoverPercentage`), which
    /// scales how visible an invisible mob is.
    pub armor: f32,
    /// The item on its head, when it is one of the mob heads whose
    /// `mob_visibility` lowers it.
    pub head: Option<&'static str>,
}

/// The mob heads that have a `mob_visibility` component.
const MOB_HEADS: [&str; 4] = [
    "minecraft:skeleton_skull",
    "minecraft:zombie_head",
    "minecraft:creeper_head",
    "minecraft:piglin_head",
];

/// The [`Cover`] of `m`: its armour slots and its head.
pub fn cover(m: &super::Mob) -> Cover {
    let equipment = &m.extra.equipment;
    let filled = horizon_items::equipment::ARMOR
        .iter()
        .filter(|slot| equipment.get(**slot).is_some())
        .count();
    Cover {
        armor: filled as f32 / 4.0,
        head: equipment
            .get(horizon_items::equipment::HEAD)
            .and_then(|stack| MOB_HEADS.into_iter().find(|head| *head == &*stack.item)),
    }
}

/// The `mob_visibility` component vanilla gives an item by default
/// (`Item.Properties.loweredMobVisibility`): how visible its wearer is to a
/// mob of type `looking`, 1 for every other item and mob. Only the mob
/// heads have one, each for exactly the types named (a skeleton skull fools
/// a skeleton, not a stray or a bogged), and all four are head equipment,
/// the slot `PlayerView::head` holds.
pub fn mob_visibility(item: &str, looking: Species) -> f32 {
    let lowered = match item {
        "minecraft:skeleton_skull" => looking == Species::Skeleton,
        "minecraft:zombie_head" => looking == Species::Zombie,
        "minecraft:creeper_head" => looking == Species::Creeper,
        "minecraft:piglin_head" => matches!(looking, Species::Piglin | Species::PiglinBrute),
        _ => false,
    };
    if lowered { 0.5 } else { 1.0 }
}

impl Seen {
    /// `LivingEntity.getVisibilityPercent(level, looking)` of a mob before the
    /// visibility event and the clamp: invisibility by armour cover, and the
    /// worn head's `mob_visibility` for the `looking` mob's type. Sneaking is
    /// left out: vanilla reads it through `isDiscrete`, which a mob lacks.
    pub fn visibility_percent(&self, looking: Species) -> f64 {
        let mut visibility = 1.0;
        if self.invisible {
            visibility *= 0.7 * f64::from(self.cover.armor.max(0.1));
        }
        if let Some(head) = self.cover.head {
            visibility *= f64::from(mob_visibility(head, looking));
        }
        visibility
    }
}

impl MobWorld<'_> {
    /// The squared range from which mob `looking` sees the mob `target`, with the
    /// 2-block floor `TargetingConditions.test` keeps: `range` times its
    /// visibility, which a mod's `living_visibility` scales last and which
    /// clamps to 0..=10 as vanilla's does.
    pub fn reach_sqr(&self, target: u32, looking: &super::Mob, range: f64) -> f64 {
        let Some(seen) = self.seen(target) else {
            return range * range;
        };
        let visibility = seen.visibility_percent(looking.species());
        let visibility = match self.mob_hooks {
            Some(hooks) => hooks.living_visibility_mob(target, looking.id, visibility),
            None => visibility,
        };
        (range * visibility.clamp(0.0, 10.0)).max(2.0).powi(2)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::goals::{Cx, Goal, NearestTarget};
    use crate::mobs::world::with_test_world;
    use crate::mobs::{Mob, MobHooks, PlayerView};
    use horizon_core::geometry::Vec3;

    /// A mod's `living_visibility` that hides every zombie from everyone.
    struct HideZombies;
    impl MobHooks for HideZombies {
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
        fn living_visibility_mob(&self, target: u32, _: u32, _: f64) -> f64 {
            if target == 2 { 0.0 } else { 1.0 }
        }
    }

    /// Whether a golem at the origin picks the zombie `distance` blocks away
    /// as prey, with `prepare` changing the zombie and `hooks` set on the world.
    fn golem_targets(
        distance: f64,
        prepare: impl FnOnce(&mut Mob),
        hooks: Option<&dyn MobHooks>,
    ) -> Option<bool> {
        let mut seen = None;
        with_test_world(&[], |_, types| {
            let kind = std::sync::Arc::new(types.types["minecraft:zombie"].clone());
            let mut zombie = Mob::new(2, kind, Vec3::new(0.5 + distance, 64.0, 0.5), 0.0);
            prepare(&mut zombie);
            seen = Some(zombie.seen(None, &crate::items::Items::empty()));
        });
        // No prepared data: the world did not run, as other mob tests skip.
        let seen = seen?;
        let mut result = None;
        with_test_world(&[seen], |w, types| {
            let mut w = w.clone();
            w.mob_hooks = hooks;
            let kind = std::sync::Arc::new(types.types["minecraft:iron_golem"].clone());
            let mut golem = Mob::new(1, kind, Vec3::new(0.5, 64.0, 0.5), 0.0);
            let mut goal = NearestTarget::species(&[Species::Zombie], false).interval(0);
            let (mut test_level, mut events) = (crate::health::TestLevel::new(0), Vec::new());
            let level = &mut test_level.level();
            let mut cx = Cx {
                w: &w,
                level,
                events: &mut events,
            };
            result = Some(goal.can_use(&mut golem, &mut cx));
        });
        result
    }

    /// `TargetingConditions.test` shrinks the range by the target's
    /// visibility: an invisible zombie with no armour is 7% as visible, so a
    /// golem 10 blocks away does not find it, and one in full armour is.
    #[test]
    #[ignore = "daily"]
    fn an_invisible_mob_is_found_from_less_far() {
        let invisible = |m: &mut Mob| m.invisible = true;
        let Some(visible) = golem_targets(10.0, |_| {}, None) else {
            return;
        };
        assert!(visible, "a visible zombie is found");
        assert_eq!(
            golem_targets(10.0, invisible, None),
            Some(false),
            "16 x 0.07, floored to 2 blocks"
        );
        assert_eq!(
            golem_targets(1.5, invisible, None),
            Some(true),
            "within the 2-block floor"
        );
    }

    /// A mod's visibility answer scales a mob target's range as a player's.
    #[test]
    #[ignore = "daily"]
    fn a_mods_visibility_scales_a_mobs_range_to_a_mob() {
        assert_ne!(golem_targets(10.0, |_| {}, Some(&HideZombies)), Some(true));
        assert_ne!(
            golem_targets(1.5, |_| {}, Some(&HideZombies)),
            Some(false),
            "the floor is 2 blocks"
        );
    }
}
