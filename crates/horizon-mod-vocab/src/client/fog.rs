//! Moved here from the game so mods can name it.
#[allow(unused_imports)]
use crate::*;
/// `FogType` of the camera position.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: a closed set the game matches without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum FogType {
    Atmospheric,
    Water,
    Lava,
    PowderSnow,
}
/// The fog uniform (`FogData`).
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
// exhaustive: the game builds it with a literal; mods only read it
#[derive(horizon_mod_api_macros::Wire)]
pub struct FogData {
    pub color: [f32; 4],
    pub environmental_start: f32,
    pub environmental_end: f32,
    pub render_distance_start: f32,
    pub render_distance_end: f32,
    pub sky_end: f32,
    pub cloud_end: f32,
}
