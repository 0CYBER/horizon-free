#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum RecipeTransferResult {
    Moves,
    Missing,
    NoRoom,
}
