//! Mobs for native server mods (docs/native-mods.md, "Mobs", API 23): spawn one,
//! read the mobs of the level in place, and move, hurt or remove one by its id.
//! They act where the world is simulated, never on a client's copy, and only on
//! the level in place. While the game ticks its mobs they are out of the level:
//! a mob a hook spawns then joins it after the tick, and the others read as gone.
use super::{Game, commands::Target};
use horizon_core::geometry::Vec3;

/// A mob as a native mod reads it.
#[allow(dead_code)]
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeMob {
    pub(crate) id: u32,
    /// `minecraft:zombie`.
    pub(crate) kind: String,
    pub(crate) pos: [f64; 3],
    /// Degrees; 0 faces south.
    pub(crate) yaw: f32,
    pub(crate) health: f32,
    pub(crate) max_health: f32,
}

/// What a mob starts with (`native_spawn_mob`); the default is a plain mob.
#[allow(dead_code)]
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct NativeSpawn {
    /// A name over it, shown to everyone.
    pub(crate) name: Option<String>,
    /// Cut to its maximum.
    pub(crate) health: Option<f32>,
    pub(crate) no_ai: bool,
    pub(crate) invisible: bool,
    pub(crate) silent: bool,
    pub(crate) invulnerable: bool,
    pub(crate) no_gravity: bool,
}

fn finite(v: [f64; 3]) -> bool {
    v.into_iter().all(f64::is_finite)
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// Spawns a mob of `kind` (`minecraft:zombie`) at `pos`, finalized as a
    /// natural spawn is, and kept when the chunk unloads: its id, or none for a
    /// kind the game doesn't know or a position that isn't finite. API 23.
    pub(super) fn native_spawn_mob(
        &mut self,
        kind: &str,
        pos: [f64; 3],
        options: &NativeSpawn,
    ) -> Option<u32> {
        if self.remote.is_some() || !finite(pos) {
            return None;
        }
        let id = self.spawn_mob(kind, pos, true)?;
        // A jockey's other mob comes after it: this one is found by its id.
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.no_ai = options.no_ai;
            m.invisible = options.invisible;
            m.silent = options.silent;
            m.invulnerable_flag = options.invulnerable;
            m.no_gravity = options.no_gravity;
            if let Some(name) = &options.name {
                m.custom_name = Some(name.clone());
                m.name_visible = true;
            }
            if let Some(health) = options.health.filter(|h| h.is_finite() && *h > 0.0) {
                m.health = health.min(m.attrs.max_health);
            }
        }
        Some(id)
    }
    /// The living mob `id` in the level in place. API 23.
    pub(super) fn native_mob(&self, id: u32) -> Option<NativeMob> {
        let mob = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        mob.alive().then(|| native_mob_of(mob))
    }
    /// The living mobs in the level in place. API 23.
    pub(super) fn native_mobs(&self) -> Vec<NativeMob> {
        self.mobs
            .iter()
            .filter(|m| !m.removed && m.alive())
            .map(native_mob_of)
            .collect()
    }
    /// Moves mob `id` to `pos` at once, as `/tp` does, and resets its fall:
    /// false for no such mob or a position that isn't finite. API 23.
    pub(super) fn native_mob_teleport(&mut self, id: u32, pos: [f64; 3]) -> bool {
        if self.remote.is_some() || !finite(pos) {
            return false;
        }
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        let to = Vec3::new(pos[0], pos[1], pos[2]);
        m.position = to;
        m.previous = to;
        m.fall_distance = 0.0;
        m.needs_sync = true;
        true
    }
    /// Adds `velocity` to mob `id`'s: false for no such mob or a velocity that
    /// isn't finite. API 23.
    pub(super) fn native_mob_push(&mut self, id: u32, velocity: [f64; 3]) -> bool {
        if self.remote.is_some() || !finite(velocity) {
            return false;
        }
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        m.velocity = Vec3::new(
            m.velocity.x + velocity[0],
            m.velocity.y + velocity[1],
            m.velocity.z + velocity[2],
        );
        m.needs_sync = true;
        true
    }
    /// Sets the living mob `id`'s health, cut to its maximum: false for no such
    /// mob or a health that isn't above 0 (`native_kill_mob` kills). API 23.
    pub(super) fn native_mob_set_health(&mut self, id: u32, health: f32) -> bool {
        if self.remote.is_some() || !health.is_finite() || health <= 0.0 {
            return false;
        }
        let Some(m) = self
            .mobs
            .iter_mut()
            .find(|m| m.id == id && !m.removed && m.alive())
        else {
            return false;
        };
        m.health = health.min(m.attrs.max_health);
        m.needs_sync = true;
        true
    }
    /// Kills mob `id` as `/kill` does: it falls and drops its loot. False for
    /// no such living mob. API 23.
    pub(super) fn native_kill_mob(&mut self, id: u32) -> bool {
        if self.remote.is_some() || self.native_mob(id).is_none() {
            return false;
        }
        self.kill_target(Target::Mob(id));
        true
    }
    /// Takes mob `id` out of the world with no death and no loot: false for no
    /// such mob. API 23.
    pub(super) fn native_remove_mob(&mut self, id: u32) -> bool {
        if self.remote.is_some() {
            return false;
        }
        let Some(m) = self.mobs.iter_mut().find(|m| m.id == id && !m.removed) else {
            return false;
        };
        m.removed = true;
        true
    }
}

fn native_mob_of(m: &horizon_client::mobs::Mob) -> NativeMob {
    NativeMob {
        id: m.id,
        kind: ["minecraft:", m.kind.id.trim_start_matches("minecraft:")].concat(),
        pos: [m.position.x, m.position.y, m.position.z],
        yaw: m.yaw,
        health: m.health,
        max_health: m.attrs.max_health,
    }
}
