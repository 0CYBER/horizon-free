//! The shape of a rail (`RailShape`), as rail Parts answer it.
#[allow(unused_imports)]
use crate::*;

/// `RailShape`: the track a rail lays, the value of its `shape` block state property.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: vanilla's closed set of track shapes
#[derive(horizon_mod_api_macros::Wire)]
pub enum RailShape {
    /// Straight, north to south.
    NorthSouth,
    /// Straight, east to west.
    EastWest,
    /// Climbing toward the east.
    AscendingEast,
    /// Climbing toward the west.
    AscendingWest,
    /// Climbing toward the north.
    AscendingNorth,
    /// Climbing toward the south.
    AscendingSouth,
    /// A curve joining south and east.
    SouthEast,
    /// A curve joining south and west.
    SouthWest,
    /// A curve joining north and west.
    NorthWest,
    /// A curve joining north and east.
    NorthEast,
}

impl RailShape {
    /// Every shape, in vanilla's order.
    pub const ALL: [Self; 10] = [
        Self::NorthSouth,
        Self::EastWest,
        Self::AscendingEast,
        Self::AscendingWest,
        Self::AscendingNorth,
        Self::AscendingSouth,
        Self::SouthEast,
        Self::SouthWest,
        Self::NorthWest,
        Self::NorthEast,
    ];

    /// The `shape` property value (`RailShape.getSerializedName`): `north_south`, `ascending_east`...
    pub const fn name(self) -> &'static str {
        match self {
            Self::NorthSouth => "north_south",
            Self::EastWest => "east_west",
            Self::AscendingEast => "ascending_east",
            Self::AscendingWest => "ascending_west",
            Self::AscendingNorth => "ascending_north",
            Self::AscendingSouth => "ascending_south",
            Self::SouthEast => "south_east",
            Self::SouthWest => "south_west",
            Self::NorthWest => "north_west",
            Self::NorthEast => "north_east",
        }
    }

    /// The shape whose `shape` property value is `name`.
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|shape| shape.name() == name)
    }

    /// Whether the track climbs (`RailShape.isSlope`).
    pub const fn is_slope(self) -> bool {
        matches!(
            self,
            Self::AscendingEast | Self::AscendingWest | Self::AscendingNorth | Self::AscendingSouth
        )
    }
}
