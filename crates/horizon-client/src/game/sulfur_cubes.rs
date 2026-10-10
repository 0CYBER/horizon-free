//! Sulfur cubes on the game side (`SulfurCube.mobInteract`, `pickUpItem`,
//! `remove`): feeding babies, swallowing a held block, shearing it back
//! out, lighting an explosive one, swallowing blocks from the ground and
//! splitting into two babies.
use super::Game;
use horizon_client::{
    items::ItemStack,
    mobs::{Mob, Species, sulfur_cube},
};
use horizon_core::random::RandomSource;

impl Game<'_> {
    fn sulfur_sound(&mut self, index: usize, sound: &str) {
        let at = self.mobs[index].center();
        self.audio
            .play(&format!("minecraft:{sound}"), Some(at), 1.0, 1.0);
    }
    /// Spit the swallowed block out on top (`EntityAttachment.PASSENGER`).
    fn eject_sulfur_block(&mut self, index: usize) {
        let m = &mut self.mobs[index];
        let Some(item) = m.extra.head.take() else {
            return;
        };
        let at = [
            m.position.x,
            m.position.y + f64::from(m.height()),
            m.position.z,
        ];
        self.spawn_item(ItemStack::new(&item, 1), at, [0.0, 0.2, 0.0], 10);
    }
    /// `SulfurCube.mobInteract`. Returns whether the click was used.
    pub(super) fn interact_sulfur_cube(&mut self, index: usize, item: &str) -> bool {
        let m = &self.mobs[index];
        if m.species() != Species::SulfurCube || !m.alive() {
            return false;
        }
        if m.is_baby() {
            // Slime balls grow a baby up by a tenth.
            if !m.kind.food.iter().any(|f| f == item) || m.extra.age_locked {
                return false;
            }
            let m = &mut self.mobs[index];
            let secs = (-m.extra.age / 20) as f32 * 0.1;
            m.extra.age = (m.extra.age + (secs as i32) * 20).min(0);
            self.consume_held();
            self.sulfur_sound(index, "entity.small_sulfur_cube.eat");
            return true;
        }
        if sulfur_cube::primed(m) {
            return false;
        }
        let explosive = sulfur_cube::archetype(m).is_some_and(|a| a.explosion.is_some());
        if explosive && matches!(item, "minecraft:flint_and_steel" | "minecraft:fire_charge") {
            if !self.ui.rules.bool("tnt_explodes") {
                return false;
            }
            sulfur_cube::prime(&mut self.mobs[index], false);
            if item == "minecraft:flint_and_steel" {
                self.damage_held_tool();
            } else {
                self.consume_held();
            }
            return true;
        }
        if item == "minecraft:shears" && sulfur_cube::has_body(m) {
            // shear: out comes the block, and nothing is swallowed for 5 s.
            self.eject_sulfur_block(index);
            self.mobs[index].extra.ints[3] = sulfur_cube::PICKUP_TIMER;
            self.sulfur_sound(index, "entity.sulfur_cube.eject");
            self.damage_held_tool();
            return true;
        }
        if sulfur_cube::swallowable(m, item) {
            // equipItem: not the same block again; the old one comes out.
            if m.extra.head.as_deref() == Some(item) {
                return false;
            }
            self.eject_sulfur_block(index);
            self.mobs[index].extra.head = Some(item.into());
            self.consume_held();
            self.sulfur_sound(index, "entity.sulfur_cube.absorb");
            return true;
        }
        false
    }
    /// `SulfurCubeSearchForItemsGoal` and `pickUpItem`: an adult with nothing
    /// inside hops to a block it can swallow within 8 and takes it in.
    pub(super) fn sulfur_cube_pickups(&mut self) {
        let griefing = self.ui.rules.bool("mob_griefing");
        for i in 0..self.mobs.len() {
            let m = &self.mobs[i];
            if m.species() != Species::SulfurCube {
                continue;
            }
            if !m.alive() || m.is_baby() || sulfur_cube::has_body(m) || m.extra.ints[3] > 0 {
                self.mobs[i].extra.point = None;
                continue;
            }
            let reach = m
                .bounds()
                .inflate(horizon_core::geometry::Vec3::new(8.0, 8.0, 8.0));
            let pos = m.position;
            let best = self
                .dropped
                .iter()
                .enumerate()
                .filter(|(_, e)| {
                    e.pickup_delay == 0
                        && reach.contains(e.position)
                        && sulfur_cube::swallowable(m, &e.stack.item)
                })
                .map(|(j, e)| (j, e.position.distance_squared(pos)))
                .min_by(|a, b| a.1.total_cmp(&b.1));
            let Some((j, _)) = best else {
                self.mobs[i].extra.point = None;
                continue;
            };
            // Mob.aiStep: items within the box grown by (1, 0, 1).
            let touch = m
                .bounds()
                .inflate(horizon_core::geometry::Vec3::new(1.0, 0.0, 1.0));
            let e = &self.dropped[j];
            let close = touch.contains(e.position);
            self.mobs[i].extra.point = Some([e.position.x, e.position.y, e.position.z]);
            if !griefing || !close {
                continue;
            }
            let item = e.stack.item.clone();
            let e = &mut self.dropped[j];
            e.stack.count -= 1;
            if e.stack.count == 0 {
                self.dropped.remove(j);
            }
            let m = &mut self.mobs[i];
            m.extra.head = Some(item);
            m.extra.point = None;
            self.sulfur_sound(i, "entity.sulfur_cube.absorb");
        }
    }
    /// `AbstractCubeMob.remove`: a dead adult splits into two babies (none
    /// when it was primed).
    pub(super) fn split_sulfur_cube(&mut self, m: &Mob) {
        let size = m.extra.size;
        if size <= 1 {
            return;
        }
        let offset = f64::from(m.width()) / 2.0;
        for i in 0..sulfur_cube::split_count(m) {
            let xd = (f64::from(i % 2) - 0.5) * offset;
            let zd = (f64::from(i / 2) - 0.5) * offset;
            let at = [m.position.x + xd, m.position.y + 0.5, m.position.z + zd];
            let yaw = self.world.random.next_float() * 360.0;
            if let Some(id) = self.spawn_mob(&m.kind.id, at, m.persistent)
                && let Some(c) = self.mobs.iter_mut().find(|c| c.id == id)
            {
                c.extra.age = -24000;
                sulfur_cube::set_size(c, size / 2, true);
                c.yaw = yaw;
                c.body_yaw = yaw;
                c.head_yaw = yaw;
            }
        }
    }
}
