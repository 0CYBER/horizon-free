use crate::geometry::Direction;

/// A neighbour update a block asks for when it is removed, each the one vanilla's own removal
/// uses (`Level.updateNeighborsAt`, `updateNeighborsAtExceptFromFacing`,
/// `updateNeighbourForOutputSignal`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NeighbourUpdate {
    /// `Level.updateNeighbourForOutputSignal`: comparators beside the position re-read.
    OutputSignal,
    /// `Level.updateNeighborsAt`: all six neighbours hear the block changed.
    Neighbours,
    /// `Level.updateNeighborsAtExceptFromFacing`: all neighbours but the one in the direction.
    NeighboursExcept(Direction),
}
