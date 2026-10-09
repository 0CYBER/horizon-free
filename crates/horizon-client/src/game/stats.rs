//! Player statistics (`ServerStatsCounter`): the custom counters, blocks
//! mined, items used/crafted/broken/picked up/dropped and mobs killed or
//! killed by, saved to `players/stats/<uuid>.json`, plus the phantom
//! spawner that reads the time since rest (`PhantomSpawner`).
use super::{Dim, Game};
use horizon_core::random::RandomSource;
use std::collections::BTreeMap;

/// `stats[type][key]`, with `type` and `key` namespaced ids.
#[derive(Default)]
pub(super) struct Stats {
    pub(super) values: BTreeMap<String, BTreeMap<String, i32>>,
    loaded: bool,
    /// Ticks until the phantom spawner runs again.
    phantom_tick: i32,
    /// Statistic changes (type, key, amount; `None` = reset) for the
    /// scoreboard criteria to follow.
    pub(super) events: Vec<(String, String, Option<i32>)>,
}
impl Stats {
    pub(super) fn add(&mut self, ty: &str, key: &str, n: i32) {
        let v = self
            .values
            .entry(format!("minecraft:{ty}"))
            .or_default()
            .entry(key.to_owned())
            .or_insert(0);
        *v = v.saturating_add(n);
        self.events
            .push((format!("minecraft:{ty}"), key.to_owned(), Some(n)));
    }
    pub(super) fn get(&self, ty: &str, key: &str) -> i32 {
        self.values
            .get(&format!("minecraft:{ty}"))
            .and_then(|m| m.get(key))
            .copied()
            .unwrap_or(0)
    }
    pub(super) fn reset(&mut self, ty: &str, key: &str) {
        if let Some(m) = self.values.get_mut(&format!("minecraft:{ty}")) {
            m.insert(key.to_owned(), 0);
        }
        self.events
            .push((format!("minecraft:{ty}"), key.to_owned(), None));
    }
    pub(super) fn custom(&mut self, key: &str, n: i32) {
        self.add("custom", &format!("minecraft:{key}"), n);
    }
}
impl Game<'_> {
    fn stats_path(&self) -> Option<std::path::PathBuf> {
        let uuid = self.level.player_uuid?;
        let hex: String = uuid.iter().map(|v| format!("{:08x}", *v as u32)).collect();
        let text = format!(
            "{}-{}-{}-{}-{}",
            &hex[0..8],
            &hex[8..12],
            &hex[12..16],
            &hex[16..20],
            &hex[20..32]
        );
        Some(
            self.world_root
                .join("players/stats")
                .join(format!("{text}.json")),
        )
    }
    /// `ServerStatsCounter.parseLocal`.
    pub(super) fn load_stats(&mut self) {
        if self.stats.loaded || self.remote.is_some() {
            return;
        }
        self.stats.loaded = true;
        let Some(path) = self.stats_path() else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return;
        };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else {
            return;
        };
        if let Some(stats) = json.get("stats").and_then(|s| s.as_object()) {
            for (ty, entries) in stats {
                let Some(entries) = entries.as_object() else {
                    continue;
                };
                let map = self.stats.values.entry(ty.clone()).or_default();
                for (key, v) in entries {
                    if let Some(n) = v.as_i64() {
                        map.insert(key.clone(), n as i32);
                    }
                }
            }
        }
    }
    /// `ServerStatsCounter.save`.
    pub(super) fn save_stats(&self) {
        if !self.stats.loaded || self.remote.is_some() {
            return;
        }
        let Some(path) = self.stats_path() else {
            return;
        };
        let stats: serde_json::Map<String, serde_json::Value> = self
            .stats
            .values
            .iter()
            .filter(|(_, m)| !m.is_empty())
            .map(|(ty, m)| {
                let entries: serde_json::Map<String, serde_json::Value> =
                    m.iter().map(|(k, v)| (k.clone(), (*v).into())).collect();
                (ty.clone(), serde_json::Value::Object(entries))
            })
            .collect();
        let json = serde_json::json!({
            "stats": stats,
            "DataVersion": horizon_world::DATA_VERSION,
        });
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&json) {
            let _ = std::fs::write(path, text);
        }
    }
    /// The per-tick counters (`Player.tick`/`ServerPlayer.tick` awardStat).
    pub(super) fn tick_stats(&mut self) {
        self.load_stats();
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let alive = !p.health.dead_or_dying();
        let crouching = p.pose == horizon_client::pose::Pose::Crouching;
        let s = &mut self.stats;
        s.custom("play_time", 1);
        s.custom("total_world_time", 1);
        if alive {
            s.custom("time_since_death", 1);
            if self.sleeping.is_none() {
                s.custom("time_since_rest", 1);
            }
        }
        if crouching {
            s.custom("sneak_time", 1);
        }
        self.tick_movement_stats();
    }
    /// `Player.checkMovementStatistics` / `checkRidingStatistics`.
    fn tick_movement_stats(&mut self) {
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let d = p.position.subtract(self.previous_position);
        let horizontal = ((d.x * d.x + d.z * d.z).sqrt() * 100.0).round() as i32;
        let total = ((d.x * d.x + d.y * d.y + d.z * d.z).sqrt() * 100.0).round() as i32;
        // Teleports are not travel.
        if total <= 0 || total > 1000 {
            return;
        }
        let crouching = p.pose == horizon_client::pose::Pose::Crouching;
        let feet = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        let eye_in_water = self.res.facts.get(self.world.block(
            feet[0],
            (p.position.y + f64::from(p.dimensions.eye_height)).floor() as i32,
            feet[2],
        ));
        let in_water = self
            .res
            .facts
            .get(self.world.block(feet[0], feet[1], feet[2]))
            .fluid
            == horizon_client::blocks::Fluid::Water;
        let on_ground = p.ground.on_ground;
        let (sprinting, flying, gliding, swimming) = (
            p.sprinting,
            p.flying,
            p.fall_flying,
            p.pose == horizon_client::pose::Pose::Swimming,
        );
        let climbing = p.last_climbable.is_some() && d.y > 0.0;
        let riding = self.riding.map(|id| {
            self.mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| m.species())
                .ok_or(())
        });
        let s = &mut self.stats;
        match riding {
            Some(Ok(horizon_client::mobs::Species::Pig)) => s.custom("pig_one_cm", total),
            Some(Ok(horizon_client::mobs::Species::Strider)) => s.custom("strider_one_cm", total),
            Some(Ok(sp)) if horizon_client::mobs::equine::is_equine(sp) => {
                s.custom("horse_one_cm", horizontal)
            }
            Some(_) => {
                if self.boats.iter().any(|b| Some(b.id) == self.riding) {
                    s.custom("boat_one_cm", total)
                } else {
                    s.custom("minecart_one_cm", total)
                }
            }
            None if swimming => s.custom("swim_one_cm", total),
            None if eye_in_water.fluid == horizon_client::blocks::Fluid::Water => {
                s.custom("walk_under_water_one_cm", total)
            }
            None if in_water => s.custom("walk_on_water_one_cm", horizontal),
            None if climbing => s.custom("climb_one_cm", (d.y * 100.0).round() as i32),
            None if on_ground => {
                if sprinting {
                    s.custom("sprint_one_cm", horizontal)
                } else if crouching {
                    s.custom("crouch_one_cm", horizontal)
                } else {
                    s.custom("walk_one_cm", horizontal)
                }
            }
            None if gliding => s.custom("aviate_one_cm", total),
            None if flying => s.custom("fly_one_cm", horizontal),
            None if d.y < 0.0 => s.custom("fall_one_cm", (-d.y * 100.0).round() as i32),
            None => {}
        }
    }
    /// `PhantomSpawner.tick`: tired players under an open night sky draw
    /// phantoms.
    pub(super) fn spawn_phantoms(&mut self) {
        if !self.ui.rules.bool("spawn_phantoms") || self.dim != Dim::Overworld {
            return;
        }
        self.stats.phantom_tick -= 1;
        if self.stats.phantom_tick > 0 {
            return;
        }
        let r = &mut self.world.random;
        self.stats.phantom_tick += (60 + r.next_int_bound(60).unwrap_or(0)) * 20;
        if self.sky_darken() < 5 || self.spectator() {
            return;
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let pos = [
            p.position.x.floor() as i32,
            p.position.y.floor() as i32,
            p.position.z.floor() as i32,
        ];
        if pos[1] < 63 || !self.can_see_sky(pos) {
            return;
        }
        // DifficultyInstance.isHarderThan(random * 3) at normal difficulty.
        let r = &mut self.world.random;
        if r.next_float() * 3.0 >= 2.0 {
            return;
        }
        let rest = self.stats.get("custom", "minecraft:time_since_rest").max(1);
        if r.next_int_bound(rest).unwrap_or(0) < 72000 {
            return;
        }
        let at = [
            pos[0] - 10 + r.next_int_bound(21).unwrap_or(10),
            pos[1] + 20 + r.next_int_bound(15).unwrap_or(0),
            pos[2] - 10 + r.next_int_bound(21).unwrap_or(10),
        ];
        if !self.res.facts.is_air(self.world.block(at[0], at[1], at[2])) {
            return;
        }
        let group = 1 + self.world.random.next_int_bound(3).unwrap_or(0);
        for _ in 0..group {
            let _ = self.spawn_mob(
                "minecraft:phantom",
                [
                    f64::from(at[0]) + 0.5,
                    f64::from(at[1]),
                    f64::from(at[2]) + 0.5,
                ],
                false,
            );
        }
    }
    fn can_see_sky(&self, pos: [i32; 3]) -> bool {
        (pos[1] + 1..pos[1] + 256).all(|y| {
            let f = self.res.facts.get(self.world.block(pos[0], y, pos[2]));
            !f.full_collision && f.fluid == horizon_client::blocks::Fluid::None
        })
    }
}
