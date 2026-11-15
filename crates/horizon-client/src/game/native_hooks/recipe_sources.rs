//! Additional ingredient containers named by subscribed mods.
use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_recipe_sources(
        &mut self,
        player: u128,
        menu: &str,
        pos: [i32; 3],
        sources: &mut Vec<[i32; 3]>,
    ) {
        self.fire_recipe_sources(player, menu, pos, sources);
    }
}
