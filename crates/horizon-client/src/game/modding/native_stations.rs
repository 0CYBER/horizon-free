//! Workstations a native mod opens with no block (`PlayerRef::open_station`, #18347): the
//! crafting table, loom, grindstone, cartography table, stonecutter, smithing table or anvil
//! menu of a "workbench on a stick", as a Java item opens it with a `ContainerLevelAccess` at
//! the player. It works where the player stands at each use, stays open while they carry one
//! of the mod's items, and lets the mods keep its inputs as it closes (`station_closing`).
use super::{Game, stations::Kind as StationKind};
use horizon_client::{items::ItemStack, lan::guest::Guest};
use mod_api::{ItemKey, MenuKey, OpenStationError, PlayerRef};

/// A workstation a mod opened with no block: its window and the items that keep it open.
pub(in crate::game) struct PortableStation {
    window: i32,
    keep: Vec<String>,
}

/// What a mod may open with no block: the crafting table, or a station kind.
#[derive(Clone, Copy)]
enum Portable {
    Table,
    Station(StationKind),
}

fn portable(menu: &str) -> Option<Portable> {
    Some(match menu {
        "minecraft:crafting" => Portable::Table,
        "minecraft:loom" => Portable::Station(StationKind::Loom),
        "minecraft:grindstone" => Portable::Station(StationKind::Grindstone),
        "minecraft:cartography_table" => Portable::Station(StationKind::Cartography),
        "minecraft:stonecutter" => Portable::Station(StationKind::Stonecutter),
        "minecraft:smithing" => Portable::Station(StationKind::Smithing),
        "minecraft:anvil" => Portable::Station(StationKind::Anvil),
        _ => return None,
    })
}

/// The block the player stands in (`Entity.blockPosition`): where a station with no block works.
fn block_pos(g: &Guest) -> [i32; 3] {
    g.position.map(|v| v.floor() as i32)
}

impl Game<'_> {
    /// Runs `step` with guest `conn`'s station marked as asking the mods (`station_busy`): a
    /// hook it asks can't open another station, which the step's write-back would drop with its
    /// inputs. The mark is put back as it was after.
    pub(in crate::game) fn with_station_busy<T>(
        &mut self,
        conn: u32,
        step: impl FnOnce(&mut Self) -> T,
    ) -> T {
        let was = self
            .lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .map(|menus| std::mem::replace(&mut menus.station_busy, true));
        let out = step(self);
        if let Some(was) = was
            && let Some(menus) = self.lan.as_mut().and_then(|host| host.menus.get_mut(&conn))
        {
            menus.station_busy = was;
        }
        out
    }

    /// Opens vanilla workstation menu `menu` for a connected player with no block, its input
    /// slots starting with `inputs`, kept open while their inventory holds one of `keep`.
    pub(super) fn player_open_station(
        &mut self,
        player: PlayerRef,
        menu: MenuKey,
        inputs: Vec<Option<ItemStack>>,
        keep: Vec<ItemKey>,
    ) -> Result<Option<i32>, OpenStationError> {
        let kind = portable(&self.menu_key_id(menu)).ok_or(OpenStationError::NotStation)?;
        let slots = match kind {
            Portable::Table => 9,
            Portable::Station(k) => super::lan_stations::menu_of(k).map_or(0, |(.., r)| r.len()),
        };
        if inputs.len() > slots {
            return Err(OpenStationError::TooManyInputs);
        }
        self.online_player_record(player)
            .ok_or(OpenStationError::PlayerUnavailable)?;
        let conn = self
            .native_conn(player.uuid())
            .ok_or(OpenStationError::PlayerUnavailable)?;
        let pos = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(block_pos)
            .ok_or(OpenStationError::PlayerUnavailable)?;
        // An open asks the mods (the restored grid's `prepare_item_craft`...), and one of them
        // opening another station for the player would replace this one and lose its inputs:
        // refused before any window is sent or input taken.
        let menus = self
            .lan
            .as_mut()
            .ok_or(OpenStationError::PlayerUnavailable)?
            .menus
            .entry(conn)
            .or_default();
        if menus.station_busy {
            return Err(OpenStationError::Busy);
        }
        menus.station_busy = true;
        let keep = keep.into_iter().map(|k| self.item_key_id(k)).collect();
        let window = match kind {
            Portable::Table => self.lan_open_crafting_with(conn, pos, inputs),
            Portable::Station(k) => self.lan_open_station_at(conn, k, pos, inputs).flatten(),
        };
        if let Some(menus) = self.lan.as_mut().and_then(|host| host.menus.get_mut(&conn)) {
            menus.station_busy = false;
        }
        if let Some(window) = window
            && let Some(host) = self.lan.as_mut()
        {
            host.menus.entry(conn).or_default().portable = Some(PortableStation { window, keep });
        }
        Ok(window)
    }

    /// `stillValid` of guest `g`'s menu `window` when a mod opened it with no block: whether
    /// their inventory holds one of its items. None for every other menu.
    pub(in crate::game) fn portable_station_valid(&self, g: &Guest, window: i32) -> Option<bool> {
        let portable = self
            .lan
            .as_ref()?
            .menus
            .get(&g.conn)?
            .portable
            .as_ref()
            .filter(|p| p.window == window)?;
        // `Inventory.contains`: the main slots, the armour and the off hand.
        let mut held = g.inventory.iter().chain(&g.armor).chain([&g.offhand]);
        Some(held.any(|slot| {
            slot.as_ref()
                .is_some_and(|stack| portable.keep.iter().any(|id| **id == *stack.item))
        }))
    }

    /// Whether `conn`'s menu `window` is a station a mod opened with no block.
    pub(in crate::game) fn is_portable_station(&self, conn: u32, window: i32) -> bool {
        self.lan
            .as_ref()
            .and_then(|host| host.menus.get(&conn))
            .and_then(|menus| menus.portable.as_ref())
            .is_some_and(|p| p.window == window)
    }

    /// Where `conn`'s station `window` works when a mod opened it with no block: the block
    /// the player stands in now. None for every other menu.
    pub(in crate::game) fn portable_station_pos(&self, conn: u32, window: i32) -> Option<[i32; 3]> {
        if !self.is_portable_station(conn, window) {
            return None;
        }
        self.lan
            .as_ref()?
            .hub
            .guest(conn)
            .map(block_pos)
    }

    /// `conn`'s station `window` closes: forgets it as a mod's station with no block. Whether
    /// it was one.
    pub(in crate::game) fn portable_station_closed(&mut self, conn: u32, window: i32) -> bool {
        self.lan
            .as_mut()
            .and_then(|host| host.menus.get_mut(&conn))
            .and_then(|menus| menus.portable.take_if(|p| p.window == window))
            .is_some()
    }

    /// `conn`'s station `window` of type `menu` closes with `inputs` in its input slots: when a
    /// mod opened it with no block, the mods may keep stacks (`station_closing`), and the slots
    /// they emptied come back empty. Only emptying counts, so no stack is made or grown here.
    /// Whether it was such a station.
    pub(in crate::game) fn portable_station_closing(
        &mut self,
        conn: u32,
        window: i32,
        menu: &str,
        inputs: &mut [Option<ItemStack>],
    ) -> bool {
        if !self.portable_station_closed(conn, window) {
            return false;
        }
        let Some(uuid) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| g.uuid)
        else {
            return true;
        };
        let mut answer = inputs.to_vec();
        self.native_station_closing(uuid, window, menu, &mut answer);
        for (k, slot) in inputs.iter_mut().enumerate() {
            if answer.get(k).is_some_and(Option::is_none) {
                *slot = None;
            }
        }
        true
    }
}
