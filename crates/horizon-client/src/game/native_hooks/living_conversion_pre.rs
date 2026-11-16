use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A mob is about to convert into another type (NeoForge's
    /// `LivingConversionEvent.Pre`, vanilla's `Mob.convertTo`): a zombie
    /// drowning, a piglin or hoglin in the Overworld, a tadpole growing, a
    /// mooshroom sheared by a dispenser, a pig or villager struck by
    /// lightning. `outcome` is the new type. Set `cancel` to keep the mob as
    /// it is; a drowning zombie starts its timers again. Not told: a
    /// mooshroom a player shears, a zombie villager's cure, a villager turned
    /// by a zombie. API 391.
    pub(in crate::game) fn native_living_conversion_pre(
        &mut self,
        entity: u32,
        kind: &str,
        outcome: &str,
        cancel: &mut bool,
    ) {
        self.fire_living_conversion_pre(entity, kind, outcome, cancel);
    }
}
