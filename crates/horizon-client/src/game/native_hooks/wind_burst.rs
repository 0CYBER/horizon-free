use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Before a wind burst interacts with blocks, `blocks` holds the shuffled
    /// candidate positions in `dimension`. Remove positions to spare those
    /// blocks; entity knockback and the shuffle's random draws are unchanged.
    /// Not called when mob griefing prevents a breeze from interacting with
    /// blocks.
    pub(in crate::game) fn native_wind_burst(
        &mut self,
        dimension: &str,
        center: [f64; 3],
        blocks: &mut Vec<[i32; 3]>,
    ) {
        self.fire_wind_burst(dimension, center, blocks);
    }
}
