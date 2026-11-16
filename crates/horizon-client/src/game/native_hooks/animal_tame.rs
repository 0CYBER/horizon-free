use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// The player `tamer` (UUID) and `tamer_entity_id` is about to tame the
    /// mob `entity` of `kind` (`AnimalTameEvent`): its taming roll succeeded
    /// (a wolf's bone, a cat's fish, a parrot's seeds or a nautilus's food;
    /// a horse, donkey, mule or llama the player rides). Set `cancel` for the
    /// attempt to fail instead, as a failed roll does: smoke for a pet, and
    /// a horse bucks its rider off. Before `entity_tame`. Server-side.
    pub(in crate::game) fn native_animal_tame(
        &mut self,
        tamer: u128,
        tamer_entity_id: i32,
        entity: u32,
        kind: &str,
        cancel: &mut bool,
    ) {
        self.fire_animal_tame(tamer, tamer_entity_id, entity, kind, cancel);
    }
}
