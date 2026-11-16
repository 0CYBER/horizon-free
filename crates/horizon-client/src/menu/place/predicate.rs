//! The same augmenting-path assignment serves authoritative predicates and UI availability.
use super::picker::Picker;
pub fn can_craft_matching(
    amounts: &mut [i32],
    ingredients: usize,
    limit: impl FnMut(usize, usize) -> i32,
) -> bool {
    ingredients != 0 && Picker::new(amounts, ingredients, limit).try_pick(1, None)
}
