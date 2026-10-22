//! Guests' saved state: `players/data/<uuid>.dat` in the vanilla player
//! compound, through the same code that saves the host.
use super::guest::Guest;
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
    guest.spawn = level.spawn.map(|s| [s[0] + 0.5, s[1], s[2] + 0.5]);
    guest.saved = level.player_extra.clone();
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
    level.on_ground = guest.on_ground;
    level.player_extra = guest.saved.clone();
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
