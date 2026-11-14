/// The `entity_move` hook's cause (#2414), as the fixture's stand-in for
/// the mod API's own `MoveCause`: vanilla's five mover names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum MoveCause {
    SelfMovement,
    Player,
    Piston,
    ShulkerBox,
    Shulker,
}
