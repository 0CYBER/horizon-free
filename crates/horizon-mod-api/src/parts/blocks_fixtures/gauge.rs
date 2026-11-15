/// A comparator reading of 5 more than it was.
pub(crate) struct Gauge;
#[mod_api::horizon_part]
impl BlockPart for Gauge {
    #[cfg_attr(test, inline)]
    fn analog_output(&self, _: &dyn LevelHost, event: &mut AnalogOutputEvent) {
        event.output += 5;
    }
    #[cfg_attr(test, cfg(any()))]
    fn should_check_weak_power(&self, _: &dyn LevelHost, _: &mut ShouldCheckWeakPowerEvent) {}
}
