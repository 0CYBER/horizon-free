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
        title: horizon_client::text::Text,
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
    /// patterns its inputs allow and the stonecutter the recipes of its input.
    pub(super) fn remote_station_slot_changed(&mut self, internal: usize) {
        if !self.station.as_ref().is_some_and(|s| {
            matches!(
                s.kind,
                Kind::Loom
                    | Kind::Stonecutter
                    | Kind::Grindstone
                    | Kind::Anvil
                    | Kind::Smithing
                    | Kind::Cartography
            )
        }) {
            return;
        }
        self.menu_session(|menu, env, to_menu| {
            let i = to_menu(internal)?;
            let stack = menu.slot(i).cloned();
            menu.set_slot(env, i, stack);
            Some(true)
        });
        // AnvilScreen.slotChanged: the box shows the new input's name, which
        // its responder sends.
        self.remote_anvil_rename();
    }

    /// `AbstractContainerMenu.setData` from the server.
    pub(super) fn remote_station_data(&mut self, id: usize, value: i16) {
        let Some(s) = self.station.as_mut() else {
            return;
        };
        match (s.kind, id) {
            // LoomMenu.selectedBannerPatternIndex.
            (Kind::Loom, 0) => s.loom.selected = usize::try_from(value).ok(),
            // StonecutterMenu.selectedRecipeIndex.
            (Kind::Stonecutter, 0) => s.selected = usize::try_from(value).ok(),
            // AnvilMenu.cost.
            (Kind::Anvil, 0) => s.anvil.cost = i32::from(value),
            // EnchantmentMenu: costs, the seed (the screen's glyph names),
            // the clues' enchantments and levels.
            (Kind::Enchanting, 0..3) => s.offers.costs[id] = i32::from(value),
            (Kind::Enchanting, 3) => self.level.xp_seed = i32::from(value),
            (Kind::Enchanting, 4..7) => {
                let name = self.remote.as_ref().and_then(|r| {
                    r.registries
                        .name("minecraft:enchantment", i32::from(value))
                        .map(std::sync::Arc::<str>::from)
                });
                let level = s.offers.clues[id - 4].as_ref().map_or(1, |(_, l)| *l);
                s.offers.clues[id - 4] = name.map(|n| (n, level));
            }
            (Kind::Enchanting, 7..10) => {
                if let Some((_, level)) = s.offers.clues[id - 7].as_mut() {
                    *level = value.max(0) as u32;
                }
            }
            // BeaconMenu: the levels, then the powers (BeaconScreen's
            // dataChanged shows the menu's).
            (Kind::Beacon, 0) => s.beacon_levels = Some(i32::from(value)),
            (Kind::Beacon, 1 | 2) => {
                let effect = self.remote_effect_id(i32::from(value) - 1);
                let index =
                    effect.and_then(|e| super::beacon::EFFECTS.iter().position(|(n, _)| *n == e));
                if let Some(s) = self.station.as_mut() {
                    s.beacon[id - 1] = index;
                }
            }
            _ => {}
        }
    }

    /// `AnvilScreen.onNameChanged` on a server: the rename box's text (blank
    /// when it is the input's own name) goes to the server when the menu's
    /// `setItemName` takes it.
    pub(super) fn remote_anvil_rename(&mut self) {
        if self.remote.is_none() {
            return;
        }
        let Some(s) = self
            .station
            .as_ref()
            .filter(|s| s.kind == Kind::Anvil && s.inputs[0].is_some())
        else {
            return;
        };
        let name = self.anvil_item_name(s);
        if s.anvil_sent.as_deref() == Some(&name) {
            return;
        }
        if let Some(s) = self.station.as_mut() {
            s.anvil_sent = Some(name.clone());
        }
        self.send(S::RenameItem(play::RenameItem { name }));
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
