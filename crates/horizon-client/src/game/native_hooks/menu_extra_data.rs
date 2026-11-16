use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Block `block` at `pos` opens its mod menu type `menu` for server
    /// player `player`: set `data` to hand the menu's screen bytes of your
    /// own (a machine's tier, mode or recipes), up to 32 KiB, which the
    /// client reads with `screen_extra_data` while the screen is open
    /// (NeoForge's `openMenu` with an extra-data writer). The first mod to
    /// set it wins, so answer only for your blocks. The player's state is not
    /// swapped in. 401.
    pub(in crate::game) fn native_menu_extra_data(
        &mut self,
        player: u128,
        menu: &str,
        block: &str,
        pos: [i32; 3],
        data: &mut Option<Vec<u8>>,
    ) {
        self.fire_menu_extra_data(player, menu, block, pos, data);
    }
}

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// The block of the mod screen that is open (`native_screen`), as the
    /// server named it when the block opened its menu type; None when no mod
    /// screen is open. 401.
    pub(in crate::game) fn screen_pos(&self) -> Option<[i32; 3]> {
        self.shown_native_screen()?.pos()
    }
    /// What the block's mod wrote for the mod screen that is open
    /// (`native_menu_extra_data` on the server), empty when it wrote nothing;
    /// None when no mod screen is open. @API:menu_extra_data@.
    pub(in crate::game) fn screen_extra_data(&self) -> Option<Vec<u8>> {
        let screen = self.shown_native_screen()?;
        Some(
            screen
                .extra
                .as_ref()
                .map_or_else(Vec::new, |e| e.data.clone()),
        )
    }
    /// The `horizon:menu_extra` payload the server sent just before its
    /// `OpenScreen`, in the layout its revision query in configuration chose:
    /// the current one from a server that asked, the legacy one from a server
    /// that never did (an older Horizon server).
    pub(in crate::game) fn receive_menu_extra(&mut self, data: &[u8]) {
        let revision = self
            .remote
            .as_ref()
            .map_or(0, |r| r.registries.menu_extra_revision);
        let decoded = if revision >= 1 {
            horizon_protocol::menu_extra::decode(data)
        } else {
            horizon_protocol::menu_extra::decode_legacy(data)
        };
        match decoded {
            Ok(extra) => {
                if let Some(r) = self.remote.as_mut() {
                    r.menu_extra = Some(extra);
                }
            }
            Err(e) => log::debug!("Horizon: a bad menu extra payload: {e}"),
        }
    }
}
