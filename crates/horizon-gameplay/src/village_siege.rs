//! `VillageSiege`: at midnight one night in ten, 20 zombies spawn two ticks
//! apart near a village 32 blocks from a player standing in it.
use crate::custom_spawner::{CustomSpawnerLevel, section, sections_to_village};
use horizon_client::mobs::{Species, raider, spawn_rules};
use horizon_core::{geometry::Vec3, random::RandomSource};
use std::collections::HashSet;

/// `VillageSiege`: whether tonight is a siege, whether the spot is chosen,
/// and the zombies still to spawn there.
#[derive(Default)]
pub struct VillageSiege {
    tonight: bool,
    set_up: bool,
    zombies_to_spawn: i32,
    next_spawn_time: i32,
    spot: [i32; 3],
}

impl VillageSiege {
    /// `VillageSiege.tick`.
    pub fn tick(&mut self, l: &mut impl CustomSpawnerLevel) {
        if !l.overworld() {
            return;
        }
        if l.sky_darken() < 4 || !l.spawn_mobs_and_monsters() {
            self.tonight = false;
            self.set_up = false;
            return;
        }
        // ClockTimeMarkers.ROLL_VILLAGE_SIEGE: tick 18000 of the 24000 tick day.
        let world = l.parts().world;
        if world.day_time.rem_euclid(24000) == 18000 {
            self.tonight = world.random.next_int_bound(10).unwrap_or(1) == 0;
        }
        if !self.tonight {
            return;
        }
        if !self.set_up {
            if !self.try_to_setup(l) {
                return;
            }
            self.set_up = true;
        }
        if self.next_spawn_time > 0 {
            self.next_spawn_time -= 1;
            return;
        }
        self.next_spawn_time = 2;
        if self.zombies_to_spawn > 0 {
            self.try_spawn(l);
            self.zombies_to_spawn -= 1;
        } else {
            self.tonight = false;
        }
    }

    /// `VillageSiege.tryToSetupSiege`: the first player standing in a village
    /// picks a spot 32 blocks away (ten tries); true when one stood in a
    /// village, even if no spot was found.
    fn try_to_setup(&mut self, l: &mut impl CustomSpawnerLevel) -> bool {
        let centers = l.village_centers();
        for player in l.players() {
            if player.spectator {
                continue;
            }
            let center = block_of(player.position);
            if !is_village(&centers, center)
                || l.biome_tag("minecraft:without_zombie_sieges", center)
            {
                continue;
            }
            for _ in 0..10 {
                let angle = l.parts().world.random.next_float() * std::f32::consts::TAU;
                let spot = [
                    center[0] + (angle.cos() * 32.0).floor() as i32,
                    center[1],
                    center[2] + (angle.sin() * 32.0).floor() as i32,
                ];
                if let Some(attempted_pos) = find_spawn_pos(l, &centers, spot) {
                    if l.village_siege_cancelled(player.uuid, attempted_pos) {
                        return false;
                    }
                    self.spot = spot;
                    self.next_spawn_time = 0;
                    self.zombies_to_spawn = 20;
                    break;
                }
            }
            return true;
        }
        false
    }

    /// `VillageSiege.trySpawn`: one zombie, in a random yaw.
    fn try_spawn(&mut self, l: &mut impl CustomSpawnerLevel) {
        let centers = l.village_centers();
        let Some(at) = find_spawn_pos(l, &centers, self.spot) else {
            return;
        };
        let Some(id) = l.spawn_event_mob("minecraft:zombie", at, false, raider::Reason::Event)
        else {
            return;
        };
        let p = l.parts();
        let yaw = p.world.random.next_float() * 360.0;
        if let Some(m) = p.mobs.iter_mut().find(|m| m.id == id) {
            (m.yaw, m.previous_yaw, m.body_yaw, m.previous_body_yaw) = (yaw, yaw, yaw, yaw);
            (m.head_yaw, m.previous_head_yaw) = (yaw, yaw);
        }
    }
}

/// `ServerLevel.isVillage`: within one section of a village.
pub fn is_village(centers: &HashSet<(i32, i32, i32)>, pos: [i32; 3]) -> bool {
    sections_to_village(centers, section(pos)) <= 1
}

/// `VillageSiege.findRandomSpawnPos`: ten tries within 8 blocks of the
/// spot for a village column where a zombie may spawn.
fn find_spawn_pos(
    l: &mut impl CustomSpawnerLevel,
    centers: &HashSet<(i32, i32, i32)>,
    spot: [i32; 3],
) -> Option<[f64; 3]> {
    for _ in 0..10 {
        let world = l.parts().world;
        let x = spot[0] + world.random.next_int_bound(16).unwrap_or(0) - 8;
        let z = spot[2] + world.random.next_int_bound(16).unwrap_or(0) - 8;
        if !world.chunks.contains_key(&(x >> 4, z >> 4)) {
            continue;
        }
        let y = l.surface_height(x, z);
        if is_village(centers, [x, y, z])
            && l.check_spawn_rules(
                "minecraft:zombie",
                Species::Zombie,
                spawn_rules::Reason::Event,
                [x, y, z],
            )
        {
            return Some([f64::from(x) + 0.5, f64::from(y), f64::from(z) + 0.5]);
        }
    }
    None
}

fn block_of(p: Vec3) -> [i32; 3] {
    [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32]
}
