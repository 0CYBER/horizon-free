//! Which structures contain a position (`StructureManager.getStructureWithPieceAt`).
use super::Game;

impl Game<'_> {
    /// The structures with a piece containing `pos`.
    pub(super) fn structures_at(&self, _pos: [i32; 3]) -> Vec<String> {
        Vec::new()
    }
}
