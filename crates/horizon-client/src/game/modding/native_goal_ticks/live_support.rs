//! Reads that combine a lent mob with authoritative game state.
use super::{GoalCx, Resolved};
use crate::mobs::Mob;
use mod_api::EntityRef;

impl GoalCx<'_, '_> {
    pub(super) fn r_entity_on_block(&self, entity: EntityRef) -> Option<[i32; 3]> {
        let mob = match self.resolve(entity)? {
            Resolved::Own if !self.m.removed => &*self.m,
            Resolved::Live(mob) if !mob.removed => mob,
            _ => return None,
        };
        Some(self.w.goal_host?.mob_on_block(mob))
    }

    pub(super) fn r_entity_entities_team_damage_allowed(
        &self,
        entity: EntityRef,
        source: EntityRef,
    ) -> Option<bool> {
        let mob = |handle| -> Option<Option<&Mob>> {
            match self.resolve(handle) {
                Some(Resolved::Own) if !self.m.removed => Some(Some(&*self.m)),
                Some(Resolved::Live(mob)) if !mob.removed => Some(Some(mob)),
                Some(
                    Resolved::Seen(_) | Resolved::Own | Resolved::Live(_) | Resolved::Unavailable,
                ) => None,
                _ if self.is_game_entity(handle) => Some(None),
                _ => None,
            }
        };
        self.w.goal_host?.entities_team_damage_allowed(
            entity,
            mob(entity)?,
            source,
            mob(source)?,
            self.m,
            self.w,
        )
    }
}
