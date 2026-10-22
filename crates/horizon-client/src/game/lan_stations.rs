//! Workstation menus for LAN guests: the screen a guest's use opens runs on
//! the host's menu engine (`horizon_client::menu`), the one single player
//! uses, with vanilla's packets: OpenScreen with the station's `MenuType`,
//! container clicks, `ServerboundContainerButtonClickPacket`, the content and
//! data slot sync after each (`broadcastChanges`), and the inputs back into
//! the inventory when the menu closes (`AbstractContainerMenu.clearContainer`).
use super::{
    Game,
    lan::LanHost,
    lan_crafting::input,
    stations::{Kind as StationKind, Station},
};
use horizon_client::{
    items::ItemStack,
    menu::{Kind, Menu},
};
use horizon_protocol::packets::play::{self, Clientbound as C};

/// A guest's open workstation menu.
pub(super) struct GuestStation {
    window: i32,
    state: i32,
    kind: StationKind,
    pos: [i32; 3],
    menu: Box<Menu>,
    /// The data slots the client was last sent.
    sent_data: Vec<i32>,
    /// The game time of the menu's last sound (`lastSoundTime`).
    last_sound: u64,
}

/// The `MenuType`, the engine's menu and the input slots
/// (`clearContainer`'s container) of a station a guest can use.
fn menu_of(kind: StationKind) -> Option<(&'static str, Kind, std::ops::Range<usize>)> {
    Some(match kind {
        StationKind::Loom => ("minecraft:loom", Kind::Loom, 0..3),
        StationKind::Stonecutter => ("minecraft:stonecutter", Kind::Stonecutter, 0..1),
        StationKind::Grindstone => ("minecraft:grindstone", Kind::Grindstone, 0..2),
        _ => return None,
    })
}

impl Game<'_> {
    /// A workstation a guest's use opened (`ServerPlayer.openMenu`); false
    /// when guests can't use that one yet.
    pub(super) fn lan_open_station(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        station: &Station,
    ) -> bool {
        let Some((menu_type, kind, _)) = menu_of(station.kind) else {
            return false;
        };
        self.lan_station_closed(host, conn, None);
        self.lan_crafting_closed(host, conn, None);
        // The container it replaces loses its opener.
        self.guest_container_closed(host, conn);
        let title = horizon_protocol::text::translatable(station.kind.title_key(), &[]);
        let Some(window) = host.hub.open_window(conn, menu_type, title) else {
            return true;
        };
        host.play.entry(conn).or_default().station = Some(GuestStation {
            window,
            state: 0,
            kind: station.kind,
            pos: station.pos,
            menu: Box::new(Menu::new(kind)),
            sent_data: Vec::new(),
            last_sound: 0,
        });
        self.lan_send_station(host, conn);
        true
    }

    /// Runs `op` on the guest's open station (window `window`, any when
    /// None) with their inventory in it, then sends what changed. False when
    /// no such window is open.
    fn lan_station_op(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: Option<i32>,
        op: impl FnOnce(&mut Menu, &horizon_client::menu::Env),
    ) -> bool {
        let Some(mut gs) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.station.take_if(|s| window.is_none_or(|w| s.window == w)))
        else {
            return false;
        };
        if let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn)
            && !host.hub.guests[gi].spectator()
        {
            self.guest_menu_op(host, gi, &mut gs.menu, op);
            self.lan_station_sounds(&mut gs);
            if let Some(amount) = gs.menu.ground.take() {
                self.grindstone_taken(gs.pos, amount);
            }
        }
        host.play.entry(conn).or_default().station = Some(gs);
        self.lan_send_station(host, conn);
        true
    }

    /// The sounds the menu played, at the station (heard by everyone around,
    /// as `level.playSound(null, pos, ...)`), once per tick.
    fn lan_station_sounds(&mut self, gs: &mut GuestStation) {
        let sounds = std::mem::take(&mut gs.menu.sounds);
        if sounds.is_empty() || gs.last_sound == self.ticks {
            return;
        }
        gs.last_sound = self.ticks;
        let at = gs.pos.map(|v| f64::from(v) + 0.5);
        for sound in sounds {
            self.audio.play(sound, Some(at), 1.0, 1.0);
        }
    }

    /// A click on the guest's station window; false for other windows.
    pub(super) fn lan_station_click(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        p: &play::ContainerClick,
    ) -> bool {
        self.lan_station_op(host, conn, Some(p.container), |menu, env| {
            let _ = menu.click(env, i32::from(p.slot), i32::from(p.button), input(p.input));
        })
    }

    /// `ServerGamePacketListenerImpl.handleContainerButtonClick`:
    /// `clickMenuButton`, then `broadcastChanges` when the menu took it.
    pub(super) fn lan_station_button(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: i32,
        id: i32,
    ) {
        self.lan_station_op(host, conn, Some(window), |menu, env| {
            menu.button(env, id);
        });
    }

    /// The whole station menu and its changed data slots for the client
    /// (`sendAllDataToRemote` / `broadcastChanges`).
    fn lan_send_station(&mut self, host: &mut LanHost, conn: u32) {
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return;
        };
        let Some(gs) = host.play.get_mut(&conn).and_then(|p| p.station.as_mut()) else {
            return;
        };
        gs.state = (gs.state + 1) & 0x7FFF;
        let own = gs.menu.slot_count() - 36;
        let mut items: Vec<Option<ItemStack>> =
            (0..own).map(|s| gs.menu.slot(s).cloned()).collect();
        let g = &host.hub.guests[gi];
        items.extend((9..36).chain(0..9).map(|i| g.inventory[i].clone()));
        let carried = g.carried.clone();
        let packet = C::ContainerSetContent(play::ContainerSetContent {
            container: gs.window,
            state: gs.state,
            items: items.iter().map(|s| host.hub.stack(s)).collect(),
            carried: host.hub.stack(&carried),
        });
        gs.sent_data.resize(gs.menu.data.len(), i32::MIN);
        let mut data = Vec::new();
        for (k, &v) in gs.menu.data.iter().enumerate() {
            if gs.sent_data[k] != v {
                gs.sent_data[k] = v;
                data.push(C::ContainerSetData(play::ContainerSetData {
                    container: gs.window,
                    id: k as i16,
                    value: v as i16,
                }));
            }
        }
        host.hub.send_to(conn, &packet);
        for packet in data {
            host.hub.send_to(conn, &packet);
        }
    }

    /// `AbstractContainerMenu.removed` of the guest's station (`window`;
    /// None closes it whichever it is): the carried stack and the inputs go
    /// back to the inventory, what doesn't fit is dropped. False when no such
    /// window is open.
    pub(super) fn lan_station_closed(
        &mut self,
        host: &mut LanHost,
        conn: u32,
        window: Option<i32>,
    ) -> bool {
        let Some(gs) = host
            .play
            .get_mut(&conn)
            .and_then(|p| p.station.take_if(|s| window.is_none_or(|w| s.window == w)))
        else {
            return false;
        };
        let Some(gi) = host.hub.guests.iter().position(|g| g.conn == conn) else {
            return true;
        };
        // AbstractContainerMenu.removed: the carried stack first, then the inputs.
        let mut back: Vec<ItemStack> = host.hub.guests[gi].carried.take().into_iter().collect();
        back.extend(station_inputs(&gs));
        if !back.is_empty() {
            self.lan_place_back(host, gi, back);
        }
        true
    }

    /// `ServerPlayer.doTick`'s `stillValid`: a guest's station closes once its
    /// block is gone or the guest is out of reach (`isWithinBlockInteractionRange(pos, 4)`).
    pub(super) fn lan_stations_tick(&mut self, host: &mut LanHost) {
        let conns: Vec<u32> = host
            .play
            .iter()
            .filter(|(_, p)| p.station.is_some())
            .map(|(c, _)| *c)
            .collect();
        for conn in conns {
            let Some(g) = host.hub.guests.iter().find(|g| g.conn == conn) else {
                continue;
            };
            if !host.hub.here(g) {
                continue;
            }
            let Some(gs) = host.play.get(&conn).and_then(|p| p.station.as_ref()) else {
                continue;
            };
            let [x, y, z] = gs.pos;
            let name = &self.res.facts().get(self.world.block(x, y, z)).name;
            let block = StationKind::from_block(name) == Some(gs.kind);
            let eye = [g.position[0], g.position[1] + g.eye_height(), g.position[2]];
            // The block interaction range (4.5, creative 5) and the buffer.
            let reach = if g.creative() { 5.0 } else { 4.5 } + 4.0;
            let near = (0..3)
                .map(|k| {
                    let lo = f64::from(gs.pos[k]);
                    (lo - eye[k]).max(eye[k] - lo - 1.0).max(0.0).powi(2)
                })
                .sum::<f64>()
                < reach * reach;
            if !(block && near) {
                let window = gs.window;
                self.lan_station_closed(host, conn, Some(window));
                host.hub.send_to(
                    conn,
                    &C::ContainerClose(play::ContainerClose { container: window }),
                );
            }
        }
    }
}

/// What `clearContainer` gives back from a station's menu.
pub(super) fn station_inputs(gs: &GuestStation) -> Vec<ItemStack> {
    menu_of(gs.kind)
        .map(|(_, _, inputs)| inputs.filter_map(|s| gs.menu.slot(s).cloned()).collect())
        .unwrap_or_default()
}
