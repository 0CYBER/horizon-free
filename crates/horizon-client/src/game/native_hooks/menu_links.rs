//! Linked containers named by subscribed mods.
use super::*;

impl Game<'_> {
    pub(in crate::game) fn native_menu_links(
        &mut self,
        player: u128,
        menu: &str,
        pos: [i32; 3],
        links: &mut Vec<[i32; 3]>,
    ) {
        self.fire_menu_links(player, menu, pos, links);
    }
}
