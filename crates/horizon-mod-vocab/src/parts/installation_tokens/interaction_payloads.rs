//! Borrowed event values for API-owned registration and dispatch fixtures.
/// Build the click payload at its vocabulary boundary.
#[cfg(any(test, feature = "testing"))]
pub fn block_click<'a>(
    side: crate::LogicalSide,
    dimension: crate::LevelDimension,
    pos: [i32; 3],
    hit: crate::BlockHit,
    hand: crate::Hand,
    stack: Option<&'a crate::ItemStack>,
    player: (u128, i32),
) -> crate::BlockClick<'a> {
    crate::BlockClick {
        side,
        dimension,
        pos,
        hit,
        hand,
        stack,
        player: player.0,
        player_entity_id: player.1,
    }
}
/// Build the `item_fished` event view (the bobber's id and position) at its vocabulary boundary.
#[cfg(any(test, feature = "testing"))]
pub fn item_fished(hook: u32, position: [f64; 3]) -> crate::ItemFished {
    crate::ItemFished { hook, position }
}
