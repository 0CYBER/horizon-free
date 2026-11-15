//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// `InteractionHand`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum Hand {
    Main,
    Off,
}
impl Hand {
    pub const BOTH: [Hand; 2] = [Hand::Main, Hand::Off];
}
