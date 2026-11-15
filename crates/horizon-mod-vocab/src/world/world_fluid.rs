//! Mod-owned world fluid registration values.
/// Simulation and bucket settings for a declared world fluid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub struct WorldFluid {
    /// Ticks between flow steps, at least one.
    pub tick_delay: i32,
    /// Amount lost per block, from one to seven.
    pub drop_off: i32,
    /// Search distance for a way down, from one to four.
    pub slope_find_distance: i32,
    /// Whether two sources can create another source.
    pub source_conversion: bool,
    /// Whether a filled bucket is registered.
    pub bucket: bool,
}
impl Default for WorldFluid {
    fn default() -> Self {
        Self {
            tick_delay: 5,
            drop_off: 1,
            slope_find_distance: 4,
            source_conversion: false,
            bucket: true,
        }
    }
}

impl WorldFluid {
    /// Refuses values the simulation cannot run: no step, no spread, or an unbounded search.
    pub fn check(&self) -> Result<(), String> {
        if self.tick_delay < 1 {
            return Err("tick_delay is 1 or more".into());
        }
        if !(1..=7).contains(&self.drop_off) {
            return Err("drop_off is 1 to 7".into());
        }
        if !(1..=4).contains(&self.slope_find_distance) {
            return Err("slope_find_distance is 1 to 4".into());
        }
        Ok(())
    }
}
