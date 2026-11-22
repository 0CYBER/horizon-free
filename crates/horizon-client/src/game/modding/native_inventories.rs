//! Another player's inventories for server mods:
//! what an operator's `/invsee` or `/endersee` needs. A player online is read
//! and changed live; a player who isn't on the server is read from, and
//! written to, their file (`players/data/<uuid>.dat`, as the server saves
//! it), never while they are logging in or out, dead or swapped in. One stack moves
//! at a time, checked against what the mod last showed, so a menu built on
//! these never copies or loses an item. Server menus are in `native_menus.rs`;
//! maps a mod draws in `player_send_map` below.
use super::{Game, api_values::TryIntoApi, commands::Target};
use horizon_client::{
    items::ItemStack,
    lan::{guest::Guest, playerdata},
    save::{BODY_SLOT, Level, SADDLE_SLOT, read_dat, write_dat_checked},
};
use horizon_core::nbt::{JavaString, Tag};
use mod_api::{
    __private::game::Fields, PlayerInventoryError as Error, PlayerInventoryKind,
    PlayerInventoryMove, PlayerInventorySnapshot, PlayerRef, TransactionLifecycle,
};
use std::path::Path;

/// Which of the two inventories to inspect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum NativeInventoryKind {
    /// The player's main inventory, armour and offhand.
    Player,
    /// The player's ender chest.
    Ender,
}

impl NativeInventoryKind {
    pub(crate) fn size(self) -> usize {
        match self {
            Self::Player => 41,
            Self::Ender => 27,
        }
    }
}

impl From<PlayerInventoryKind> for NativeInventoryKind {
    fn from(kind: PlayerInventoryKind) -> Self {
        match kind {
            PlayerInventoryKind::Player => Self::Player,
            PlayerInventoryKind::Ender => Self::Ender,
            _ => unreachable!("unsupported player inventory"),
        }
    }
}

/// The legacy inventory record kept available to declared internals users.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeInventory {
    pub slots: Vec<Option<ItemStack>>,
    pub online: bool,
}

/// `Level.inventory`'s slot numbers, including the two extra equipment slots kept in the file.
fn file_slot(i: usize) -> Option<u8> {
    match i {
        0..36 => Some(i as u8),
        36..40 => Some(100 + (i - 36) as u8),
        40 => Some(150),
        41 => Some(BODY_SLOT),
        42 => Some(SADDLE_SLOT),
        _ => None,
    }
}

fn guest_slots(g: &Guest, kind: NativeInventoryKind) -> Vec<Option<ItemStack>> {
    match kind {
        NativeInventoryKind::Player => {
            let mut s = g.inventory.clone();
            s.resize(36, None);
            // `armor` is head first.
            s.extend([3, 2, 1, 0].map(|i| g.armor[i].clone()));
            s.push(g.offhand.clone());
            s
        }
        NativeInventoryKind::Ender => {
            let mut s = g.ender_items.clone();
            s.resize(kind.size(), None);
            s
        }
    }
}

fn set_guest_slots(g: &mut Guest, kind: NativeInventoryKind, mut s: Vec<Option<ItemStack>>) {
    match kind {
        NativeInventoryKind::Player => {
            g.offhand = s.pop().flatten();
            for (k, i) in [3, 2, 1, 0].into_iter().enumerate() {
                g.armor[i] = s[36 + k].take();
            }
            s.truncate(36);
            g.inventory = s;
        }
        NativeInventoryKind::Ender => g.ender_items = s,
    }
}

fn level_inventory(level: &Level, size: usize) -> Vec<Option<ItemStack>> {
    let mut slots = vec![None; size];
    for (slot, item, count, damage) in &level.inventory {
        if let Some(i) = (0..size).find(|&i| file_slot(i) == Some(*slot)) {
            slots[i] = Some(ItemStack {
                item: item.as_str().into(),
                count: *count,
                damage: *damage,
                extra: level.inventory_components.get(slot).cloned(),
            });
        }
    }
    slots
}

fn level_slots(level: &Level, kind: NativeInventoryKind) -> Vec<Option<ItemStack>> {
    match kind {
        NativeInventoryKind::Player => level_inventory(level, kind.size()),
        NativeInventoryKind::Ender => {
            let mut s = level.ender_items.clone();
            s.resize(kind.size(), None);
            s
        }
    }
}

fn set_level_slots(level: &mut Level, kind: NativeInventoryKind, s: Vec<Option<ItemStack>>) {
    match kind {
        NativeInventoryKind::Player => {
            let edited = |slot| (0..41).any(|i| file_slot(i) == Some(slot));
            level.inventory.retain(|(slot, ..)| !edited(*slot));
            level.inventory_components.retain(|slot, _| !edited(*slot));
            for (i, stack) in s.into_iter().enumerate() {
                if let (Some(slot), Some(st)) = (file_slot(i), stack) {
                    level
                        .inventory
                        .push((slot, st.item.to_string(), st.count, st.damage));
                    if let Some(extra) = st.extra {
                        level.inventory_components.insert(slot, extra);
                    }
                }
            }
        }
        NativeInventoryKind::Ender => level.ender_items = s,
    }
}

struct SavedPlayer {
    level: Box<Level>,
    backup_only: bool,
}

/// Keep the whole player compound and remember which file supplied it.
fn read_level(world: &Path, uuid: u128) -> Option<SavedPlayer> {
    let file = playerdata::path(world, uuid);
    let read = |path: &Path| read_dat(path).filter(|tag| matches!(tag, Tag::Compound(_)));
    let (tag, backup_only) = match read(&file) {
        Some(tag) => (tag, false),
        None => (read(&file.with_extension("dat_old"))?, true),
    };
    let mut level = Box::new(Level::default());
    level.read_player(&tag);
    level.player_extra = Some(tag);
    Some(SavedPlayer { level, backup_only })
}

/// Overlay only the edited inventories onto the loaded player file.
fn saved_inventory_tag(level: &Level, edit_player: bool, edit_ender: bool) -> Tag {
    let written = level.player_tag();
    let Some(Tag::Compound(mut original)) = level.player_extra.clone() else {
        return written;
    };
    let keys = [
        ("Inventory", edit_player),
        ("equipment", edit_player),
        ("EnderItems", edit_ender),
    ];
    for (key, edited) in keys {
        if edited {
            let name = JavaString::from(key);
            if let Some(tag) = written.get(key) {
                original.insert(name, tag.clone());
            } else {
                original.remove(&name);
            }
        }
    }
    Tag::Compound(original)
}

fn write_inventories(
    file: &Path,
    level: &Level,
    edit_player: bool,
    edit_ender: bool,
) -> Result<(), Error> {
    let tag = saved_inventory_tag(level, edit_player, edit_ender);
    let written = write_dat_checked(file, tag, |planned, saved| {
        // The disk codec can change nested data without failing. Check the
        // serialized inventories, including whichever inventory we left alone.
        if ["Inventory", "equipment", "EnderItems"]
            .into_iter()
            .any(|key| planned.get(key) != saved.get(key))
        {
            return false;
        }
        let mut kept = Level::default();
        kept.read_player(saved);
        (!edit_player || slots_saved(level_inventory(level, 43), level_inventory(&kept, 43)))
            && (!edit_ender
                || slots_saved(
                    level_slots(level, NativeInventoryKind::Ender),
                    level_slots(&kept, NativeInventoryKind::Ender),
                ))
    })
    .map_err(|e| Error::File(e.to_string()))?;
    written.then_some(()).ok_or(Error::Unsaveable)
}

fn slots_saved(planned: Vec<Option<ItemStack>>, kept: Vec<Option<ItemStack>>) -> bool {
    planned
        .iter()
        .zip(&kept)
        .all(|(planned, kept)| match (planned, kept) {
            (Some(planned), Some(kept)) => planned.matches_saved(kept),
            (None, None) => true,
            _ => false,
        })
}

/// Where one side of a move lives while it is made.
enum Side {
    /// A guest's index in the hub.
    Live(usize),
    /// A player's file, read.
    File(SavedPlayer),
}

/// Another player's inventories, for server mods.
impl Game<'_> {
    /// A player's live or saved inventory for native internals callers.
    #[allow(dead_code, reason = "published native internal")]
    pub(super) fn native_inventory(
        &self,
        uuid: u128,
        kind: NativeInventoryKind,
    ) -> Result<NativeInventory, String> {
        let side = self.native_inventory_side(uuid)?;
        Ok(self.inventory_snapshot(side, kind))
    }

    #[allow(dead_code, reason = "published native internal")]
    fn native_inventory_side(&self, uuid: u128) -> Result<Side, String> {
        self.inventory_side(uuid).map_err(|error| error.to_string())
    }

    fn inventory_snapshot(&self, side: Side, kind: NativeInventoryKind) -> NativeInventory {
        let (slots, online) = match side {
            Side::Live(i) => {
                let host = self.lan.as_ref().expect("live");
                (guest_slots(&host.hub.guests[i], kind), true)
            }
            Side::File(saved) => (level_slots(&saved.level, kind), false),
        };
        NativeInventory { slots, online }
    }

    pub(super) fn server_player_inventory(
        &mut self,
        uuid: u128,
        kind: PlayerInventoryKind,
    ) -> Result<PlayerInventorySnapshot, Error> {
        self.finish_inventory_autosaves(&[uuid])?;
        let side = self.inventory_side(uuid)?;
        let NativeInventory { slots, online } = self.inventory_snapshot(side, kind.into());
        let slots = slots.try_into_api().map_err(|_| Error::InvalidStack)?;
        Ok(PlayerInventorySnapshot::new(slots, online))
    }

    fn finish_inventory_autosaves(&mut self, players: &[u128]) -> Result<(), Error> {
        let host = self.lan.as_ref().ok_or(Error::NoServer)?;
        if players
            .iter()
            .any(|&uuid| host.hub.player_logout_pending(uuid))
        {
            return Err(Error::LoggingIn);
        }
        if players
            .iter()
            .any(|&uuid| !host.hub.guests.iter().any(|g| g.uuid == uuid))
        {
            self.finish_player_autosaves(true)
                .map_err(|e| Error::File(e.to_string()))?;
        }
        Ok(())
    }

    fn inventory_side(&self, uuid: u128) -> Result<Side, Error> {
        let Some(host) = self.lan.as_ref() else {
            return Err(Error::NoServer);
        };
        if host.hub.player_logout_pending(uuid) {
            return Err(Error::LoggingIn);
        }
        if let Some(i) = host.hub.guests.iter().position(|g| g.uuid == uuid) {
            let g = &host.hub.guests[i];
            return if !g.placed {
                Err(Error::LoggingIn)
            } else if g.dead {
                Err(Error::Dead)
            } else if !matches!(
                self.player(uuid)
                    .and_then(|player| self.native_entity_target(player.entity())),
                Some(Target::Guest(_))
            ) {
                Err(Error::SwappedOut)
            } else {
                Ok(Side::Live(i))
            };
        }
        if host.hub.player_logout_save_failed(uuid) {
            return Err(Error::File("the player's last logout save failed".into()));
        }
        if self.world_root.as_os_str().is_empty() {
            return Err(Error::NoWorld);
        }
        read_level(&self.world_root, uuid)
            .map(Side::File)
            .ok_or(Error::NoSavedPlayer)
    }

    /// Moves the stack in slot `from_slot` of `from`'s inventory `from_kind`,
    /// which must still be exactly `expected`, into `to`'s inventory
    /// `to_kind` where it fits whole (onto equal stacks first, then empty
    /// slots; for `Player`, the 36 main slots, never armour or off hand), or
    /// nothing moves and the Err says why. Each side is live or its file
    /// (`server_player_inventory`); at most one may be a file, which is written
    /// before the live side changes. Their screens follow with the tick's
    /// sync.
    #[allow(dead_code, reason = "published native internal")]
    pub(super) fn native_move_stack(
        &mut self,
        from: (u128, NativeInventoryKind, usize),
        expected: &ItemStack,
        to: (u128, NativeInventoryKind),
    ) -> Result<(), String> {
        self.move_inventory_stack(from, to, |stack| Ok(stack == expected))
            .map_err(|error| error.to_string())
    }

    pub(super) fn server_move_inventory_stack(
        &mut self,
        request: &PlayerInventoryMove,
    ) -> Result<(), Error> {
        if self.transaction_lifecycle() != TransactionLifecycle::None {
            return Err(Error::Transaction);
        }
        let (from_uuid, from_kind, from_slot) = request.from;
        let (to_uuid, to_kind) = request.to;
        self.move_inventory_stack(
            (from_uuid, from_kind.into(), from_slot as usize),
            (to_uuid, to_kind.into()),
            |stack| {
                let stack = stack
                    .clone()
                    .try_into_api()
                    .map_err(|_| Error::InvalidStack)?;
                Ok(stack == request.expected)
            },
        )
    }

    fn move_inventory_stack(
        &mut self,
        from: (u128, NativeInventoryKind, usize),
        to: (u128, NativeInventoryKind),
        matches: impl FnOnce(&ItemStack) -> Result<bool, Error>,
    ) -> Result<(), Error> {
        let (from_uuid, from_kind, from_slot) = from;
        let (to_uuid, to_kind) = to;
        if from_uuid == to_uuid && from_kind == to_kind {
            return Err(Error::SameInventory);
        }
        self.finish_inventory_autosaves(&[from_uuid, to_uuid])?;
        let a = self.inventory_side(from_uuid)?;
        let b = if from_uuid == to_uuid {
            None
        } else {
            Some(self.inventory_side(to_uuid)?)
        };
        if matches!(a, Side::File(_)) && matches!(b, Some(Side::File(_))) {
            return Err(Error::BothAway);
        }
        if [&a]
            .into_iter()
            .chain(b.as_ref())
            .any(|side| matches!(side, Side::File(saved) if saved.backup_only))
        {
            return Err(Error::BackupOnly);
        }
        let max = |s: &ItemStack| s.max_stack_size(self.items());
        let read = |side: &Side, kind| match side {
            Side::Live(i) => guest_slots(&self.lan.as_ref().expect("live").hub.guests[*i], kind),
            Side::File(saved) => level_slots(&saved.level, kind),
        };
        let mut src = read(&a, from_kind);
        let mut dst = read(b.as_ref().unwrap_or(&a), to_kind);
        let source = src.get_mut(from_slot).ok_or(Error::StackChanged)?;
        let stack = source.take().ok_or(Error::StackChanged)?;
        if !matches(&stack)? {
            return Err(Error::StackChanged);
        }
        let room = match to_kind {
            NativeInventoryKind::Player => 36,
            NativeInventoryKind::Ender => 27,
        };
        if !super::native_menus::fit(&mut dst[..room], std::slice::from_ref(&stack), &max) {
            return Err(Error::NoRoom);
        }
        // The file first: when it can't be written, nothing has moved.
        let world = self.world_root.clone();
        let write =
            |side: Side, kind, slots: Vec<Option<ItemStack>>, uuid, other| -> Result<Side, Error> {
                if let Side::File(mut saved) = side {
                    set_level_slots(&mut saved.level, kind, slots);
                    write_inventories(
                        &playerdata::path(&world, uuid),
                        &saved.level,
                        kind == NativeInventoryKind::Player
                            || other == Some(NativeInventoryKind::Player),
                        kind == NativeInventoryKind::Ender
                            || other == Some(NativeInventoryKind::Ender),
                    )?;
                    return Ok(Side::File(saved));
                }
                Ok(side)
            };
        let (a, b) = match b {
            None => {
                // Two inventories of one player: their file or live.
                let mut both = a;
                if let Side::File(saved) = &mut both {
                    set_level_slots(&mut saved.level, from_kind, src.clone());
                }
                let both = write(both, to_kind, dst.clone(), from_uuid, Some(from_kind))?;
                (both, None)
            }
            Some(b) if matches!(b, Side::File(_)) => {
                let b = write(b, to_kind, dst.clone(), to_uuid, None)?;
                (a, Some(b))
            }
            Some(b) => {
                let a = write(a, from_kind, src.clone(), from_uuid, None)?;
                (a, Some(b))
            }
        };
        let host = self.lan.as_mut().expect("checked");
        if let Side::Live(i) = a {
            host.hub
                .guests
                .update(i, |g| set_guest_slots(g, from_kind, src));
            if b.is_none() {
                host.hub
                    .guests
                    .update(i, |g| set_guest_slots(g, to_kind, dst));
                return Ok(());
            }
        }
        if let Some(Side::Live(i)) = b {
            host.hub
                .guests
                .update(i, |g| set_guest_slots(g, to_kind, dst));
        }
        Ok(())
    }

    /// Sends the player `uuid` the whole picture of map `map_id`: 128 x 128
    /// colours (`MapColor` ids times 4 plus the shade, row by row), scale 0,
    /// locked, no decorations, as `ClientboundMapItemDataPacket`. A filled
    /// map the mod gives with that `minecraft:map_id` then shows it in hand
    /// and in item frames; the server keeps nothing (a map id with no saved
    /// map is never sent again by the game), so send it again after the
    /// player joins. Use ids far from the world's own (they count up from 0).
    /// False for a player not here, or `colors` not 16384 long.
    pub(super) fn player_send_map(
        &mut self,
        player: PlayerRef,
        map_id: i32,
        colors: &[u8],
    ) -> bool {
        use horizon_protocol::packets::play::{Clientbound, MapItemData, MapPatch};
        if self.player_is_fake(player) != Some(false) || colors.len() != 128 * 128 {
            return false;
        }
        self.native_send(
            player.fields().uuid,
            Clientbound::MapItemData(MapItemData {
                map_id,
                scale: 0,
                locked: true,
                decorations: None,
                patch: Some(MapPatch {
                    width: 128,
                    height: 128,
                    start_x: 0,
                    start_y: 0,
                    colors: colors.to_vec(),
                }),
            }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::{super::native_guest_turn_tests::with_guests, *};
    use horizon_client::save::write_dat;
    use std::{fs, sync::Arc};

    fn stack(item: &str, count: u32) -> ItemStack {
        ItemStack {
            item: item.into(),
            count,
            damage: 0,
            extra: None,
        }
    }

    #[test]
    fn a_files_slots_map_to_the_41_and_back() {
        let mut level = Level::default();
        let mut s = vec![None; 41];
        s[0] = Some(stack("minecraft:stone", 5));
        s[35] = Some(stack("minecraft:dirt", 1));
        s[36] = Some(stack("minecraft:iron_boots", 1));
        s[39] = Some(stack("minecraft:iron_helmet", 1));
        s[40] = Some(stack("minecraft:shield", 1));
        set_level_slots(&mut level, NativeInventoryKind::Player, s.clone());
        let slots: Vec<u8> = level.inventory.iter().map(|e| e.0).collect();
        assert_eq!(slots, [0, 35, 100, 103, 150]);
        assert_eq!(level_slots(&level, NativeInventoryKind::Player), s);
    }

    #[test]
    #[ignore = "daily"]
    fn an_offline_inventory_move_is_visible_on_login() {
        use mod_api::prelude::PlayersServerHost;

        with_guests(|game| {
            let dir = tempfile::tempdir().unwrap();
            game.world_root = dir.path().to_owned();
            let file = playerdata::path(dir.path(), 42);
            let mut level = Level {
                xp_level: 7,
                ..Level::default()
            };
            let apple = stack("minecraft:apple", 3);
            let mut ender = vec![None; 27];
            ender[5] = Some(apple.clone());
            set_level_slots(&mut level, NativeInventoryKind::Ender, ender);
            let Tag::Compound(mut original) = level.player_tag() else {
                unreachable!()
            };
            original.insert("untouched".into(), Tag::Long(123));
            write_dat(&file, Tag::Compound(original)).unwrap();

            let away =
                PlayersServerHost::server_player_inventory(game, 42, PlayerInventoryKind::Ender)
                    .unwrap();
            assert!(!away.online);
            assert_eq!(away.slots[5], Some(apple.try_into_api().unwrap()));
            let request = PlayerInventoryMove::new(
                (42, PlayerInventoryKind::Ender, 5),
                away.slots[5].clone().unwrap(),
                (12, PlayerInventoryKind::Player),
            );
            PlayersServerHost::server_move_inventory_stack(game, &request).unwrap();
            assert_eq!(
                PlayersServerHost::server_move_inventory_stack(game, &request),
                Err(Error::StackChanged)
            );
            let online =
                PlayersServerHost::server_player_inventory(game, 12, PlayerInventoryKind::Player)
                    .unwrap();
            assert!(online.online);
            assert_eq!(
                online
                    .slots
                    .iter()
                    .flatten()
                    .filter(|s| **s == request.expected)
                    .count(),
                1
            );
            let saved = read_dat(&file).unwrap();
            assert_eq!(saved.get("untouched"), Some(&Tag::Long(123)));
            assert_eq!(saved.get("XpLevel"), Some(&Tag::Int(7)));
            assert!(
                level_slots(
                    &read_level(dir.path(), 42).unwrap().level,
                    NativeInventoryKind::Ender
                )[5]
                .is_none()
            );
        });
    }

    #[test]
    #[ignore = "daily"]
    fn an_inventory_move_preserves_the_only_readable_backup() {
        with_guests(|game| {
            let dir = tempfile::tempdir().unwrap();
            game.world_root = dir.path().to_owned();
            let file = playerdata::path(dir.path(), 42);
            let backup = file.with_extension("dat_old");
            let apple = stack("minecraft:apple", 3);
            let saved = Level {
                ender_items: vec![Some(apple.clone())],
                ..Level::default()
            };
            write_dat(&backup, saved.player_tag()).unwrap();
            let backup_before = fs::read(&backup).unwrap();
            game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                g.inventory[0] = Some(apple.clone());
            });
            for corrupt_primary in [false, true] {
                if corrupt_primary {
                    write_dat(&file, Tag::Int(5)).unwrap();
                }
                let primary_before = fs::read(&file).ok();
                let snapshot = game
                    .server_player_inventory(42, PlayerInventoryKind::Ender)
                    .unwrap();
                assert!(!snapshot.online);
                assert_eq!(
                    snapshot.slots[0],
                    Some(apple.clone().try_into_api().unwrap())
                );
                assert_eq!(
                    game.move_inventory_stack(
                        (12, NativeInventoryKind::Player, 0),
                        (42, NativeInventoryKind::Ender),
                        |stack| Ok(stack == &apple),
                    ),
                    Err(Error::BackupOnly)
                );
                assert_eq!(fs::read(&backup).unwrap(), backup_before);
                assert_eq!(fs::read(&file).ok(), primary_before);
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0].inventory[0],
                    Some(apple.clone())
                );
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_live_stack_moves_to_offline_storage_with_canonical_components() {
        with_guests(|game| {
            let dir = tempfile::tempdir().unwrap();
            game.world_root = dir.path().to_owned();
            let file = playerdata::path(dir.path(), 42);
            write_dat(&file, Level::default().player_tag()).unwrap();

            let mut pick = stack("minecraft:iron_pickaxe", 1);
            let mut expected = pick.clone();
            for (key, live, saved) in [
                (
                    "minecraft:attribute_modifiers",
                    r#"[{type:"attack_damage",id:"base_attack_damage",amount:20.0d,operation:"add_value",slot:"mainhand"}]"#,
                    r#"[{type:"minecraft:attack_damage",id:"minecraft:base_attack_damage",amount:20.0d,operation:"add_value",slot:"mainhand"}]"#,
                ),
                (
                    "minecraft:can_break",
                    r#"{blocks:["minecraft:stone"]}"#,
                    r#"{blocks:"minecraft:stone"}"#,
                ),
                (
                    "minecraft:can_place_on",
                    r#"{predicates:{"minecraft:custom_data":{mark:7}}}"#,
                    r#"{predicates:{"minecraft:custom_data":"{mark:7}"}}"#,
                ),
            ] {
                let parse = |s| Some(horizon_core::nbt::parse_snbt(s).unwrap().0);
                pick.data_mut().set_other(key, parse(live));
                expected.data_mut().set_other(key, parse(saved));
            }
            game.lan.as_mut().unwrap().hub.update_guest(12, |g| {
                g.inventory[0] = Some(pick.clone());
            });
            let source = game
                .server_player_inventory(12, PlayerInventoryKind::Player)
                .unwrap();
            let request = PlayerInventoryMove::new(
                (12, PlayerInventoryKind::Player, 0),
                source.slots[0].clone().unwrap(),
                (42, PlayerInventoryKind::Ender),
            );
            game.server_move_inventory_stack(&request).unwrap();
            assert!(game.lan.as_ref().unwrap().hub.guests[0].inventory[0].is_none());
            let saved = read_level(dir.path(), 42).unwrap().level;
            assert_eq!(saved.ender_items[0], Some(expected));
        });
    }

    #[test]
    #[ignore = "daily"]
    fn lossy_live_to_offline_moves_leave_the_inventory_and_both_files_intact() {
        with_guests(|game| {
            let dir = tempfile::tempdir().unwrap();
            game.world_root = dir.path().to_owned();
            let file = playerdata::path(dir.path(), 42);
            let backup = file.with_extension("dat_old");
            write_dat(&file, Tag::compound([("XpLevel", Tag::Int(6))])).unwrap();
            write_dat(&file, Tag::compound([("XpLevel", Tag::Int(7))])).unwrap();
            let primary_before = fs::read(&file).unwrap();
            let backup_before = fs::read(&backup).unwrap();

            let mut large_string = stack("minecraft:stone", 1);
            large_string.data_mut().other.push((
                "minecraft:custom_data".into(),
                Tag::compound([(
                    "nested",
                    Tag::compound([("text", Tag::String("a".repeat(65_536).as_str().into()))]),
                )]),
            ));
            // The active item's limit allows a count the saved-item codec cannot read.
            let mut items = game.items().clone();
            items.items.get_mut("minecraft:stone").unwrap().max_stack = 128;
            game.server_items = Some(Arc::new(items));
            let mut firework = stack("minecraft:firework_rocket", 1);
            firework.data_mut().fireworks = Some((200, Vec::new()));
            let mut predicate = stack("minecraft:stone", 1);
            predicate.data_mut().set_other(
                "minecraft:can_break",
                Some(Tag::compound([(
                    "predicates",
                    Tag::compound([(
                        "minecraft:custom_data",
                        Tag::compound([("mark", Tag::String(JavaString(vec![0xd800])))]),
                    )]),
                )])),
            );
            for item in [
                large_string,
                stack("minecraft:stone", 100),
                firework,
                predicate,
            ] {
                let expected_live = {
                    let hub = &mut game.lan.as_mut().unwrap().hub;
                    hub.update_guest(12, |g| g.inventory[0] = Some(item.clone()));
                    hub.guests[0].inventory.clone()
                };
                assert_eq!(
                    game.move_inventory_stack(
                        (12, NativeInventoryKind::Player, 0),
                        (42, NativeInventoryKind::Ender),
                        |stack| Ok(stack == &item),
                    ),
                    Err(Error::Unsaveable)
                );
                assert_eq!(
                    game.lan.as_ref().unwrap().hub.guests[0].inventory,
                    expected_live
                );
                assert_eq!(fs::read(&file).unwrap(), primary_before);
                assert_eq!(fs::read(&backup).unwrap(), backup_before);
            }
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_move_respects_the_stacks_own_limit() {
        with_guests(|game| {
            let mut item = stack("minecraft:stone", 1);
            item.data_mut()
                .other
                .push(("minecraft:max_stack_size".into(), Tag::Int(1)));
            let hub = &mut game.lan.as_mut().unwrap().hub;
            for conn in [12, 13] {
                hub.update_guest(conn, |g| g.inventory[0] = Some(item.clone()));
            }
            game.move_inventory_stack(
                (12, NativeInventoryKind::Player, 0),
                (13, NativeInventoryKind::Player),
                |stack| Ok(stack == &item),
            )
            .unwrap();
            let guests = &game.lan.as_ref().unwrap().hub.guests;
            assert!(guests[0].inventory[0].is_none());
            assert_eq!(guests[1].inventory[..2], [Some(item.clone()), Some(item)]);
        });
    }

    /// A stack nested past the save depth fits the planned slots but not the
    /// file: the edit is refused before anything is written or cleared.
    #[test]
    fn a_stack_the_player_file_would_drop_refuses_the_edit() {
        let mut deep = Tag::compound([("bottom", Tag::Int(1))]);
        for _ in 0..horizon_core::nbt::Limits::saved().max_depth {
            deep = Tag::compound([("nested", deep)]);
        }
        let mut lost = ItemStack::new("minecraft:stone", 1);
        lost.data_mut()
            .other
            .push(("minecraft:custom_data".into(), deep));
        let mut level = Level::default();
        let mut ender = vec![None; 27];
        ender[0] = Some(stack("minecraft:diamond", 2));
        set_level_slots(&mut level, NativeInventoryKind::Ender, ender.clone());
        let dir = tempfile::tempdir().unwrap();
        let file = playerdata::path(dir.path(), 42);
        write_inventories(&file, &level, false, true).unwrap();
        ender[3] = Some(lost);
        set_level_slots(&mut level, NativeInventoryKind::Ender, ender);
        assert_eq!(
            write_inventories(&file, &level, false, true),
            Err(Error::Unsaveable)
        );
    }

    #[test]
    fn a_saved_player_file_round_trips_its_inventory_and_keeps_the_rest() {
        let dir = std::env::temp_dir().join(format!("native-inv-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let uuid = 0x1234_u128;
        let file = playerdata::path(&dir, uuid);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut level = Level {
            health: 13.0,
            xp_level: 7,
            ..Level::default()
        };
        level.ender_items = vec![None; 27];
        level.ender_items[2] = Some(stack("minecraft:diamond", 3));
        let mut s = vec![None; 41];
        s[4] = Some(stack("minecraft:bread", 12));
        set_level_slots(&mut level, NativeInventoryKind::Player, s);
        write_dat(&file, level.player_tag()).unwrap();
        let mut back = read_level(&dir, uuid).unwrap().level;
        assert_eq!(back.health, 13.0);
        assert_eq!(back.xp_level, 7);
        assert_eq!(
            level_slots(&back, NativeInventoryKind::Ender)[2],
            Some(stack("minecraft:diamond", 3))
        );
        let mut inv = level_slots(&back, NativeInventoryKind::Player);
        assert_eq!(inv[4], Some(stack("minecraft:bread", 12)));
        inv[4] = None;
        let mut named = stack("minecraft:stone", 2);
        named.data_mut().custom_name = Some(Tag::compound([("text", Tag::String("kept".into()))]));
        inv[5] = Some(named);
        set_level_slots(&mut back, NativeInventoryKind::Player, inv);
        write_inventories(&file, &back, true, false).unwrap();
        let again = read_level(&dir, uuid).unwrap().level;
        let mut slots = level_slots(&again, NativeInventoryKind::Player);
        let named = slots[5].take().unwrap();
        assert_eq!(named.count, 2);
        assert_eq!(
            named.data().unwrap().custom_name,
            Some(Tag::String("kept".into()))
        );
        assert!(slots.iter().all(Option::is_none));
        assert_eq!(again.xp_level, 7);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
