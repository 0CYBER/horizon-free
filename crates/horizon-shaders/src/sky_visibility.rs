//! Independent visibility requests from `shaders.properties`.

use crate::Properties;

/// Geometry supplied to a shader pack. Only explicit `false` disables a part; absent or
/// unrecognized values preserve Horizon's existing draws.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SkyVisibility {
    pub sky: bool,
    pub stars: bool,
    pub sun: bool,
    pub moon: bool,
}

impl Default for SkyVisibility {
    fn default() -> Self {
        Self {
            sky: true,
            stars: true,
            sun: true,
            moon: true,
        }
    }
}

impl SkyVisibility {
    pub fn parse(properties: &Properties) -> Self {
        let enabled = |name| {
            properties
                .get(name)
                .is_none_or(|value| value.trim() != "false")
        };
        Self {
            sky: enabled("sky"),
            stars: enabled("stars"),
            sun: enabled("sun"),
            moon: enabled("moon"),
        }
    }
}
