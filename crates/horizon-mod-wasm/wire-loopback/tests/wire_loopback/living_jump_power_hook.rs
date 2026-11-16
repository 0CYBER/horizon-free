use super::*;

pub(super) struct LivingJumpPowerHooks;
impl LivingJumpPowerHooks {
    pub(super) fn living_jump_power(
        &self,
        context: &dyn LoopContext,
        event: &mut mod_api::LivingJumpPowerEvent<'_>,
    ) {
        // This is a plain import made through the generated Guest, not a stubbed reply.
        assert!(matches!(context.side(), mod_api::LoopSide::Server));
        event.entity = 99;
        event.power = 7.25;
        event.kind = "must not write back";
    }
}
