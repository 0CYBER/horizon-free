use super::*;
use horizon_client::mobs::Mob;

impl Game<'_> {
    /// Decide whether a feeding interaction may start love mode.
    pub(in crate::game) fn native_entity_enter_love_mode_pre(
        &mut self,
        call: mod_api::__private::EntityEnterLoveModePreCall<'_>,
    ) {
        self.fire_entity_enter_love_mode_pre(call);
    }

    /// The current mob index and whether to start love mode. A listener may
    /// move or remove the animal, in which case the interaction ends here.
    pub(in crate::game) fn animal_love_decision(
        &mut self,
        index: usize,
        eligible: bool,
    ) -> Option<(usize, bool)> {
        if !eligible
            || !self.is_logical_server()
            || !mod_api::__private::listened(mod_api::HookId::EntityEnterLoveModePre)
        {
            return Some((index, eligible));
        }
        let id = self.mobs[index].id;
        let (Some(animal), Some(level)) = (self.entity(id), self.level(self.dim.id())) else {
            return Some((index, true));
        };
        let mut canceled = false;
        let feeder = self.native_acting_player();
        self.native_entity_enter_love_mode_pre(mod_api::__private::EntityEnterLoveModePreCall {
            animal,
            level,
            feeder,
            canceled: &mut canceled,
        });

        let index = self.feeding_animal_index(index, id)?;
        let animal = &self.mobs[index];
        let allowed = !canceled && animal.extra.age == 0 && animal.extra.in_love <= 0;
        Some((index, allowed))
    }

    /// Mutable hooks may remove or move mobs. Keep the original target if it is still here.
    pub(in crate::game) fn feeding_animal_index(&self, index: usize, id: u32) -> Option<usize> {
        let still_here = |mob: &Mob| mob.id == id && mob.alive() && !mob.removed;
        if self.mobs.get(index).is_some_and(still_here) {
            Some(index)
        } else {
            self.mobs.iter().position(still_here)
        }
    }
}
