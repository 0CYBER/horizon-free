//! A server's workstation menus on this client: the server opens the menu
//! (`ClientboundOpenScreenPacket` with the station's `MenuType`), the
//! station screen draws it, and its clicks and buttons are predicted on the
//! menu engine (`Game::menu_session`) and sent, as
//! `MultiPlayerGameMode.handleContainerInput` and `handleInventoryButtonClick`
//! do. The menu's client-side `slotsChanged` follows the server's slots, and
//! its data slots come in `ClientboundContainerSetDataPacket`.
use super::{
    Game, Screen,
    stations::{Kind, Station},
};
use horizon_protocol::packets::play::{self, Serverbound as S};

impl Game<'_> {
    /// The server opened workstation menu `menu` in window `container`.
    pub(super) fn remote_open_station(
        &mut self,
        container: i32,
        menu: String,
        kind: Kind,
        title: String,
        pos: [i32; 3],
    ) {
        if let Some(r) = self.remote.as_mut() {
            r.menu = Some((container, menu));
            r.menu_title = Some(title);
        }
        self.craft_size = 2;
        self.container = None;
        self.station = Some(Station::new_kind(kind, pos));
        self.screen = Screen::Inventory;
        self.capture(false);
    }

    /// Window `container` is the server's workstation menu, open here.
    pub(super) fn remote_station_open(&self, container: i32) -> bool {
        self.station.is_some()
            && self
                .remote
                .as_ref()
                .and_then(|r| r.menu.as_ref())
                .is_some_and(|(id, name)| {
                    *id == container && Self::remote_station_kind(name).is_some()
                })
    }

    /// The server set station slot `internal`: the client menu's
    /// `Slot.set` runs its container's `setChanged`, so the loom lists the
    /// patterns its inputs allow.
    pub(super) fn remote_station_slot_changed(&mut self, internal: usize) {
        if !self.station.as_ref().is_some_and(|s| s.kind == Kind::Loom) {
            return;
        }
        self.menu_session(|menu, env, to_menu| {
            let i = to_menu(internal)?;
            let stack = menu.slot(i).cloned();
            menu.set_slot(env, i, stack);
            Some(true)
        });
    }

    /// `AbstractContainerMenu.setData` from the server.
    pub(super) fn remote_station_data(&mut self, id: usize, value: i16) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        // LoomMenu.selectedBannerPatternIndex.
        if (s.kind, id) == (Kind::Loom, 0) {
            s.loom.selected = usize::try_from(value).ok();
        }
    }

    /// `MultiPlayerGameMode.handleInventoryButtonClick`: the screen already
    /// ran `clickMenuButton` on its own menu.
    pub(super) fn remote_station_button(&self, button: i32) {
        if let Some((container, _)) = self.remote.as_ref().and_then(|r| r.menu.as_ref())
            && self.remote_station_open(*container)
        {
            self.send(S::ContainerButtonClick(play::ContainerButtonClick {
                container: *container,
                button,
            }));
        }
    }
}
