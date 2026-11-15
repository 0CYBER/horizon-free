pub(super) struct ApplyEffectTickHooks;
impl ApplyEffectTickHooks {
    pub(super) fn apply_effect_tick(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::ApplyEffectTickEvent<'_>,
    ) {
        event.heal(2.5);
        event.hurt("minecraft:wither", 3.);
        event.eat(2, 0.25);
        event.add_exhaustion(0.5);
        event.keep = false;
    }
}
