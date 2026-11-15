use super::*;

impl GameLevel<'_> {
    /// How a mod's world fluid flows at `pos`, starting from what it declared (NeoForge's
    /// `BaseFlowingFluid` and `FluidType` getters: `getTickDelay`, `getDropOff`,
    /// `getSlopeFindDistance`, `canConvertToSource`). `fluid` is the id it was declared with.
    /// Asked for every flow step and every cell a spread looks at: answer fast. An answer
    /// outside what `WorldFluid` accepts is brought back within it. Server-side.
    #[inline(always)]
    pub(in crate::game) fn native_fluid_rules(&self, call: mod_api::__private::FluidRulesCall<'_>) {
        self.fire_fluid_rules(call);
    }

    /// What a mod's world fluid does at `pos` in `dimension`: `registered`, then as the parts of the
    /// fluid answer through `dispatch`, brought back within what `WorldFluid` accepts. The one place
    /// that builds the hook's call, for a block-behaviour level and for a mod's read alike.
    pub(in crate::game) fn answered_fluid_rules(
        dimension: horizon_core::dimension::Dimension,
        pos: BlockPos,
        fluid: horizon_world::fluid::ModFluid,
        registered: horizon_world::fluid::ModFluidRules,
        dispatch: impl FnOnce(mod_api::__private::FluidRulesCall<'_>),
    ) -> horizon_world::fluid::ModFluidRules {
        let mut rules = registered;
        dispatch(mod_api::__private::FluidRulesCall {
            dimension,
            pos,
            fluid: &fluid.get().id,
            tick_delay: &mut rules.tick_delay,
            drop_off: &mut rules.drop_off,
            slope_find_distance: &mut rules.slope_find_distance,
            converts_to_source: &mut rules.converts_to_source,
        });
        rules.tick_delay = rules.tick_delay.max(1);
        rules.drop_off = rules.drop_off.clamp(1, 7);
        rules.slope_find_distance = rules.slope_find_distance.clamp(1, 4);
        rules
    }
}
