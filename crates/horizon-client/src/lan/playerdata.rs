//! Guests' saved state: `players/data/<uuid>.dat` in the vanilla player
//! compound, through the same code that saves the host.
use super::guest::{Guest, SavedBody};
use crate::{
    items::ItemStack,
    save::{Level, put, read_dat, uuid_from_tag, uuid_tag, write_dat},
};
use horizon_protocol::crypto::uuid_string;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};
use {
    horizon_core::nbt::Tag,
    horizon_mod_runtime::attachments::{ATTACHMENTS_KEY, attachment_compound},
};

/// `PlayerDataStorage`'s player directory (`players/data`), created on first use
/// as its constructor does with `mkdirs`.
pub fn directory(world: &Path) -> PathBuf {
    let dir = directory_path(world);
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("Horizon: {} is not created: {e}", dir.display());
    }
    dir
}

fn directory_path(world: &Path) -> PathBuf {
    world.join("players").join("data")
}

pub fn path(world: &Path, uuid: u128) -> PathBuf {
    directory_path(world).join(format!("{}.dat", uuid_string(uuid)))
}

/// `UUIDUtil.uuidToIntArray`.
pub fn uuid_ints(uuid: u128) -> [i32; 4] {
    [
        (uuid >> 96) as i32,
        (uuid >> 64) as i32,
        (uuid >> 32) as i32,
        uuid as i32,
    ]
}
/// `UUIDUtil.uuidFromIntArray`.
pub fn ints_uuid(ints: [i32; 4]) -> u128 {
    ints.iter()
        .fold(0, |acc, &i| acc << 32 | u128::from(i as u32))
}

/// Whether a saved file exists for this player.
pub fn exists(world: &Path, uuid: u128) -> bool {
    path(world, uuid).is_file()
}

/// The key `ServerPlayer.addAdditionalSaveData` stores the `WardenSpawnTracker` under.
pub const WARDEN_TRACKER_KEY: &str = "warden_spawn_tracker";

/// Put a player's `warden_spawn_tracker` into the compound `store` writes from
/// (`guest.saved`), so a save keeps it rather than the older one loaded with
/// the file (the LAN owner's tracker lives in `Game`, `save_warden_tracker`).
/// Other saved keys are left as they are; no tracker (not loaded yet) changes
/// nothing.
pub fn with_warden(guest: &mut Guest, tracker: Option<Tag>) -> &mut Guest {
    let Some(tracker) = tracker else {
        return guest;
    };
    let saved = guest
        .saved
        .get_or_insert_with(|| Tag::Compound(Default::default()));
    if let Tag::Compound(map) = saved {
        map.insert(WARDEN_TRACKER_KEY.into(), tracker);
    }
    guest
}

/// `PlayerDataStorage.load`: a guest's saved state, if they have been here
/// before. The player's file, else its `.dat_old` (what a save cut off
/// between its two renames leaves, `write_dat`).
pub fn load(guest: &mut Guest, world: &Path) -> bool {
    let file = path(world, guest.uuid);
    let Some(tag) = read_dat(&file).or_else(|| read_dat(&file.with_extension("dat_old"))) else {
        return false;
    };
    let mut level = Level {
        health: 20.0,
        food: 20,
        saturation: 5.0,
        ..Level::default()
    };
    level.read_player(&tag);
    level.player_extra = Some(tag);
    apply(guest, &level);
    true
}

/// `PlayerList.loadPlayerData` for the world's single-player owner: the file
/// level.dat names (`singleplayer_uuid`), as the world opening read it into
/// `saved`, before the owner's own file. Builds that save the owner to the
/// named file alone (0.2.93) leave the own file behind, stale (#5351). The
/// own file is for a world whose named file is missing; then the player the
/// world opening found instead (the newest player file, the legacy
/// `Data.Player`, or nothing in a new world). Returns whether the player has
/// been in this world before.
pub fn load_owner(guest: &mut Guest, world: &Path, saved: Option<&Level>) -> bool {
    if let Some(level) = saved.filter(|l| l.singleplayer_file) {
        apply(guest, level);
        return level.position.is_some();
    }
    if load(guest, world) {
        return true;
    }
    saved.is_some_and(|level| {
        apply(guest, level);
        level.position.is_some()
    })
}

/// Save the world's single-player owner: its state into `level` (`store`),
/// then to the file level.dat names, which it loads ([`load_owner`]), and to
/// its own file, so that the one it loads is never the older of the two.
pub fn save_owner(guest: &Guest, level: &mut Level, world: &Path) -> bool {
    // `MinecraftServer.saveAllChunks`: `saveDataTag(worldData, getSingleplayerProfile().id())`
    // — the server's own profile on every save, never the stale `singleplayer_uuid` a
    // world arrived with (a Realms download, a server world). The player file written is
    // then the owner's own, not another player's the world happened to name.
    level.player_uuid = Some(uuid_ints(guest.uuid));
    store(guest, level, None);
    // `saveDataTag`: level.dat carries the profile the player file is named after, so the
    // file the owner loads next is the one just written.
    if let Err(e) = level.write(world) {
        log::warn!("Horizon: the owner's playerdata is not saved: {e}");
    }
    save(guest, world)
}

/// Move a world that older Horizon builds named by the offline profile to the account's
/// (`IntegratedServer.setSingleplayerProfile`): `players/data/<offline uuid>.dat` becomes
/// `players/data/<account uuid>.dat` — the newer of the two when both exist — with its
/// `UUID` rewritten, and level.dat's `singleplayer_uuid` follows. Entity `Owner`
/// references are migrated as their chunks load (`entity_nbt::migrate_player_uuid`).
///
/// `old` is `offline_uuid(player name)`, `new` the account's UUID. False when the world
/// is not this owner's (another account's world, a world made under another name): its
/// files and level data are left alone.
pub fn migrate_owner(world: &Path, level: &mut Level, old: u128, new: u128) -> bool {
    if old == new {
        return false;
    }
    let (old_ints, new_ints) = (uuid_ints(old), uuid_ints(new));
    let old_file = path(world, old);
    let old_old = old_file.with_extension("dat_old");
    let new_file = path(world, new);
    // `PlayerDataStorage.load`: the owner's file, else its `.dat_old` (what a save cut
    // off between its two renames leaves).
    let old_data = read_dat(&old_file).or_else(|| read_dat(&old_old));
    // Only this owner's world: level.dat names the offline profile (its `singleplayer_uuid`
    // or the file's own `UUID`), or a world without an owner yet (a server world, a Realms
    // download) holds the owner's player file by that name. Another account's world names
    // neither and is left alone.
    let owns_named_file = level.player_uuid == Some(old_ints);
    let ownerless = level.player_uuid.is_none() && old_data.is_some();
    if !owns_named_file && !ownerless {
        return false;
    }
    let new_data = read_dat(&new_file);
    // Both files: the one written more recently is the player's data. When their
    // times cannot be compared, the file level.dat names (the old one) wins.
    let prefer_new = match (&old_data, &new_data) {
        (Some(_), Some(_)) => {
            let modified = |path: &Path| std::fs::metadata(path).and_then(|m| m.modified()).ok();
            matches!(
                (modified(&old_file), modified(&new_file)),
                (Some(old), Some(new)) if new > old
            )
        }
        (Some(_), None) => false,
        (None, Some(_)) => true,
        (None, None) => false,
    };
    let source = if prefer_new { new_data } else { old_data };
    // No player data anywhere: nothing to move, and the owner's first save names the new
    // file. (`level.dat`'s old `singleplayer_uuid` then names no data, as before.)
    if source.is_none() && level.player_extra.is_none() {
        return false;
    }
    if let Some(mut tag) = source {
        // The player file is named after its `UUID` (`PlayerDataStorage.save`).
        put(&mut tag, "UUID", uuid_tag(new_ints));
        if let Err(e) = write_dat(&new_file, tag.clone()) {
            log::warn!(
                "Horizon: cannot move the owner's playerdata to {}: {e}",
                new_file.display()
            );
            return false;
        }
        // The player this open gives the account is the file that won, not the older one
        // `Level::read` loaded from `singleplayer_uuid`.
        level.read_player(&tag);
        level.player_extra = Some(tag);
        level.singleplayer_file = true;
        // The same data is under the account's name now. The offline files are kept
        // beside it as `.dat_offline` / `.dat_old_offline` rather than deleted, so a
        // migration that went wrong never costs the player their inventory.
        let _ = std::fs::rename(&old_file, old_file.with_extension("dat_offline"));
        let _ = std::fs::rename(&old_old, old_file.with_extension("dat_old_offline"));
    }
    level.player_uuid = Some(new_ints);
    // `saveDataTag`: `singleplayer_uuid` is the profile the world's player file is named
    // after. Written before the account's first session saves, so an interrupted first
    // open still finds the file written above.
    if let Err(e) = level.write(world) {
        log::warn!("Horizon: cannot save the migrated owner: {e}");
        return false;
    }
    true
}

/// A player new to the world faces the world spawn's angle (`PrepareSpawnTask`:
/// the saved rotation, else `respawnData.yaw/pitch`).
pub fn face_world_spawn(guest: &mut Guest, known: bool, [yaw, pitch]: [f32; 2]) {
    if known {
        return;
    }
    guest.yaw = yaw;
    guest.pitch = pitch;
    guest.head_yaw = yaw;
}

/// Give a guest the player state read into `level`.
pub fn apply(guest: &mut Guest, level: &Level) {
    if let Some(p) = level.position {
        guest.place(p);
    }
    guest.yaw = level.rotation[0];
    guest.pitch = level.rotation[1];
    guest.head_yaw = guest.yaw;
    guest.on_ground = level.on_ground;
    // LivingEntity.readAdditionalSaveData: a saved FallFlying sets shared
    // flag 7, so a player saved mid-glide (#11193) lands gliding; its next
    // `updateFallFlying` ends the glide if it can't go on.
    guest.fall_flying = level.fall_flying;
    guest.health = level.health;
    guest.saved_absorption = Some(level.absorption);
    guest.food = level.food;
    guest.saturation = level.saturation;
    guest.xp_level = level.xp_level;
    guest.xp_progress = level.xp_progress;
    guest.xp_total = level.xp_total;
    guest.score = level.score;
    guest.xp_seed = level.xp_seed;
    guest.effects = crate::effects::PlayerEffects::from_saved(&level.active_effects);
    guest.game_mode = level.mode();
    guest.previous_game_mode = level.previous_game_mode;
    // `Player.readAdditionalSaveData`, then `ServerPlayerGameMode.setGameModeForPlayer`
    // (`GameType.updatePlayerAbilities`) as the player is placed: the mode's flags
    // win, flying (creative) and the speeds stay as saved.
    guest.abilities = level
        .abilities
        .unwrap_or_else(|| crate::lan::abilities::PlayerAbilities {
            flying: level.flying,
            flying_speed: level.fly_speed.filter(|s| s.is_finite()).unwrap_or(0.05),
            ..Default::default()
        });
    guest.abilities.update_for_mode(guest.game_mode);
    guest.selected = (level.selected_slot.max(0) as usize).min(8);
    // A point that is the world spawn is none of the player's own: it
    // respawns wherever the world spawn is then (`setworldspawn`).
    guest.spawn = level
        .spawn
        .filter(|s| Some(s.map(|v| v.floor() as i32)) != level.world_spawn)
        .map(|s| [s[0] + 0.5, s[1], s[2] + 0.5]);
    guest.respawn_elsewhere = level.respawn_elsewhere.clone();
    guest.saved = level.player_extra.clone();
    let attachments = match attachment_compound(
        guest
            .saved
            .as_ref()
            .and_then(|tag| tag.get(ATTACHMENTS_KEY)),
    ) {
        Ok(Some(data)) => horizon_mod_runtime::attachments::Attachments::from_nbt(data.clone()),
        _ => Default::default(),
    };
    guest.attachments.replace_contents(attachments);
    guest.seen_credits = level.seen_credits;
    guest.post_effects.clone_from(&level.post_effects);
    guest.last_death = level.last_death.clone();
    let extra = level.player_extra.as_ref();
    let short = |key: &str, default: i64| {
        extra
            .and_then(|t| crate::save::number(t.get(key)))
            .unwrap_or(default) as i32
    };
    // Entity.load: a component of Motion over 10 blocks a tick reads as 0.
    let motion = level.motion.map(|v| if v.abs() > 10.0 { 0.0 } else { v });
    guest.loaded_body = Some(SavedBody {
        // Entity.load: `getShortOr("Fire", 0)` narrows; Air and TicksFrozen are ints.
        fire_ticks: i32::from(short("Fire", -20) as i16),
        air: short("Air", 300),
        fall_distance: level.fall_distance,
        velocity: motion,
        food_exhaustion: level.food_exhaustion,
        food_tick_timer: level.food_tick_timer,
        ticks_frozen: short("TicksFrozen", 0),
        // The `Invulnerable` flag and its window, as the host's own load
        // reads them (arismoko/horizon#916).
        invulnerable: extra
            .and_then(|t| crate::save::number(t.get("Invulnerable")))
            .is_some_and(|v| v != 0),
        invulnerable_time: short("invulnerable_time", 0),
    });
    guest.saved_vehicle = level.root_vehicle;
    guest.recipe_book = level.recipe_book.clone();
    guest.dimension = level.dimension.clone();
    if level.ender_items.len() == 27 {
        guest.ender_items = level.ender_items.clone();
    }
    for (slot, item, count, damage) in &level.inventory {
        let mut stack = ItemStack {
            item: Arc::from(item.as_str()),
            count: *count,
            damage: *damage,
            extra: level.inventory_components.get(slot).cloned(),
        };
        if stack.count == 0 {
            stack.count = 1;
        }
        match *slot {
            0..=35 => guest.inventory[usize::from(*slot)] = Some(stack),
            100 => guest.armor[3] = Some(stack),
            101 => guest.armor[2] = Some(stack),
            102 => guest.armor[1] = Some(stack),
            103 => guest.armor[0] = Some(stack),
            150 => guest.offhand = Some(stack),
            _ => {
                if let Some(slot) = crate::save::equipment_slot(*slot) {
                    guest.equipment.set(slot, Some(stack));
                }
            }
        }
    }
}

/// The shared player compound for logout and autosave snapshots, and for
/// `player-export`'s record (`arrivals::export_record`).
pub fn player_tag(guest: &Guest) -> Tag {
    let mut level = Level {
        player_uuid: Some(uuid_ints(guest.uuid)),
        ..Level::default()
    };
    store(guest, &mut level, None);
    level.player_tag()
}

/// Save a leaving guest, including its saved vehicle tree. True if its file was written.
pub fn save(guest: &Guest, world: &Path) -> bool {
    let file = path(world, guest.uuid);
    match write_dat(&file, player_tag(guest)) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("Horizon: {} is not saved: {e}", file.display());
            false
        }
    }
}

/// [`save`], synced to the disk and saying whether the file was written: a hand-over's commit
/// point (`player-confirm`) answers only once it is.
pub fn save_now(guest: &Guest, world: &Path) -> Result<(), String> {
    let file = path(world, guest.uuid);
    crate::save::write_dat_durable(&file, player_tag(guest))
        .map_err(|e| format!("{} is not saved: {e}", file.display()))
}

/// An immutable player snapshot. Compression and all filesystem work belong to
/// the writer; native callbacks are delivered by the server after completion.
struct Snapshot {
    uuid: u128,
    name: String,
    file: PathBuf,
    tag: Tag,
}

pub struct SavedPlayer {
    pub uuid: u128,
    pub name: String,
}

fn write_snapshot(snapshot: Snapshot) -> Result<SavedPlayer, String> {
    write_dat(&snapshot.file, snapshot.tag)
        .map(|()| SavedPlayer {
            uuid: snapshot.uuid,
            name: snapshot.name,
        })
        .map_err(|e| format!("{} is not saved: {e}", snapshot.file.display()))
}

struct PlayerWriter {
    send: std::sync::mpsc::Sender<Snapshot>,
    done: std::sync::mpsc::Receiver<Result<SavedPlayer, String>>,
    thread: std::thread::JoinHandle<()>,
}

/// One FIFO writer for a server's player autosaves. Synchronous logout writes
/// must follow a barrier, so an older autosave can never replace them later.
#[derive(Default)]
pub struct Autosaves {
    writer: Option<PlayerWriter>,
    pending: usize,
    error: Option<String>,
    ready: Vec<SavedPlayer>,
}

impl Autosaves {
    pub fn enqueue(&mut self, guest: &Guest, world: &Path) {
        let snapshot = Snapshot {
            uuid: guest.uuid,
            name: guest.name.clone(),
            file: path(world, guest.uuid),
            tag: player_tag(guest),
        };
        if self.writer.is_none() {
            let (send, receive) = std::sync::mpsc::channel::<Snapshot>();
            let (complete, done) = std::sync::mpsc::channel();
            match std::thread::Builder::new()
                .name("horizon-player-writer".into())
                .spawn(move || {
                    for snapshot in receive {
                        let _ = complete.send(write_snapshot(snapshot));
                    }
                }) {
                Ok(thread) => self.writer = Some(PlayerWriter { send, done, thread }),
                Err(error) => {
                    log::warn!("Horizon: cannot start player writer, saving inline: {error}")
                }
            }
        }
        let snapshot = if let Some(writer) = &self.writer {
            match writer.send.send(snapshot) {
                Ok(()) => {
                    self.pending += 1;
                    return;
                }
                Err(error) => {
                    let completed = self.completed(true);
                    self.ready.extend(completed);
                    if let Some(writer) = self.writer.take() {
                        drop(writer.send);
                        let _ = writer.thread.join();
                    }
                    error.0
                }
            }
        } else {
            snapshot
        };
        match write_snapshot(snapshot) {
            Ok(player) => self.ready.push(player),
            Err(error) => {
                log::warn!("Horizon: {error}");
                self.error = Some(error);
            }
        }
    }

    /// Serialize a newer logout snapshot after every queued autosave.
    /// Returns the completed autosaves and whether the logout snapshot was written.
    pub fn save_after(&mut self, guest: &Guest, world: &Path) -> (Vec<SavedPlayer>, bool) {
        let saved = self.completed(true);
        (saved, save(guest, world))
    }

    /// Drain completions on the tick thread; `wait` is the durability barrier
    /// for explicit saves, logout, and shutdown. Failures remain visible.
    pub fn completed(&mut self, wait: bool) -> Vec<SavedPlayer> {
        let mut saved = std::mem::take(&mut self.ready);
        while self.pending > 0 {
            let done = &self.writer.as_ref().unwrap().done;
            let result = if wait {
                done.recv()
                    .map_err(|_| std::sync::mpsc::TryRecvError::Disconnected)
            } else {
                done.try_recv()
            };
            match result {
                Ok(result) => {
                    self.pending -= 1;
                    match result {
                        Ok(player) => saved.push(player),
                        Err(error) => {
                            log::warn!("Horizon: {error}");
                            self.error = Some(error);
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.error = Some("the player writer stopped".into());
                    log::warn!("Horizon: the player writer stopped");
                    self.pending = 0;
                    if let Some(writer) = self.writer.take() {
                        drop(writer.send);
                        let _ = writer.thread.join();
                    }
                }
            }
        }
        saved
    }

    pub fn writes_finished(&mut self) -> crate::Result<bool> {
        match self.error.take() {
            Some(error) => Err(error.into()),
            None => Ok(self.pending == 0),
        }
    }
}

impl Drop for Autosaves {
    fn drop(&mut self) {
        if let Some(writer) = self.writer.take() {
            drop(writer.send);
            let _ = writer.thread.join();
        }
    }
}

/// Put a guest's state in `level`'s player fields; `vehicle` is the
/// RootVehicle compound of the ride (`ServerPlayer.saveParentVehicle`).
pub fn store(guest: &Guest, level: &mut Level, vehicle: Option<Tag>) {
    level.player_impulse_grace = guest.motion.as_ref().map(|p| p.impulse.reset_grace_time);
    level.player_fall_particles = guest
        .motion
        .as_ref()
        .map(|p| p.spawn_extra_particles_on_fall);
    level.position = Some(guest.position);
    level.dimension = guest.dimension.clone();
    level.rotation = [guest.yaw, guest.pitch];
    level.health = guest.health;
    level.absorption = guest
        .motion
        .as_ref()
        .map_or(guest.saved_absorption.unwrap_or(0.0), |m| {
            m.health.absorption()
        });
    level.food = guest.food;
    level.saturation = guest.saturation;
    level.xp_level = guest.xp_level;
    level.xp_progress = guest.xp_progress;
    level.xp_total = guest.xp_total;
    level.score = guest.score;
    level.xp_seed = guest.xp_seed;
    level.active_effects = guest.effects.saved();
    level.game_mode = guest.game_mode;
    // ServerPlayer.addAdditionalSaveData: previousPlayerGameType, when there
    // is one (storeNullable).
    level.previous_game_mode = guest.previous_game_mode;
    level.creative = guest.game_mode == 1;
    level.flying = guest.abilities.flying;
    level.fly_speed = Some(guest.abilities.flying_speed);
    level.abilities = Some(guest.abilities);
    level.selected_slot = guest.selected as i32;
    level.spawn = guest.spawn.map(|s| [s[0] - 0.5, s[1], s[2] - 0.5]);
    level.respawn_elsewhere = guest.respawn_elsewhere.clone();
    level.on_ground = guest.on_ground;
    // LivingEntity.addAdditionalSaveData: FallFlying is isFallFlying().
    level.fall_flying = guest.fall_flying;
    level.seen_credits = guest.seen_credits;
    level.post_effects.clone_from(&guest.post_effects);
    level.last_death = guest.last_death.clone();
    level.player_extra = guest.saved.clone();
    let extra = level
        .player_extra
        .get_or_insert_with(|| Tag::Compound(Default::default()));
    if let Tag::Compound(map) = extra
        && attachment_compound(map.get(&ATTACHMENTS_KEY.into())).is_ok()
    {
        let data = guest.attachments.to_nbt();
        if data.is_empty() {
            map.remove(&ATTACHMENTS_KEY.into());
        } else {
            map.insert(ATTACHMENTS_KEY.into(), Tag::Compound(data));
        }
    }
    // The vehicle the player rides (`ServerPlayer.saveParentVehicle`), or
    // the one its file names that has not come back yet (kept in `saved`).
    level.root_vehicle = guest.saved_vehicle;
    if let Some(vehicle) = vehicle {
        level.root_vehicle = uuid_from_tag(vehicle.get("Attach"));
        let extra = level
            .player_extra
            .get_or_insert_with(|| Tag::Compound(Default::default()));
        put(extra, "RootVehicle", vehicle);
    }
    // The live body's fire, air, fall distance and motion
    // (Entity.saveWithoutId), or what the file had while the player has no
    // body yet.
    let body = guest
        .motion
        .as_deref()
        .map(SavedBody::of)
        .or(guest.loaded_body);
    if let Some(body) = body {
        level.fall_distance = body.fall_distance;
        level.motion = body.velocity;
        // FoodData.addAdditionalSaveData: the exhaustion and the timer that
        // regenerates or starves the player go with the food level.
        level.food_exhaustion = body.food_exhaustion;
        level.food_tick_timer = body.food_tick_timer;
        let extra = level
            .player_extra
            .get_or_insert_with(|| horizon_core::nbt::Tag::Compound(Default::default()));
        if let horizon_core::nbt::Tag::Compound(map) = extra {
            let short = |v: i32| horizon_core::nbt::Tag::Short(v.clamp(-32768, 32767) as i16);
            map.insert("Fire".into(), short(body.fire_ticks));
            map.insert("Air".into(), short(body.air));
            // Entity.saveWithoutId writes TicksFrozen only above 0, so a
            // count from the file the player thawed since does not come back.
            if body.ticks_frozen > 0 {
                map.insert(
                    "TicksFrozen".into(),
                    horizon_core::nbt::Tag::Int(body.ticks_frozen),
                );
            } else {
                map.remove(&"TicksFrozen".into());
            }
            // The `Invulnerable` flag always, its `invulnerable_time` window
            // only while open, as the host's own save keeps them (arismoko/horizon#916).
            map.insert(
                "Invulnerable".into(),
                horizon_core::nbt::Tag::Byte(i8::from(body.invulnerable)),
            );
            if body.invulnerable_time > 0 {
                map.insert(
                    "invulnerable_time".into(),
                    horizon_core::nbt::Tag::Int(body.invulnerable_time),
                );
            } else {
                map.remove(&"invulnerable_time".into());
            }
        }
    }
    // LivingEntity.addAdditionalSaveData: the live body's AttributeMap.pack,
    // or the list the file had while the player has no body yet.
    // A player between two bodies (a respawn, a change of level) saves what
    // the next one takes over.
    let attributes = guest
        .motion
        .as_deref()
        .map(crate::motion::PlayerMotion::saved_attributes)
        .or_else(|| guest.carried_attributes.clone());
    if let Some(attributes) = attributes {
        let extra = level
            .player_extra
            .get_or_insert_with(|| horizon_core::nbt::Tag::Compound(Default::default()));
        if let horizon_core::nbt::Tag::Compound(map) = extra {
            map.insert("attributes".into(), attributes);
        }
    }
    level.recipe_book = guest.recipe_book.clone();
    level.ender_items = guest.ender_items.clone();
    level.inventory.clear();
    level.inventory_components.clear();
    let mut put = |slot: u8, stack: &Option<ItemStack>| {
        if let Some(s) = stack {
            level
                .inventory
                .push((slot, s.item.to_string(), s.count, s.damage));
            if let Some(extra) = &s.extra {
                level.inventory_components.insert(slot, extra.clone());
            }
        }
    };
    for (i, s) in guest.inventory.iter().enumerate() {
        put(i as u8, s);
    }
    put(103, &guest.armor[0]);
    put(102, &guest.armor[1]);
    put(101, &guest.armor[2]);
    put(100, &guest.armor[3]);
    put(150, &guest.offhand);
    for slot in [
        horizon_items::equipment::BODY,
        horizon_items::equipment::SADDLE,
    ] {
        if let Some(saved) = crate::save::saved_slot(slot) {
            put(saved, &guest.equipment[slot]);
        }
    }
}

#[cfg(test)]
mod tests;
