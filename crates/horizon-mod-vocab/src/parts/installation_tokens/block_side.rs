//! Construction for API-owned fixtures.
/// Build the typed value without exposing its native representation.
#[cfg(any(test, feature = "testing"))]
pub fn block_side(
    level: crate::LevelRef,
    pos: [i32; 3],
    side: Option<crate::Direction>,
) -> crate::BlockSide {
    crate::BlockSide { level, pos, side }
}
