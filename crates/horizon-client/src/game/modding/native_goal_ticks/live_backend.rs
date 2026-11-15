//! Determines which entity reads can use the ordinary game backend while a
//! mob tick has its active and other mobs on loan.
use super::{GoalCx, Resolved};
use mod_api::EntityRef;

impl GoalCx<'_, '_> {
    pub(super) fn is_game_entity(&self, entity: EntityRef) -> bool {
        if matches!(
            self.resolve(entity),
            Some(Resolved::Own | Resolved::Live(_) | Resolved::Seen(_) | Resolved::Unavailable)
        ) {
            return false;
        }
        self.host.entity(entity.id()) == Some(entity)
    }
}
