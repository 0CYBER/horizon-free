//! Target dispatch: the level, position and type a command's target names,
//! by family. Moved out of `mod.rs` unchanged (arismoko/horizon#2791's discipline).
use super::*;

impl Game<'_> {
    /// The level `t` is in (`Entity.level`): a server's players have their
    /// own, which need not be the one in place; everything else is in the
    /// level in place.
    pub(in crate::game) fn target_level(&self, t: Target) -> Dim {
        let conn = match t {
            Target::Player => self.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => None,
        };
        conn.and_then(|c| self.command_guest(c))
            .map_or(self.dim, |g| Dim::from_id(&g.dimension))
    }
    pub(in crate::game) fn target_position(&self, t: Target) -> Option<[f64; 3]> {
        match t {
            Target::Player => self
                .player
                .as_ref()
                .map(|p| [p.position.x, p.position.y, p.position.z]),
            Target::Guest(conn) => self.command_guest(conn).map(|g| g.position),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map(|m| [m.position.x, m.position.y, m.position.z]),
            Target::Item(id) => self
                .dropped
                .iter()
                .find(|i| i.id == id)
                .map(|i| [i.position.x, i.position.y, i.position.z]),
            Target::Orb(id) => self
                .orbs
                .iter()
                .find(|o| o.id == id)
                .map(|o| [o.position.x, o.position.y, o.position.z]),
            Target::Projectile(id) => self
                .creatures
                .projectiles
                .iter()
                .find(|a| a.id == id)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
            Target::Stand(id) => self
                .armor_stands
                .iter()
                .find(|a| a.id == id)
                .map(|a| [a.position.x, a.position.y, a.position.z]),
            Target::Frame(id) => self.item_frames.iter().find(|f| f.id == id).map(|f| {
                let (lo, hi) = f.bounds();
                [
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ]
            }),
            Target::Painting(id) => self.paintings.iter().find(|p| p.id == id).map(|p| {
                let (lo, hi) = p.bounds();
                [
                    (lo[0] + hi[0]) / 2.0,
                    (lo[1] + hi[1]) / 2.0,
                    (lo[2] + hi[2]) / 2.0,
                ]
            }),
            Target::Boat(id) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map(|b| [b.position.x, b.position.y, b.position.z]),
            Target::Cart(id) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::Other(id) => self
                .others
                .iter()
                .find(|o| o.id == id)
                .map(|o| [o.position.x, o.position.y, o.position.z]),
            Target::Crystal(id) => self
                .crystals
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::Dragon(id) => self
                .dragon
                .as_ref()
                .filter(|d| d.id == id)
                .map(|d| [d.position.x, d.position.y, d.position.z]),
            Target::Cushion(id) => self
                .cushions
                .iter()
                .find(|c| c.id == id)
                .map(|c| [c.position.x, c.position.y, c.position.z]),
            Target::DragonPart(id) => self.resident_dragon_part(id).map(|(d, i)| {
                let p = d.parts[i].0;
                [p.x, p.y, p.z]
            }),
            Target::Tnt(id) => self
                .tnts
                .iter()
                .find(|t| t.id == id)
                .map(|t| [t.position.x, t.position.y, t.position.z]),
            Target::Falling(id) => self
                .falling
                .iter()
                .find(|f| f.id == id)
                .map(|f| [f.position.x, f.position.y, f.position.z]),
            Target::Rocket(id) => self
                .rockets
                .iter()
                .find(|r| r.id == id)
                .map(|r| [r.position.x, r.position.y, r.position.z]),
            Target::Cloud(id) => self.breath_clouds.iter().find(|c| c.id == id).map(|c| c.at),
            Target::Hook(id) => self.fishing_hook_by_id(id).map(|(h, _)| {
                let p = h.position;
                [p.x, p.y, p.z]
            }),
            Target::Trident(id) => self
                .tridents
                .iter()
                .find(|t| t.id == id)
                .map(|t| [t.position.x, t.position.y, t.position.z]),
            Target::Fangs(id) => self.fangs.list.iter().find(|f| f.id == id).map(|f| f.pos),
            Target::Spawner(id) => self
                .ominous_spawners
                .iter()
                .find(|s| s.id == id)
                .map(|s| s.position),
            Target::Knot(id) => self.natural_knot(id).map(|fence| {
                let p = horizon_client::mobs::leash::knot_state(fence).position;
                [p.x, p.y, p.z]
            }),
            // Stage 3's insertion joins live rows; a selector cannot find one yet.
            Target::General(id) => self
                .resident_entities
                .general
                .rows
                .iter()
                .find(|row| row.id == id)
                .map(|row| [row.position.x, row.position.y, row.position.z]),
        }
    }
    pub(in crate::game) fn target_type(&self, t: Target) -> String {
        match t {
            Target::Player | Target::Guest(_) => "minecraft:player".into(),
            Target::Mob(id) => self
                .mobs
                .iter()
                .find(|m| m.id == id)
                .map_or_else(String::new, |m| {
                    horizon_client::command::args::normalize(&m.kind.id)
                }),
            Target::Item(_) => "minecraft:item".into(),
            Target::Orb(_) => "minecraft:experience_orb".into(),
            Target::Projectile(id) => self
                .creatures
                .projectiles
                .iter()
                .find(|p| p.id == id)
                .map_or_else(String::new, |p| p.entity_type().into()),
            Target::Stand(_) => "minecraft:armor_stand".into(),
            Target::General(_) => String::new(),
            Target::Frame(id) => {
                if self.item_frames.iter().any(|f| f.id == id && f.glow) {
                    "minecraft:glow_item_frame".into()
                } else {
                    "minecraft:item_frame".into()
                }
            }
            Target::Painting(_) => "minecraft:painting".into(),
            Target::Boat(id) => self
                .boats
                .iter()
                .find(|b| b.id == id)
                .map_or_else(|| "minecraft:oak_boat".into(), |b| b.entity_type()),
            Target::Cart(id) => self
                .carts
                .iter()
                .find(|c| c.id == id)
                .map_or_else(|| "minecraft:minecart".into(), |c| c.kind.id().into()),
            Target::Other(id) => self
                .others
                .iter()
                .find(|o| o.id == id)
                .map_or_else(String::new, |o| o.kind.clone()),
            Target::Crystal(_) => "minecraft:end_crystal".into(),
            // `EnderDragonPart` takes its parent's type.
            Target::Dragon(_) | Target::DragonPart(_) => "minecraft:ender_dragon".into(),
            Target::Tnt(_) => "minecraft:tnt".into(),
            Target::Cushion(_) => "minecraft:cushion".into(),
            Target::Falling(_) => "minecraft:falling_block".into(),
            Target::Rocket(_) => "minecraft:firework_rocket".into(),
            Target::Trident(_) => "minecraft:trident".into(),
            Target::Fangs(_) => "minecraft:evoker_fangs".into(),
            Target::Spawner(_) => "minecraft:ominous_item_spawner".into(),
            Target::Cloud(_) => "minecraft:area_effect_cloud".into(),
            Target::Hook(_) => "minecraft:fishing_bobber".into(),
            Target::Knot(_) => "minecraft:leash_knot".into(),
        }
    }
}

/// The network id of a target that is no player: its entity id.
pub(super) fn non_player_entity_id(t: Target) -> Option<i32> {
    let (Target::Mob(id)
    | Target::Item(id)
    | Target::Orb(id)
    | Target::Projectile(id)
    | Target::Stand(id)
    | Target::Frame(id)
    | Target::Painting(id)
    | Target::Boat(id)
    | Target::Cart(id)
    | Target::Other(id)
    | Target::Fangs(id)
    | Target::Crystal(id)
    | Target::Dragon(id)
    | Target::DragonPart(id)
    | Target::Tnt(id)
    | Target::Cushion(id)
    | Target::Falling(id)
    | Target::Rocket(id)
    | Target::Trident(id)
    | Target::Spawner(id)
    | Target::Cloud(id)
    | Target::Hook(id)
    | Target::General(id)
    | Target::Knot(id)) = t
    else {
        return None;
    };
    i32::try_from(id).ok()
}
