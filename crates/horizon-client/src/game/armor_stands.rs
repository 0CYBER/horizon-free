//! Armor stands: `ArmorStandItem.useOn` placement (snapped to 45 degrees),
//! falling, `interactAt` equipment swaps by clicked height, the two-hit break
//! of `hurtServer` (creative breaks at once), and `ArmorStandRenderer` with
//! the default pose, the hit wobble and humanoid armor layers.
use super::{Game, view_vector};
use horizon_client::{
    entities::ArmorStand,
    items::ItemStack,
    mesher::Vertex,
    mobs::{Part, anim::Poses},
};
use horizon_core::{
    geometry::{Aabb, Vec3},
    random::RandomSource,
};
use std::{collections::HashMap, sync::Arc};

/// Armor stand models and the humanoid layers of every equipment asset.
pub(super) struct ArmorStandAssets {
    pub main: Option<Part>,
    pub small: Option<Part>,
    /// (slot model suffix, small) -> model.
    pub layers: HashMap<(&'static str, bool), Part>,
    /// asset id -> layer kind (humanoid / humanoid_leggings) -> (texture, undyed colour).
    pub equipment: HashMap<String, HashMap<String, (String, Option<u32>)>>,
}
impl ArmorStandAssets {
    pub fn load(root: &std::path::Path) -> Self {
        let load = |key: &str| horizon_client::mobs::load_model(root, key);
        let mut layers = HashMap::new();
        for slot in ["helmet", "chestplate", "leggings", "boots"] {
            for (small, model) in [(false, "armor_stand"), (true, "armor_stand_small")] {
                if let Some(part) = load(&format!("minecraft:{model}#{slot}")) {
                    layers.insert((slot, small), part);
                }
            }
        }
        let mut equipment = HashMap::new();
        if let Ok(entries) = std::fs::read_dir(root.join("assets/minecraft/equipment")) {
            for e in entries.flatten() {
                let path = e.path();
                let Some(name) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Ok(v) =
                    std::fs::read(&path).map(|b| serde_json::from_slice::<serde_json::Value>(&b))
                else {
                    continue;
                };
                let Ok(v) = v else { continue };
                let mut kinds = HashMap::new();
                for kind in ["humanoid", "humanoid_leggings"] {
                    if let Some(layer) = v["layers"][kind].get(0) {
                        let tex = layer["texture"]
                            .as_str()
                            .unwrap_or(name)
                            .trim_start_matches("minecraft:");
                        let undyed = layer["dyeable"]["color_when_undyed"]
                            .as_i64()
                            .map(|c| c as u32 & 0xFF_FFFF);
                        kinds.insert(
                            kind.to_owned(),
                            (format!("entity/equipment/{kind}/{tex}"), undyed),
                        );
                    }
                }
                equipment.insert(format!("minecraft:{name}"), kinds);
            }
        }
        Self {
            main: load("minecraft:armor_stand#main"),
            small: load("minecraft:armor_stand_small#main"),
            layers,
            equipment,
        }
    }
}
impl Game<'_> {
    /// `ArmorStandItem.useOn`.
    pub(super) fn place_armor_stand(&mut self, hit: &super::Hit, stack: &ItemStack) -> bool {
        if &*stack.item != "minecraft:armor_stand" || hit.face == [0, -1, 0] {
            return false;
        }
        // BlockPlaceContext: a replaceable clicked block is replaced.
        let clicked = self
            .res
            .facts
            .get(self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]));
        let pos = if clicked.replaceable {
            hit.pos
        } else {
            std::array::from_fn(|i| hit.pos[i] + hit.face[i])
        };
        let foot = Vec3::new(
            f64::from(pos[0]) + 0.5,
            f64::from(pos[1]),
            f64::from(pos[2]) + 0.5,
        );
        let yaw =
            (((self.yaw - 180.0 + 180.0).rem_euclid(360.0) - 180.0 + 22.5) / 45.0).floor() * 45.0;
        let stand = ArmorStand::new(self.next_entity + 1, foot, yaw);
        let b = stand.bounds();
        // Touching the floor is not a collision.
        let (lo, hi) = (b.min_position(), b.max_position());
        let e = 1.0e-7;
        let bounds = Aabb::new(Vec3::new(lo.x + e, lo.y + e, lo.z + e), Vec3::new(hi.x - e, hi.y - e, hi.z - e));
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
        };
        if !scene.unobstructed(bounds).unwrap_or(false)
            || self
                .armor_stands
                .iter()
                .any(|o| overlaps(o.bounds(), bounds))
        {
            return false;
        }
        self.next_entity += 1;
        self.audio.play(
            "minecraft:entity.armor_stand.place",
            Some([foot.x, foot.y, foot.z]),
            0.75,
            0.8,
        );
        self.world.modified.insert((pos[0] >> 4, pos[2] >> 4));
        self.armor_stands.push(stand);
        if !self.creative {
            self.consume_held();
        }
        true
    }
    /// The armor stand under the crosshair, its distance and the hit point.
    fn armor_stand_hit(&self) -> Option<(usize, f64, [f64; 3])> {
        let eye = self.eye_position()?;
        let dir = view_vector(self.yaw, self.pitch);
        let reach: f64 = if self.creative { 5.0 } else { 3.0 };
        let mut best: Option<(usize, f64, [f64; 3])> = None;
        for (i, a) in self.armor_stands.iter().enumerate() {
            if a.marker {
                continue;
            }
            let b = a.bounds();
            let (lo, hi) = (b.min_position(), b.max_position());
            let (lo, hi) = ([lo.x, lo.y, lo.z], [hi.x, hi.y, hi.z]);
            let (mut t0, mut t1) = (0.0f64, reach);
            let mut hit = true;
            for k in 0..3 {
                if dir[k].abs() < 1e-9 {
                    hit &= eye[k] >= lo[k] && eye[k] <= hi[k];
                    continue;
                }
                let (mut ta, mut tb) = ((lo[k] - eye[k]) / dir[k], (hi[k] - eye[k]) / dir[k]);
                if ta > tb {
                    std::mem::swap(&mut ta, &mut tb);
                }
                t0 = t0.max(ta);
                t1 = t1.min(tb);
            }
            if hit && t0 <= t1 && best.is_none_or(|b| t0 < b.1) {
                best = Some((i, t0, std::array::from_fn(|k| eye[k] + dir[k] * t0)));
            }
        }
        best
    }
    fn equipment_slot(&self, stack: &ItemStack) -> usize {
        match self
            .res
            .items
            .get(&stack.item)
            .and_then(|i| i.equip_slot.as_deref())
        {
            Some("feet") => 2,
            Some("legs") => 3,
            Some("chest") => 4,
            Some("head") => 5,
            Some("offhand") => 1,
            _ => 0,
        }
    }
    /// `ArmorStand.interactAt`.
    pub(super) fn interact_armor_stand(&mut self) -> bool {
        let Some((i, d, point)) = self.armor_stand_hit() else {
            return false;
        };
        if !self.before_block(d) {
            return false;
        }
        let held = self.held().cloned();
        if held
            .as_ref()
            .is_some_and(|h| &*h.item == "minecraft:name_tag")
        {
            return false;
        }
        let a = &self.armor_stands[i];
        let slot = match &held {
            Some(h) => {
                let slot = self.equipment_slot(h);
                if slot < 2 && !a.show_arms {
                    return true;
                }
                slot
            }
            None => {
                // getClickedSlot by the height of the click.
                let y = (point[1] - a.position.y) / if a.small { 0.5 } else { 1.0 };
                let has = |s: usize| a.equipment[s].is_some();
                let small = a.small;
                if y >= 0.1 && y < 0.1 + if small { 0.8 } else { 0.45 } && has(2) {
                    2
                } else if y >= 0.9 + if small { 0.3 } else { 0.0 }
                    && y < 0.9 + if small { 1.0 } else { 0.7 }
                    && has(4)
                {
                    4
                } else if y >= 0.4 && y < 0.4 + if small { 1.0 } else { 0.8 } && has(3) {
                    3
                } else if y >= 1.6 && has(5) {
                    5
                } else if !has(0) && has(1) {
                    1
                } else {
                    0
                }
            }
        };
        let current = self.armor_stands[i].equipment[slot].clone();
        if held.is_none() && current.is_none() {
            return false;
        }
        // ArmorStand.swapItem.
        match held {
            Some(h) if h.count > 1 => {
                if current.is_some() {
                    return true;
                }
                self.armor_stands[i].equipment[slot] = Some(ItemStack {
                    count: 1,
                    ..h.clone()
                });
                self.inventory[self.selected] = Some(ItemStack {
                    count: h.count - 1,
                    ..h
                });
            }
            h => {
                self.armor_stands[i].equipment[slot] = h;
                self.inventory[self.selected] = current;
            }
        }
        let p = self.armor_stands[i].position;
        self.world
            .modified
            .insert(((p.x.floor() as i32) >> 4, (p.z.floor() as i32) >> 4));
        self.swing_arm();
        true
    }
    /// `ArmorStand.hurtServer` from a player attack.
    pub(super) fn attack_armor_stand(&mut self) -> bool {
        let Some((i, d, _)) = self.armor_stand_hit() else {
            return false;
        };
        if !self.before_block(d) || self.mob_hit().is_some_and(|m| m.1 < d) {
            return false;
        }
        let time = self.world.game_time;
        let at = self.armor_stands[i].position;
        let at = [at.x, at.y, at.z];
        if self.creative {
            self.armor_stands.remove(i);
            self.audio
                .play("minecraft:entity.armor_stand.break", Some(at), 1.0, 1.0);
            return true;
        }
        if time - self.armor_stands[i].last_hit > 5 {
            // First hit: entity event 32, the wobble and the hit sound.
            self.armor_stands[i].last_hit = time;
            self.audio
                .play("minecraft:entity.armor_stand.hit", Some(at), 0.3, 1.0);
            return true;
        }
        // brokenByPlayer: the stand item, then brokenByAnything: the equipment.
        let a = self.armor_stands.remove(i);
        self.audio
            .play("minecraft:entity.armor_stand.break", Some(at), 1.0, 1.0);
        let center = [at[0], at[1] + if a.small { 0.4875 } else { 0.9875 }, at[2]];
        let stand = ItemStack {
            item: Arc::from("minecraft:armor_stand"),
            count: 1,
            damage: 0,
            extra: None,
        };
        self.spawn_item(stand, center, [0.0; 3], 10);
        for stack in a.equipment.into_iter().flatten() {
            let r = &mut self.world.random;
            let v = [
                f64::from(r.next_float()) * 0.2 - 0.1,
                0.2,
                f64::from(r.next_float()) * 0.2 - 0.1,
            ];
            self.spawn_item(stack, center, v, 10);
        }
        self.world
            .modified
            .insert(((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4));
        true
    }
    pub(super) fn tick_armor_stands(&mut self) {
        let mut scene = horizon_client::collisions::SceneCollisions {
            world: &self.world,
            binding: self.collision,
        };
        for a in &mut self.armor_stands {
            a.tick(&mut scene);
        }
    }
    /// ArmorStandRenderer: the stand with its default pose, then armor layers.
    pub(super) fn armor_stand_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let assets = &self.armor_stand_assets;
        let sprites = &self.res.atlas.sprites;
        let uv_of = |name: &str| sprites.get(name).map(|r| [r.u0, r.v0, r.u1, r.v1]);
        let Some(wood) = uv_of("entity/armorstand/armorstand") else {
            return;
        };
        for a in &self.armor_stands {
            if a.invisible {
                continue;
            }
            let Some(model) = (if a.small { &assets.small } else { &assets.main }) else {
                continue;
            };
            let p = glam::DVec3::new(a.position.x, a.position.y, a.position.z) - camera;
            // setupRotations: 180 - body yaw, plus the wobble for 5 ticks after a hit.
            let wiggle = (self.world.game_time - a.last_hit) as f32 + alpha as f32;
            let wobble = if wiggle < 5.0 {
                (wiggle / 1.5 * std::f32::consts::PI).sin() * 3.0
            } else {
                0.0
            };
            let base = glam::Mat4::from_translation(p.as_vec3())
                * glam::Mat4::from_rotation_y((180.0 - a.yaw + wobble).to_radians())
                * glam::Mat4::from_scale(glam::Vec3::new(-1.0, -1.0, 1.0))
                * glam::Mat4::from_translation(glam::Vec3::new(0.0, -1.501, 0.0));
            let mut poses = Poses::new(model);
            // ArmorStand.DEFAULT_*_POSE and ArmorStandModel part visibility.
            let deg = |v: f32| v.to_radians();
            for (name, rot) in [
                ("left_arm", [-10.0, 0.0, -10.0]),
                ("right_arm", [-15.0, 0.0, 10.0]),
                ("left_leg", [-1.0, 0.0, -1.0]),
                ("right_leg", [1.0, 0.0, 1.0]),
            ] {
                if poses.has(name) {
                    let pose = poses.get(name);
                    pose[3] = deg(rot[0]);
                    pose[4] = deg(rot[1]);
                    pose[5] = deg(rot[2]);
                }
            }
            let mut hide = |name: &str| {
                if poses.has(name) {
                    let pose = poses.get(name);
                    pose[6] = 0.0;
                    pose[7] = 0.0;
                    pose[8] = 0.0;
                }
            };
            if !a.show_arms {
                hide("left_arm");
                hide("right_arm");
            }
            if a.no_base_plate {
                hide("base_plate");
            }
            let light = self.world.light(
                a.position.x.floor() as i32,
                (a.position.y + 0.5).floor() as i32,
                a.position.z.floor() as i32,
            );
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            horizon_client::mobs::render::draw(
                model,
                base,
                &poses,
                wood,
                0xFFFF_FFFF,
                packed,
                0,
                out,
            );
            // HumanoidArmorLayer: head, chest, legs, feet.
            for (slot, layer) in [
                (5usize, "helmet"),
                (4, "chestplate"),
                (3, "leggings"),
                (2, "boots"),
            ] {
                let Some(stack) = &a.equipment[slot] else {
                    continue;
                };
                let Some(asset) = self
                    .res
                    .items
                    .get(&stack.item)
                    .and_then(|i| i.equip_asset.clone())
                else {
                    continue;
                };
                let kind = if layer == "leggings" {
                    "humanoid_leggings"
                } else {
                    "humanoid"
                };
                let Some((texture, undyed)) =
                    assets.equipment.get(&asset).and_then(|k| k.get(kind))
                else {
                    continue;
                };
                let (Some(uv), Some(part)) = (uv_of(texture), assets.layers.get(&(layer, a.small)))
                else {
                    continue;
                };
                let color = undyed.map_or(0xFFFF_FFFF, |c| {
                    0xFF00_0000 | (c & 0xFF) << 16 | (c & 0xFF00) | (c >> 16) & 0xFF
                });
                let layer_poses = Poses::new(part);
                horizon_client::mobs::render::draw(
                    part,
                    base,
                    &layer_poses,
                    uv,
                    color,
                    packed,
                    0,
                    out,
                );
            }
        }
    }
}
fn overlaps(a: Aabb, b: Aabb) -> bool {
    let (a0, a1, b0, b1) = (
        a.min_position(),
        a.max_position(),
        b.min_position(),
        b.max_position(),
    );
    a0.x < b1.x && b0.x < a1.x && a0.y < b1.y && b0.y < a1.y && a0.z < b1.z && b0.z < a1.z
}
