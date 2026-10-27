//! Guests' saved state: `players/data/<uuid>.dat` in the vanilla player
//! compound, through the same code that saves the host.
use super::guest::{Guest, SavedBody};
use crate::{
    items::ItemStack,
    save::{Level, read_dat, write_dat},
};
use horizon_protocol::crypto::uuid_string;
use std::{
    path::{Path, PathBuf},
    sync::Arc,
};

pub fn path(world: &Path, uuid: u128) -> PathBuf {
    world
        .join("players")
        .join("data")
        .join(format!("{}.dat", uuid_string(uuid)))
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

/// Load a guest's saved state, if they have been here before.
pub fn load(guest: &mut Guest, world: &Path) -> bool {
    let Some(tag) = read_dat(&path(world, guest.uuid)) else {
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

/// Give a guest the player state read into `level`.
pub fn apply(guest: &mut Guest, level: &Level) {
    if let Some(p) = level.position {
        guest.position = p;
        guest.previous_position = p;
    }
    guest.yaw = level.rotation[0];
    guest.pitch = level.rotation[1];
    guest.head_yaw = guest.yaw;
    guest.on_ground = level.on_ground;
    guest.health = level.health;
    guest.saved_absorption = Some(level.absorption);
    guest.food = level.food;
    guest.saturation = level.saturation;
    guest.xp_level = level.xp_level;
    guest.xp_progress = level.xp_progress;
    guest.xp_total = level.xp_total;
    guest.xp_seed = level.xp_seed;
    guest.effects = crate::effects::PlayerEffects::from_saved(&level.active_effects);
    guest.game_mode = level.mode();
    guest.flying = level.flying;
    guest.selected = (level.selected_slot.max(0) as usize).min(8);
    // A point that is the world spawn is none of the player's own: it
    // respawns wherever the world spawn is then (`setworldspawn`).
    guest.spawn = level
        .spawn
        .filter(|s| Some(s.map(|v| v.floor() as i32)) != level.world_spawn)
        .map(|s| [s[0] + 0.5, s[1], s[2] + 0.5]);
    guest.respawn_elsewhere = level.respawn_elsewhere.clone();
    guest.saved = level.player_extra.clone();
    guest.seen_credits = level.seen_credits;
    guest.last_death = level.last_death.clone();
    let extra = level.player_extra.as_ref();
    let short = |key: &str, default: i64| {
        extra
            .and_then(|t| crate::save::number(t.get(key)))
            .unwrap_or(default) as i32
    };
    guest.loaded_body = Some(SavedBody {
        fire_ticks: short("Fire", -20),
        air: short("Air", 300),
        fall_distance: level.fall_distance,
    });
    guest.recipe_book = level.recipe_book.clone();
    guest.dimension = match level.dimension.as_str() {
        d @ ("minecraft:the_nether" | "minecraft:the_end") => d.to_owned(),
        _ => "minecraft:overworld".to_owned(),
    };
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
            _ => {}
        }
    }
}

/// Save a guest (PlayerDataStorage.save).
pub fn save(guest: &Guest, world: &Path) {
    let mut level = Level {
        player_uuid: Some(uuid_ints(guest.uuid)),
        ..Level::default()
    };
    store(guest, &mut level);
    let file = path(world, guest.uuid);
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = write_dat(&file, level.player_tag());
}

/// Put a guest's state in `level`'s player fields.
pub fn store(guest: &Guest, level: &mut Level) {
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
    level.xp_seed = guest.xp_seed;
    level.active_effects = guest.effects.saved();
    level.game_mode = guest.game_mode;
    level.creative = guest.game_mode == 1;
    level.flying = guest.flying;
    level.selected_slot = guest.selected as i32;
    level.spawn = guest.spawn.map(|s| [s[0] - 0.5, s[1], s[2] - 0.5]);
    level.respawn_elsewhere = guest.respawn_elsewhere.clone();
    level.on_ground = guest.on_ground;
    level.seen_credits = guest.seen_credits;
    level.last_death = guest.last_death.clone();
    level.player_extra = guest.saved.clone();
    // The live body's fire, air and fall distance (Entity.saveWithoutId),
    // or what the file had while the player has no body yet.
    let body = guest
        .motion
        .as_deref()
        .map(SavedBody::of)
        .or(guest.loaded_body);
    if let Some(body) = body {
        level.fall_distance = body.fall_distance;
        let extra = level
            .player_extra
            .get_or_insert_with(|| horizon_world::nbt::Tag::Compound(Default::default()));
        if let horizon_world::nbt::Tag::Compound(map) = extra {
            let short = |v: i32| horizon_world::nbt::Tag::Short(v.clamp(-32768, 32767) as i16);
            map.insert("Fire".into(), short(body.fire_ticks));
            map.insert("Air".into(), short(body.air));
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
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lan::{Joined, PlayerSender};
    use horizon_protocol::packets::configuration::ClientInformation;
    use std::sync::{Arc, atomic::AtomicI32, mpsc};

    fn guest() -> Guest {
        let (tx, _rx) = mpsc::channel();
        Guest::new(
            Joined {
                conn: 2,
                uuid: 2,
                name: "Alex".into(),
                properties: Vec::new(),
                info: ClientInformation::new(8),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
            },
            2,
            [0.0, 64.0, 0.0],
            8,
        )
    }

    #[test]
    fn a_saved_burn_and_fall_come_back() {
        // A player saved burning in mid-fall (#1323, #1327): Entity.saveWithoutId's
        // Fire, Air and fall_distance, read back for the next body.
        let mut g = guest();
        g.loaded_body = Some(SavedBody {
            fire_ticks: 96,
            air: 250,
            fall_distance: 9.5,
        });
        let mut level = Level::default();
        store(&g, &mut level);
        let tag = level.player_tag();
        let mut read = Level::default();
        read.read_player(&tag);
        read.player_extra = Some(tag);
        let mut back = guest();
        apply(&mut back, &read);
        assert_eq!(back.loaded_body, g.loaded_body);
    }
}
