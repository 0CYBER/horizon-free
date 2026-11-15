//! Reads of live mobs lent to the target-hook context during a mob tick.
//!
//! These use the same mob state and methods as the game's ordinary host. A
//! snapshot is intentionally not accepted: it cannot answer these reads.
use super::{GoalCx, Resolved};
use crate::mobs::{self, Mob};
use mod_api::{LivingRef, MobGoalEntry, MobRef, PathType, RaidRef};

impl GoalCx<'_, '_> {
    fn live_mob(&self, mob: MobRef) -> Option<&Mob> {
        match self.resolve(mob.entity())? {
            Resolved::Own if !self.m.removed => Some(self.m),
            Resolved::Live(mob) if !mob.removed => Some(mob),
            _ => None,
        }
    }

    pub(super) fn r_mob_goals(&self, mob: MobRef) -> Option<Vec<MobGoalEntry>> {
        Some(
            self.live_mob(mob)?
                .ai
                .goals
                .entries()
                .map(|(priority, name, running)| MobGoalEntry {
                    priority,
                    id: super::super::native_mob_goals::goal_id(name),
                    running,
                })
                .collect(),
        )
    }

    pub(super) fn r_mob_target_goals(&self, mob: MobRef) -> Option<Vec<MobGoalEntry>> {
        Some(
            self.live_mob(mob)?
                .ai
                .targets
                .entries()
                .map(|(priority, name, running)| MobGoalEntry {
                    priority,
                    id: super::super::native_mob_goals::goal_id(name),
                    running,
                })
                .collect(),
        )
    }

    pub(super) fn r_mob_is_done(&self, mob: MobRef) -> Option<bool> {
        Some(self.live_mob(mob)?.ai.nav.is_done())
    }

    pub(super) fn r_mob_navigation_speed(&self, mob: MobRef) -> Option<f64> {
        Some(self.live_mob(mob)?.ai.nav.speed_modifier)
    }

    pub(super) fn r_mob_pathfinding_malus(&self, mob: MobRef, kind: PathType) -> Option<f32> {
        Some(self.live_mob(mob)?.malus(kind))
    }

    pub(super) fn r_mob_can_use_portal(&self, mob: MobRef) -> Option<bool> {
        Some(self.w.goal_host?.mob_can_use_portal(self.live_mob(mob)?))
    }

    pub(super) fn r_mob_target(&self, mob: MobRef) -> Option<LivingRef> {
        self.w
            .goal_host?
            .mob_target(self.live_mob(mob)?, self.m, self.w)
    }

    pub(super) fn r_mob_is_aggressive(&self, mob: MobRef) -> Option<bool> {
        Some(self.live_mob(mob)?.ai.aggressive)
    }

    pub(super) fn r_mob_persistence_required(&self, mob: MobRef) -> Option<bool> {
        Some(self.live_mob(mob)?.persistent)
    }

    pub(super) fn r_mob_requires_custom_persistence(&self, mob: MobRef) -> Option<bool> {
        Some(mobs::ai::requires_custom_persistence(self.live_mob(mob)?))
    }

    pub(super) fn r_mob_no_ai(&self, mob: MobRef) -> Option<bool> {
        Some(self.live_mob(mob)?.no_ai)
    }

    pub(super) fn r_mob_restrict_center(&self, mob: MobRef) -> Option<[i32; 3]> {
        Some(self.live_mob(mob)?.ai.home_position())
    }

    pub(super) fn r_mob_restrict_radius(&self, mob: MobRef) -> Option<i32> {
        Some(self.live_mob(mob)?.ai.home.map_or(-1, |(_, radius)| radius))
    }

    pub(super) fn r_mob_has_restriction(&self, mob: MobRef) -> Option<bool> {
        Some(self.live_mob(mob)?.ai.home.is_some())
    }

    pub(super) fn r_mob_is_within_restriction(&self, mob: MobRef, at: [i32; 3]) -> Option<bool> {
        Some(self.live_mob(mob)?.ai.is_within_home(at))
    }

    pub(super) fn r_mob_left_handed(&self, mob: MobRef) -> Option<bool> {
        Some(self.live_mob(mob)?.extra.left_handed)
    }

    pub(super) fn r_mob_can_pick_up_loot(&self, mob: MobRef) -> Option<bool> {
        Some(self.live_mob(mob)?.can_pick_up_loot())
    }

    pub(super) fn r_mob_no_action_time(&self, mob: MobRef) -> Option<i32> {
        Some(self.live_mob(mob)?.ai.no_action_time)
    }

    pub(super) fn r_mob_raid(&self, mob: MobRef) -> Option<RaidRef> {
        let raid_id = mobs::raider::state(self.live_mob(mob)?)?.raid_id?;
        let level = self.host.level(self.w.level.id())?;
        mod_api::__private::backend_read(self.host, |backend| backend.level_raid(level, raid_id))
            .flatten()
    }
}
