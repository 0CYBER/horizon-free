//! Mobs for native server mods (docs/modding/src/how-to/native-mods.md, "Mobs"): spawn one,
//! read the mobs of the level in place, and move, hurt or remove one by its id.
//! They act where the world is simulated, never on a client's copy, and only on
//! the level in place, which each names by its `dimension` (`minecraft:overworld`)
//! as `native_set_block` does: another one is refused. While the game ticks its
//! mobs they are out of the level: a mob a hook spawns then joins it after the
//! tick, and the others read as gone.
use super::{Game, commands::Target};

/// The `EquipmentSlot` names of `Mob::equipment_stacks`, in its order.
const MOB_SLOTS: [&str; 8] = [
    "mainhand", "offhand", "feet", "legs", "chest", "head", "body", "saddle",
];

impl Game<'_> {
    /// `LivingEntity.collectEquipmentChanges` for the mobs that tick: each
    /// slot that holds another stack than the mods last heard is told
    /// (`native_living_equipment_change`). A mob new to the level counts from
    /// empty, as vanilla's `lastEquipmentItems` does. Only while a mod
    /// listens (`native_living_equipment_listened`): a mob's first tick after
    /// that counts from empty too.
    pub(super) fn native_mobs_equipment(&mut self) {
        if self.remote.is_some() || self.options.execution_role.is_client() {
            return;
        }
        // Reading every mob's equipment each tick costs: only when a mod listens.
        let mut listened = false;
        self.native_living_equipment_listened(&mut listened);
        if !listened {
            return;
        }
        let ticking = self.entity_ticking_area();
        let mut changes = Vec::new();
        for m in self.mobs.iter_mut().filter(|m| !m.removed) {
            if !ticking(m.position.x.floor() as i32, m.position.z.floor() as i32) {
                continue;
            }
            let now = m.equipment_stacks();
            let heard = m.extra.equipment_heard.get_or_insert_with(Default::default);
            if **heard == now {
                continue;
            }
            for (slot, name) in MOB_SLOTS.into_iter().enumerate() {
                if now[slot] != heard[slot] {
                    let from = heard[slot].take();
                    changes.push((m.id, m.kind.id.clone(), name, from, now[slot].clone()));
                }
            }
            **heard = now;
        }
        for (id, kind, slot, from, to) in changes {
            self.native_living_equipment_change(id, &kind, slot, from.as_ref(), to.as_ref());
        }
    }
}

impl Game<'_> {
    /// `LivingEntity.collectEquipmentChanges` for the server's players (they are living entities
    /// too): each slot whose stack differs from what the mods last heard is told, the previous
    /// stack with it, so a hotbar selection that moves the main hand counts. A player new to the
    /// level counts from empty, as for mobs, and only while a mod listens.
    pub(super) fn native_players_equipment(&mut self) {
        let mut listened = false;
        self.native_living_equipment_listened(&mut listened);
        if !listened {
            return;
        }
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let mut changes = Vec::new();
        for g in host.hub.guests.iter().filter(|g| host.hub.here(g)) {
            let now: [Option<crate::items::ItemStack>; 6] = [
                g.inventory.get(g.selected).cloned().flatten(),
                g.offhand.clone(),
                g.armor[3].clone(),
                g.armor[2].clone(),
                g.armor[1].clone(),
                g.armor[0].clone(),
            ];
            let Some(play) = host.play.get_mut(&g.conn) else {
                continue;
            };
            let heard = play.equipment_heard.get_or_insert_with(Default::default);
            if **heard == now {
                continue;
            }
            for (slot, name) in PLAYER_SLOTS.into_iter().enumerate() {
                if now[slot] != heard[slot] {
                    let from = heard[slot].take();
                    changes.push((g.entity_id as u32, name, from, now[slot].clone()));
                }
            }
            **heard = now;
        }
        for (id, slot, from, to) in changes {
            self.native_living_equipment_change(
                id,
                "minecraft:player",
                slot,
                from.as_ref(),
                to.as_ref(),
            );
        }
    }
}

/// The `EquipmentSlot` names of a player's equipment, in the order of `native_players_equipment`.
const PLAYER_SLOTS: [&str; 6] = ["mainhand", "offhand", "feet", "legs", "chest", "head"];

/// A mob as a native mod reads it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct NativeMob {
    pub(crate) id: u32,
    /// The level it is in (`minecraft:overworld`).
    pub(crate) dimension: String,
    /// `minecraft:zombie`.
    pub(crate) kind: String,
    pub(crate) pos: [f64; 3],
    /// Degrees; 0 faces south.
    pub(crate) yaw: f32,
    pub(crate) health: f32,
    pub(crate) max_health: f32,
}

/// What a mob starts with (`native_spawn_mob`); the default is a plain mob.
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

/// `/summon`'s world bounds (`Level.isInSpawnableBounds`).
fn spawnable(pos: [f64; 3]) -> bool {
    finite(pos)
        && (-30_000_000.0..30_000_000.0).contains(&pos[0])
        && (-30_000_000.0..30_000_000.0).contains(&pos[2])
        && (-20_000_000.0..20_000_000.0).contains(&pos[1].floor())
}

// Native mods use these; nothing in Horizon does.
#[allow(dead_code, reason = "native mod API (docs/native-mods.md)")]
impl Game<'_> {
    /// Whether this is a level that simulates (not a client's copy) and the one
    /// in place is `dimension`.
    fn native_mob_level(&self, dimension: &str) -> bool {
        self.remote.is_none() && dimension == self.dim.id()
    }
    /// Spawns a mob of `kind` (`minecraft:zombie`) at `pos` in the level in
    /// place, `dimension`, finalized as a natural spawn is, and kept when the
    /// chunk unloads: its id. None for a kind the game doesn't know, another
    /// dimension, a position outside `/summon`'s bounds or in a chunk that isn't
    /// loaded (it would never tick or be saved), or a monster on peaceful (it
    /// would despawn at once).
    pub(super) fn native_spawn_mob(
        &mut self,
        dimension: &str,
        kind: &str,
        pos: [f64; 3],
        options: &NativeSpawn,
    ) -> Option<u32> {
        if !self.native_mob_level(dimension) || !spawnable(pos) || !self.native_chunk_loaded(pos) {
            return None;
        }
        let qualified = if kind.contains(':') {
            kind.to_owned()
        } else {
            ["minecraft:", kind].concat()
        };
        let monster = self
            .res
            .mob_types()
            .get(&qualified)
            .is_some_and(|t| t.category == "monster");
        if monster && self.ui.difficulty == 0 {
            return None;
        }
        let id = self.spawn_mob(
            kind,
            pos,
            true,
            super::creatures::SpawnSite::Finalizing(
                horizon_client::mobs::spawn_rules::Reason::Command,
            ),
        )?;
        // A jockey's other mob comes after it: this one is found by its id.
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id) {
            m.no_ai = options.no_ai;
            m.invisible = options.invisible;
            m.silent = options.silent;
            m.invulnerable_flag = options.invulnerable;
            m.no_gravity = options.no_gravity;
            if let Some(name) = &options.name {
                m.custom_name = Some(name.clone());
                // A plain name replaces any styled one the spawn gave.
                m.custom_name_text = None;
                m.name_visible = true;
            }
            if let Some(health) = options.health.filter(|h| h.is_finite() && *h > 0.0) {
                m.health = health.min(m.attrs.max_health);
            }
        }
        Some(id)
    }
    /// Whether the chunk holding `pos` is loaded in the level in place.
    fn native_chunk_loaded(&self, pos: [f64; 3]) -> bool {
        let (x, z) = (pos[0].floor() as i32, pos[2].floor() as i32);
        self.world.chunks.contains_key(&(x >> 4, z >> 4))
    }
    /// The living mob `id` in the level in place, `dimension`; none on a
    /// client's copy.
    pub(super) fn native_mob(&self, dimension: &str, id: u32) -> Option<NativeMob> {
        if !self.native_mob_level(dimension) {
            return None;
        }
        let mob = self.mobs.iter().find(|m| m.id == id && !m.removed)?;
        mob.alive().then(|| self.native_mob_of(mob))
    }
    /// The living mobs in the level in place, `dimension`; none on a client's
    /// copy.
    pub(super) fn native_mobs(&self, dimension: &str) -> Vec<NativeMob> {
        if !self.native_mob_level(dimension) {
            return Vec::new();
        }
        self.mobs
            .iter()
            .filter(|m| !m.removed && m.alive())
            .map(|m| self.native_mob_of(m))
            .collect()
    }
    /// Kills mob `id` as `/kill` does: it falls and drops its loot. False for
    /// no such living mob or another dimension.
    pub(super) fn kill_mob(&mut self, dimension: &str, id: u32) -> bool {
        if self.native_mob(dimension, id).is_none() {
            return false;
        }
        self.kill_target(Target::Mob(id));
        true
    }
    fn native_mob_of(&self, m: &horizon_client::mobs::Mob) -> NativeMob {
        NativeMob {
            id: m.id,
            dimension: self.dim.id().to_owned(),
            kind: horizon_client::command::args::normalize(&m.kind.id),
            pos: [m.position.x, m.position.y, m.position.z],
            yaw: m.yaw,
            health: m.health,
            max_health: m.attrs.max_health,
        }
    }
}
