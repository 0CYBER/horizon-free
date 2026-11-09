use super::{Game, Target};

impl Game<'_> {
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
        }
    }
}
